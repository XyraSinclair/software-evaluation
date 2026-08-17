//! Shared deterministic source discovery and tree-sitter parsing.
//!
//! `SourceCorpusSession` materializes one immutable source snapshot for a
//! bounded analysis scope. Existing analyzers can keep calling
//! [`load_source_tree`] and [`parse_source`]; while a matching session is live,
//! those calls receive cheap `Arc`/tree clones from the corpus rather than
//! walking, reading, and parsing the same files independently.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, RwLock, Weak};

use ignore::WalkBuilder;
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tree_sitter::{Parser, Tree};

pub const SOURCE_CORPUS_SCHEMA_VERSION: &str = "seval.source-corpus.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceLanguage {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
}

impl SourceLanguage {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::TypeScript | Self::Tsx => "typescript",
            Self::Go => "go",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SourceFile {
    pub absolute_path: PathBuf,
    pub path: String,
    pub language: SourceLanguage,
    /// Shared immutable bytes. Cloning a source tree does not copy file bodies.
    pub bytes: Arc<[u8]>,
}

#[derive(Debug, Clone)]
pub struct SourceTree {
    pub root: String,
    pub files: Vec<SourceFile>,
    pub enumerated: usize,
    pub skipped: usize,
}

#[derive(Debug)]
pub struct ParsedSource<'a> {
    pub file: &'a SourceFile,
    pub tree: Tree,
    pub has_syntax_errors: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceFileReceipt {
    pub path: String,
    pub language: SourceLanguage,
    pub bytes: u64,
    pub sha256: String,
    pub syntax_errors: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceCorpusReceipt {
    pub schema_version: String,
    /// Reported input root; excluded from the content manifest digest.
    pub root: String,
    /// SHA-256 over the ordered path/language/length/content-digest manifest.
    pub manifest_sha256: String,
    pub enumerated_files: usize,
    pub supported_files: usize,
    pub skipped_files: usize,
    pub total_bytes: u64,
    pub syntax_error_files: usize,
    pub filesystem_walks: usize,
    pub file_reads: usize,
    pub parses: usize,
    pub files: Vec<SourceFileReceipt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SourceCorpusCacheStats {
    pub source_tree_hits: usize,
    pub parse_tree_hits: usize,
}

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("input does not exist: {0}")]
    Missing(PathBuf),
    #[error("input is a symbolic link and is not followed: {0}")]
    Symlink(PathBuf),
    #[error("cannot inspect {path}: {source}")]
    Inspect {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot traverse {path}: {message}")]
    Traverse { path: PathBuf, message: String },
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("path is not valid UTF-8: {0:?}")]
    NonUtf8(PathBuf),
    #[error("cannot configure {language} parser: {message}")]
    ParserConfiguration {
        language: &'static str,
        message: String,
    },
    #[error("tree-sitter returned no tree for {0}")]
    Parse(PathBuf),
}

#[derive(Debug)]
struct CachedParse {
    bytes: Arc<[u8]>,
    tree: Tree,
    has_syntax_errors: bool,
}

#[derive(Debug)]
struct SourceCorpus {
    tree: SourceTree,
    parsed: BTreeMap<PathBuf, CachedParse>,
    receipt: SourceCorpusReceipt,
    source_tree_hits: AtomicUsize,
    parse_tree_hits: AtomicUsize,
}

/// A scoped immutable source corpus shared by analyzers running in any thread.
///
/// Dropping the session removes it from the process-local lookup registry. A
/// previous nested session for the same input is restored rather than lost.
#[derive(Debug)]
pub struct SourceCorpusSession {
    key: PathBuf,
    corpus: Arc<SourceCorpus>,
    previous: Option<Weak<SourceCorpus>>,
}

impl SourceCorpusSession {
    pub fn activate(input: &Path) -> Result<Self, SourceError> {
        let key = input_key(input);
        let tree = load_source_tree_uncached(input)?;
        let mut parsed = BTreeMap::new();
        let mut file_receipts = Vec::with_capacity(tree.files.len());
        let mut total_bytes = 0u64;
        let mut syntax_error_files = 0usize;
        let mut manifest = Sha256::new();
        manifest.update(SOURCE_CORPUS_SCHEMA_VERSION.as_bytes());
        manifest.update([0]);
        manifest.update((tree.enumerated as u64).to_be_bytes());
        manifest.update((tree.skipped as u64).to_be_bytes());

        for file in &tree.files {
            let observation = parse_source_uncached(file)?;
            let digest = Sha256::digest(file.bytes.as_ref());
            let bytes = file.bytes.len() as u64;
            total_bytes = total_bytes.saturating_add(bytes);
            syntax_error_files += usize::from(observation.has_syntax_errors);

            update_manifest_field(&mut manifest, file.path.as_bytes());
            update_manifest_field(&mut manifest, file.language.name().as_bytes());
            manifest.update(bytes.to_be_bytes());
            manifest.update(digest);
            manifest.update([u8::from(observation.has_syntax_errors)]);

            file_receipts.push(SourceFileReceipt {
                path: file.path.clone(),
                language: file.language,
                bytes,
                sha256: hex_digest(&digest),
                syntax_errors: observation.has_syntax_errors,
            });
            parsed.insert(
                file.absolute_path.clone(),
                CachedParse {
                    bytes: Arc::clone(&file.bytes),
                    tree: observation.tree,
                    has_syntax_errors: observation.has_syntax_errors,
                },
            );
        }

        let receipt = SourceCorpusReceipt {
            schema_version: SOURCE_CORPUS_SCHEMA_VERSION.to_owned(),
            root: tree.root.clone(),
            manifest_sha256: hex_digest(&manifest.finalize()),
            enumerated_files: tree.enumerated,
            supported_files: tree.files.len(),
            skipped_files: tree.skipped,
            total_bytes,
            syntax_error_files,
            filesystem_walks: 1,
            file_reads: tree.files.len(),
            parses: tree.files.len(),
            files: file_receipts,
        };
        let corpus = Arc::new(SourceCorpus {
            tree,
            parsed,
            receipt,
            source_tree_hits: AtomicUsize::new(0),
            parse_tree_hits: AtomicUsize::new(0),
        });
        let previous = registry_write().insert(key.clone(), Arc::downgrade(&corpus));
        Ok(Self {
            key,
            corpus,
            previous,
        })
    }

    #[must_use]
    pub fn receipt(&self) -> &SourceCorpusReceipt {
        &self.corpus.receipt
    }

    #[must_use]
    pub fn cache_stats(&self) -> SourceCorpusCacheStats {
        SourceCorpusCacheStats {
            source_tree_hits: self.corpus.source_tree_hits.load(Ordering::Relaxed),
            parse_tree_hits: self.corpus.parse_tree_hits.load(Ordering::Relaxed),
        }
    }
}

impl Drop for SourceCorpusSession {
    fn drop(&mut self) {
        let current = Arc::downgrade(&self.corpus);
        let mut registry = registry_write();
        if registry
            .get(&self.key)
            .is_some_and(|registered| Weak::ptr_eq(registered, &current))
        {
            if self.previous.as_ref().and_then(Weak::upgrade).is_some() {
                registry.insert(
                    self.key.clone(),
                    self.previous.take().expect("previous session was present"),
                );
            } else {
                registry.remove(&self.key);
            }
        }
    }
}

pub fn load_source_tree(input: &Path) -> Result<SourceTree, SourceError> {
    if let Some(corpus) = active_corpus(input) {
        corpus.source_tree_hits.fetch_add(1, Ordering::Relaxed);
        return Ok(corpus.tree.clone());
    }
    load_source_tree_uncached(input)
}

fn load_source_tree_uncached(input: &Path) -> Result<SourceTree, SourceError> {
    let metadata = fs::symlink_metadata(input).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            SourceError::Missing(input.to_owned())
        } else {
            SourceError::Inspect {
                path: input.to_owned(),
                source,
            }
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(SourceError::Symlink(input.to_owned()));
    }

    let (root, candidates) = if metadata.is_file() {
        let parent = input.parent().unwrap_or_else(|| Path::new("."));
        (parent.to_path_buf(), vec![input.to_path_buf()])
    } else if metadata.is_dir() {
        let mut paths = Vec::new();
        let walker = WalkBuilder::new(input)
            .standard_filters(true)
            .require_git(false)
            .follow_links(false)
            .build();
        for entry in walker {
            let entry = entry.map_err(|error| SourceError::Traverse {
                path: input.to_owned(),
                message: error.to_string(),
            })?;
            if entry.file_type().is_some_and(|kind| kind.is_file()) {
                paths.push(entry.into_path());
            }
        }
        (input.to_path_buf(), paths)
    } else {
        return Err(SourceError::Traverse {
            path: input.to_owned(),
            message: "input is neither a regular file nor a directory".to_owned(),
        });
    };

    load_source_candidates(input, &root, candidates)
}

#[allow(dead_code)]
pub(crate) fn load_source_files(
    root: &Path,
    relative_paths: &[PathBuf],
) -> Result<SourceTree, SourceError> {
    let candidates = relative_paths.iter().map(|path| root.join(path)).collect();
    load_source_candidates(root, root, candidates)
}

fn load_source_candidates(
    reported_input: &Path,
    root: &Path,
    mut candidates: Vec<PathBuf>,
) -> Result<SourceTree, SourceError> {
    candidates.sort_by_key(|path| normalized_relative(root, path));
    let enumerated = candidates.len();
    let mut skipped = 0;
    let mut files = Vec::new();
    for absolute_path in candidates {
        let Some(language) = language_for_path(&absolute_path) else {
            skipped += 1;
            continue;
        };
        let path = relative_path(root, &absolute_path)?;
        let bytes: Arc<[u8]> = fs::read(&absolute_path)
            .map_err(|source| SourceError::Read {
                path: absolute_path.clone(),
                source,
            })?
            .into();
        files.push(SourceFile {
            absolute_path,
            path,
            language,
            bytes,
        });
    }
    Ok(SourceTree {
        root: normalized_path(reported_input)?,
        files,
        enumerated,
        skipped,
    })
}

pub fn parse_source(file: &SourceFile) -> Result<ParsedSource<'_>, SourceError> {
    if let Some((tree, has_syntax_errors, corpus)) = active_parse(file) {
        corpus.parse_tree_hits.fetch_add(1, Ordering::Relaxed);
        return Ok(ParsedSource {
            file,
            tree,
            has_syntax_errors,
        });
    }
    parse_source_uncached(file)
}

fn parse_source_uncached(file: &SourceFile) -> Result<ParsedSource<'_>, SourceError> {
    let mut parser = Parser::new();
    let language = match file.language {
        SourceLanguage::Rust => tree_sitter_rust::LANGUAGE.into(),
        SourceLanguage::Python => tree_sitter_python::LANGUAGE.into(),
        SourceLanguage::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        SourceLanguage::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        SourceLanguage::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        SourceLanguage::Go => tree_sitter_go::LANGUAGE.into(),
    };
    parser
        .set_language(&language)
        .map_err(|error| SourceError::ParserConfiguration {
            language: file.language.name(),
            message: error.to_string(),
        })?;
    let tree = parser
        .parse(file.bytes.as_ref(), None)
        .ok_or_else(|| SourceError::Parse(file.absolute_path.clone()))?;
    let has_syntax_errors = tree.root_node().has_error();
    Ok(ParsedSource {
        file,
        tree,
        has_syntax_errors,
    })
}

#[must_use]
pub fn language_for_path(path: &Path) -> Option<SourceLanguage> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "rs" => Some(SourceLanguage::Rust),
        "py" | "pyi" => Some(SourceLanguage::Python),
        "js" | "jsx" | "mjs" | "cjs" => Some(SourceLanguage::JavaScript),
        "ts" | "mts" | "cts" => Some(SourceLanguage::TypeScript),
        "tsx" => Some(SourceLanguage::Tsx),
        "go" => Some(SourceLanguage::Go),
        _ => None,
    }
}

fn active_corpus(input: &Path) -> Option<Arc<SourceCorpus>> {
    registry_read().get(&input_key(input)).and_then(Weak::upgrade)
}

fn active_parse(file: &SourceFile) -> Option<(Tree, bool, Arc<SourceCorpus>)> {
    for corpus in registry_read().values().filter_map(Weak::upgrade) {
        let Some(parsed) = corpus.parsed.get(&file.absolute_path) else {
            continue;
        };
        if Arc::ptr_eq(&parsed.bytes, &file.bytes) {
            return Some((
                parsed.tree.clone(),
                parsed.has_syntax_errors,
                corpus,
            ));
        }
    }
    None
}

fn registry() -> &'static RwLock<HashMap<PathBuf, Weak<SourceCorpus>>> {
    static REGISTRY: OnceLock<RwLock<HashMap<PathBuf, Weak<SourceCorpus>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

fn registry_read() -> std::sync::RwLockReadGuard<'static, HashMap<PathBuf, Weak<SourceCorpus>>> {
    registry()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn registry_write() -> std::sync::RwLockWriteGuard<'static, HashMap<PathBuf, Weak<SourceCorpus>>> {
    registry()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn input_key(input: &Path) -> PathBuf {
    if input.is_absolute() {
        input.to_owned()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(input)
    }
}

fn update_manifest_field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut rendered = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut rendered, "{byte:02x}");
    }
    rendered
}

fn relative_path(root: &Path, path: &Path) -> Result<String, SourceError> {
    let relative = path.strip_prefix(root).unwrap_or(path);
    normalized_path(relative)
}

fn normalized_relative(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    normalized_path(relative).unwrap_or_else(|_| format!("{relative:?}"))
}

fn normalized_path(path: &Path) -> Result<String, SourceError> {
    let mut normalized = String::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                let value = prefix
                    .as_os_str()
                    .to_str()
                    .ok_or_else(|| SourceError::NonUtf8(path.to_owned()))?;
                normalized.push_str(value);
            }
            Component::RootDir => normalized.push('/'),
            Component::CurDir => {
                if normalized.is_empty() {
                    normalized.push('.');
                }
            }
            Component::ParentDir => {
                if !normalized.is_empty() && !normalized.ends_with('/') {
                    normalized.push('/');
                }
                normalized.push_str("..");
            }
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| SourceError::NonUtf8(path.to_owned()))?;
                if !normalized.is_empty() && !normalized.ends_with('/') {
                    normalized.push('/');
                }
                normalized.push_str(part);
            }
        }
    }
    if normalized.is_empty() {
        normalized.push('.');
    }
    Ok(normalized)
}
