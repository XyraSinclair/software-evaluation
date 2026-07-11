use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;

use software_evaluation::service::archive::{
    ArchiveError, ArchiveLimits, extract_zip, extract_zip_reader,
};
use software_evaluation::service::identity::{
    GithubRepoId, IdentityError, validate_owner, validate_repo,
};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

#[test]
fn github_identity_accepts_only_the_two_strict_ascii_components() {
    for owner in ["a", "A1", "octo-cat", &"a".repeat(39)] {
        assert_eq!(validate_owner(owner), Ok(()), "valid owner {owner:?}");
    }
    for repo in ["a", "repo.rs", "repo_name", "repo-name", &"r".repeat(100)] {
        assert_eq!(validate_repo(repo), Ok(()), "valid repo {repo:?}");
    }

    for owner in [
        "",
        "-owner",
        "owner-",
        "owner_name",
        "owner/name",
        "owner?ref=x",
        "https:",
        "é",
        &"a".repeat(40),
    ] {
        assert_eq!(
            validate_owner(owner),
            Err(IdentityError),
            "invalid owner {owner:?}"
        );
    }
    for repo in [
        "",
        ".",
        "..",
        "owner/repo",
        "repo?ref=x",
        "repo#main",
        "https://x",
        "répo",
        &"r".repeat(101),
    ] {
        assert_eq!(
            validate_repo(repo),
            Err(IdentityError),
            "invalid repo {repo:?}"
        );
    }

    let identity = GithubRepoId::parse("Octo-Cat", "Repo.Name")
        .expect("strict two-component identity is valid");
    assert_eq!(identity.key(), "octo-cat/repo.name");
}

#[derive(Clone, Copy)]
enum EntryKind {
    File,
    Directory,
    UnixMode(u32),
}

fn make_zip(entries: &[(&str, &[u8], EntryKind)]) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut output);
        for (name, body, kind) in entries {
            let options = match kind {
                EntryKind::UnixMode(mode) => SimpleFileOptions::default().unix_permissions(*mode),
                _ => SimpleFileOptions::default(),
            };
            match kind {
                EntryKind::Directory => writer
                    .add_directory(*name, options)
                    .expect("add ZIP directory fixture"),
                _ => {
                    writer
                        .start_file(*name, options)
                        .expect("add ZIP file fixture");
                    writer.write_all(body).expect("write ZIP fixture body");
                }
            }
        }
        writer.finish().expect("finish ZIP fixture");
    }
    output.into_inner()
}

fn duplicate_path_zip() -> Vec<u8> {
    let mut bytes = make_zip(&[
        ("root/file", b"a", EntryKind::File),
        ("root/fyle", b"b", EntryKind::File),
    ]);
    let needle = b"root/fyle";
    let replacement = b"root/file";
    let mut replacements = 0;
    for offset in 0..=bytes.len() - needle.len() {
        if &bytes[offset..offset + needle.len()] == needle {
            bytes[offset..offset + needle.len()].copy_from_slice(replacement);
            replacements += 1;
        }
    }
    assert_eq!(
        replacements, 2,
        "ZIP name must occur in local and central headers"
    );
    bytes
}

fn extract(
    bytes: &[u8],
    limits: ArchiveLimits,
) -> Result<(TempDir, std::path::PathBuf), ArchiveError> {
    let workspace = TempDir::new().expect("temporary archive workspace");
    let archive = workspace.path().join("repository.zip");
    fs::write(&archive, bytes).expect("write ZIP fixture");
    let destination = workspace.path().join("unpacked");
    let root = extract_zip(&archive, &destination, limits)?;
    Ok((workspace, root))
}

fn generous_limits() -> ArchiveLimits {
    ArchiveLimits {
        compressed_bytes: 1_000_000,
        entries: 100,
        expanded_bytes: 1_000_000,
        file_bytes: 1_000_000,
        path_bytes: 512,
        path_components: 32,
        expansion_ratio: 10_000,
    }
}

#[test]
fn zip_positive_control_extracts_exactly_one_root() {
    let bytes = make_zip(&[
        ("project-commit/", b"", EntryKind::Directory),
        (
            "project-commit/src/lib.rs",
            b"pub fn answer() -> u8 { 42 }\n",
            EntryKind::File,
        ),
    ]);
    let (_workspace, root) = extract(&bytes, generous_limits()).expect("safe GitHub-shaped ZIP");

    assert!(root.ends_with("project-commit"));
    assert_eq!(
        fs::read(root.join("src/lib.rs")).expect("read extracted file"),
        b"pub fn answer() -> u8 { 42 }\n"
    );
}

#[test]
fn zip_rejects_paths_that_can_escape_or_change_identity() {
    let cases: &[(&str, Vec<u8>, ArchiveError)] = &[
        (
            "parent traversal",
            make_zip(&[("root/../escape", b"x", EntryKind::File)]),
            ArchiveError::UnsafePath,
        ),
        (
            "absolute path",
            make_zip(&[("/root/file", b"x", EntryKind::File)]),
            ArchiveError::UnsafePath,
        ),
        (
            "backslash path",
            make_zip(&[("root\\escape", b"x", EntryKind::File)]),
            ArchiveError::UnsafePath,
        ),
        (
            "duplicate normalized path",
            duplicate_path_zip(),
            ArchiveError::DuplicatePath,
        ),
        (
            "multiple roots",
            make_zip(&[
                ("one/file", b"a", EntryKind::File),
                ("two/file", b"b", EntryKind::File),
            ]),
            ArchiveError::MultipleRoots,
        ),
        (
            "symbolic link",
            make_zip(&[("root/link", b"target", EntryKind::UnixMode(0o120777))]),
            ArchiveError::UnsupportedEntry,
        ),
        (
            "special device",
            make_zip(&[("root/device", b"", EntryKind::UnixMode(0o060666))]),
            ArchiveError::UnsupportedEntry,
        ),
    ];

    for (name, bytes, expected) in cases {
        let actual = extract(bytes, generous_limits()).expect_err(name);
        assert_eq!(&actual, expected, "{name}");
    }
}

#[test]
fn zip_quota_boundaries_accept_the_limit_and_reject_one_over() {
    let four_bytes = make_zip(&[("root/file", b"1234", EntryKind::File)]);
    let mut limits = generous_limits();
    limits.file_bytes = 4;
    limits.expanded_bytes = 4;
    extract(&four_bytes, limits).expect("file exactly at file and expanded limits");

    let five_bytes = make_zip(&[("root/file", b"12345", EntryKind::File)]);
    assert_eq!(
        extract(&five_bytes, limits).expect_err("file one over quota"),
        ArchiveError::TooLarge
    );

    let aggregate_exact = make_zip(&[
        ("root/a", b"12", EntryKind::File),
        ("root/b", b"34", EntryKind::File),
    ]);
    limits = generous_limits();
    limits.expanded_bytes = 4;
    extract(&aggregate_exact, limits).expect("aggregate expanded bytes exactly at limit");
    let aggregate_over = make_zip(&[
        ("root/a", b"12", EntryKind::File),
        ("root/b", b"345", EntryKind::File),
    ]);
    assert_eq!(
        extract(&aggregate_over, limits).expect_err("aggregate expanded bytes one over"),
        ArchiveError::TooLarge
    );

    let two_entries = make_zip(&[
        ("root/a", b"a", EntryKind::File),
        ("root/b", b"b", EntryKind::File),
    ]);
    limits = generous_limits();
    limits.entries = 2;
    extract(&two_entries, limits).expect("entry count exactly at limit");
    limits.entries = 1;
    assert_eq!(
        extract(&two_entries, limits).expect_err("entry count one over"),
        ArchiveError::TooManyEntries
    );

    limits = generous_limits();
    limits.expansion_ratio = 2;
    let ratio_workspace = TempDir::new().expect("expansion ratio workspace");
    extract_zip_reader(
        Cursor::new(&four_bytes),
        &ratio_workspace.path().join("exact"),
        limits,
        2,
    )
    .expect("expanded bytes exactly at ratio limit");
    limits.expansion_ratio = 1;
    assert_eq!(
        extract_zip_reader(
            Cursor::new(&four_bytes),
            &ratio_workspace.path().join("over"),
            limits,
            2
        ),
        Err(ArchiveError::TooLarge)
    );

    let workspace = TempDir::new().expect("compressed quota workspace");
    let archive = workspace.path().join("repository.zip");
    fs::write(&archive, &four_bytes).expect("write compressed quota fixture");
    let compressed_len = fs::metadata(&archive).expect("archive metadata").len();
    limits = generous_limits();
    limits.compressed_bytes = compressed_len;
    extract_zip(&archive, &workspace.path().join("exact"), limits)
        .expect("compressed stream exactly at limit");
    limits.compressed_bytes = compressed_len - 1;
    assert_eq!(
        extract_zip(&archive, &workspace.path().join("over"), limits),
        Err(ArchiveError::TooLarge)
    );
}

#[test]
fn zip_path_length_and_component_boundaries_are_enforced() {
    let bytes = make_zip(&[("root/a/file", b"x", EntryKind::File)]);
    let path_len = "root/a/file".len();
    let mut limits = generous_limits();
    limits.path_bytes = path_len;
    limits.path_components = 3;
    extract(&bytes, limits).expect("path exactly at both limits");

    limits.path_bytes = path_len - 1;
    assert_eq!(
        extract(&bytes, limits).expect_err("path byte limit exceeded"),
        ArchiveError::UnsafePath
    );
    limits = generous_limits();
    limits.path_components = 2;
    assert_eq!(
        extract(&bytes, limits).expect_err("path component limit exceeded"),
        ArchiveError::UnsafePath
    );
}

#[test]
fn archive_errors_never_disclose_local_paths() {
    let workspace = TempDir::new().expect("error path workspace");
    let secret = workspace.path().join("sensitive-tenant-name.zip");
    fs::write(&secret, b"not a zip").expect("write malformed archive");
    let error = extract_zip(&secret, &workspace.path().join("output"), generous_limits())
        .expect_err("malformed archive must fail")
        .to_string();

    assert_eq!(error, "archive is malformed");
    assert!(!error.contains("sensitive-tenant-name"));
    assert!(!error.contains(Path::new("/tmp").to_string_lossy().as_ref()));
}
