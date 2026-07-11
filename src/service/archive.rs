use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Cursor, Read, Seek},
    path::{Component, Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct ArchiveLimits {
    pub compressed_bytes: u64,
    pub entries: usize,
    pub expanded_bytes: u64,
    pub file_bytes: u64,
    pub path_bytes: usize,
    pub path_components: usize,
    pub expansion_ratio: u64,
}
impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            compressed_bytes: 50 * 1024 * 1024,
            entries: 25_000,
            expanded_bytes: 250 * 1024 * 1024,
            file_bytes: 5 * 1024 * 1024,
            path_bytes: 512,
            path_components: 32,
            expansion_ratio: 100,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ArchiveError {
    #[error("archive I/O failed")]
    Io,
    #[error("archive is malformed")]
    Malformed,
    #[error("archive is encrypted")]
    Encrypted,
    #[error("archive exceeds byte limits")]
    TooLarge,
    #[error("archive has too many entries")]
    TooManyEntries,
    #[error("archive contains an unsafe path")]
    UnsafePath,
    #[error("archive contains a duplicate path")]
    DuplicatePath,
    #[error("archive has multiple roots")]
    MultipleRoots,
    #[error("archive contains an unsupported entry")]
    UnsupportedEntry,
}
impl From<io::Error> for ArchiveError {
    fn from(_: io::Error) -> Self {
        Self::Io
    }
}

pub fn extract_zip(
    path: &Path,
    destination: &Path,
    limits: ArchiveLimits,
) -> Result<PathBuf, ArchiveError> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > limits.compressed_bytes {
        return Err(ArchiveError::TooLarge);
    }
    extract_zip_reader(File::open(path)?, destination, limits, metadata.len())
}
pub fn extract_zip_reader<R: Read + Seek>(
    mut reader: R,
    destination: &Path,
    limits: ArchiveLimits,
    compressed: u64,
) -> Result<PathBuf, ArchiveError> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limits.compressed_bytes || compressed > limits.compressed_bytes {
        return Err(ArchiveError::TooLarge);
    }
    reject_raw_duplicates(&bytes, limits.entries)?;
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| ArchiveError::Malformed)?;
    if zip.len() > limits.entries {
        return Err(ArchiveError::TooManyEntries);
    }
    fs::create_dir_all(destination)?;
    let mut seen = HashSet::new();
    let mut root: Option<String> = None;
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|_| ArchiveError::Malformed)?;
        if entry.encrypted() {
            return Err(ArchiveError::Encrypted);
        }
        let name = entry.name();
        if name.as_bytes().contains(&0) || name.contains('\\') || name.len() > limits.path_bytes {
            return Err(ArchiveError::UnsafePath);
        }
        let path = Path::new(name);
        if path.is_absolute() {
            return Err(ArchiveError::UnsafePath);
        }
        let components: Vec<_> = path.components().collect();
        if components.is_empty()
            || components.len() > limits.path_components
            || components
                .iter()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(ArchiveError::UnsafePath);
        }
        let first = components[0]
            .as_os_str()
            .to_str()
            .ok_or(ArchiveError::UnsafePath)?
            .to_owned();
        match &root {
            Some(r) if r != &first => return Err(ArchiveError::MultipleRoots),
            None => root = Some(first),
            _ => {}
        }
        let normalized = components
            .iter()
            .map(|c| c.as_os_str())
            .collect::<PathBuf>();
        if !seen.insert(normalized.clone()) {
            return Err(ArchiveError::DuplicatePath);
        }
        let mode = entry
            .unix_mode()
            .unwrap_or(if entry.is_dir() { 0o040755 } else { 0o100644 });
        let kind = mode & 0o170000;
        if kind != 0 && kind != 0o040000 && kind != 0o100000 {
            return Err(ArchiveError::UnsupportedEntry);
        }
        if !entry.is_dir() {
            if entry.size() > limits.file_bytes {
                return Err(ArchiveError::TooLarge);
            }
            total = total
                .checked_add(entry.size())
                .ok_or(ArchiveError::TooLarge)?;
            if total > limits.expanded_bytes
                || (compressed > 0 && total > compressed.saturating_mul(limits.expansion_ratio))
            {
                return Err(ArchiveError::TooLarge);
            }
        }
        let out = destination.join(&normalized);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&out)
            .map_err(|_| ArchiveError::DuplicatePath)?;
        let copied = io::copy(&mut entry.by_ref().take(limits.file_bytes + 1), &mut file)?;
        if copied > limits.file_bytes {
            return Err(ArchiveError::TooLarge);
        }
    }
    let root = root.ok_or(ArchiveError::Malformed)?;
    Ok(destination.join(root))
}

fn reject_raw_duplicates(bytes: &[u8], entry_limit: usize) -> Result<(), ArchiveError> {
    let start = bytes.len().saturating_sub(65_557);
    let eocd = (start..bytes.len().saturating_sub(3))
        .rev()
        .find(|&i| bytes.get(i..i + 4) == Some(b"PK\x05\x06"))
        .ok_or(ArchiveError::Malformed)?;
    if eocd + 22 > bytes.len() {
        return Err(ArchiveError::Malformed);
    }
    let u16at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    let u32at = |i: usize| {
        u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize
    };
    let count = u16at(eocd + 10);
    if count == 0xffff {
        return Err(ArchiveError::Malformed);
    }
    if count > entry_limit {
        return Err(ArchiveError::TooManyEntries);
    }
    let mut p = u32at(eocd + 16);
    let mut names = HashSet::new();
    for _ in 0..count {
        if p + 46 > bytes.len() || bytes.get(p..p + 4) != Some(b"PK\x01\x02") {
            return Err(ArchiveError::Malformed);
        }
        let name_len = u16at(p + 28);
        let extra = u16at(p + 30);
        let comment = u16at(p + 32);
        let mode = u32at(p + 38) >> 16;
        let kind = mode & 0o170000;
        let permissions = mode & 0o777;
        if kind != 0 && kind != 0o040000 && kind != 0o100000 {
            return Err(ArchiveError::UnsupportedEntry);
        }
        if kind == 0o100000 && !matches!(permissions, 0 | 0o644 | 0o755) {
            return Err(ArchiveError::UnsupportedEntry);
        }
        let end = p
            .checked_add(46 + name_len + extra + comment)
            .ok_or(ArchiveError::Malformed)?;
        if end > bytes.len() {
            return Err(ArchiveError::Malformed);
        }
        if !names.insert(bytes[p + 46..p + 46 + name_len].to_vec()) {
            return Err(ArchiveError::DuplicatePath);
        }
        p = end
    }
    Ok(())
}
