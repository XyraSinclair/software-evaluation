use std::fs;
use std::path::Path;

use software_evaluation::source::{
    SourceError, SourceLanguage, language_for_path, load_source_tree, parse_source,
};
use tempfile::TempDir;

fn write_file(root: &Path, relative: &str, bytes: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create source fixture parent");
    }
    fs::write(path, bytes).expect("write source fixture");
}

#[test]
fn language_detection_covers_every_supported_extension_family() {
    let cases = [
        ("unit.rs", Some(SourceLanguage::Rust)),
        ("module.py", Some(SourceLanguage::Python)),
        ("module.pyi", Some(SourceLanguage::Python)),
        ("module.js", Some(SourceLanguage::JavaScript)),
        ("module.jsx", Some(SourceLanguage::JavaScript)),
        ("module.mjs", Some(SourceLanguage::JavaScript)),
        ("module.cjs", Some(SourceLanguage::JavaScript)),
        ("module.ts", Some(SourceLanguage::TypeScript)),
        ("module.mts", Some(SourceLanguage::TypeScript)),
        ("module.cts", Some(SourceLanguage::TypeScript)),
        ("module.tsx", Some(SourceLanguage::Tsx)),
        ("module.go", Some(SourceLanguage::Go)),
        ("MODULE.RS", Some(SourceLanguage::Rust)),
        ("notes.txt", None),
        ("Makefile", None),
    ];

    for (path, expected) in cases {
        assert_eq!(
            language_for_path(Path::new(path)),
            expected,
            "wrong language classification for {path}"
        );
    }
}

#[test]
fn directory_discovery_is_ignored_sorted_counted_and_parseable() {
    let directory = TempDir::new().expect("temporary source tree");
    let root = directory.path();
    let fixtures = [
        (
            "a/rust.rs",
            "fn value() -> u8 { 1 }\n",
            SourceLanguage::Rust,
            false,
        ),
        (
            "a/types.py",
            "def value() -> int: ...\n",
            SourceLanguage::Python,
            false,
        ),
        (
            "a/types.pyi",
            "def value() -> int: ...\n",
            SourceLanguage::Python,
            false,
        ),
        (
            "b/common.cjs",
            "module.exports = function value() { return 1; };\n",
            SourceLanguage::JavaScript,
            false,
        ),
        (
            "b/module.js",
            "export function value() { return 1; }\n",
            SourceLanguage::JavaScript,
            false,
        ),
        (
            "b/module.jsx",
            "export const value = <div>one</div>;\n",
            SourceLanguage::JavaScript,
            false,
        ),
        (
            "b/module.mjs",
            "export const value = 1;\n",
            SourceLanguage::JavaScript,
            false,
        ),
        (
            "c/module.cts",
            "export const value: number = 1;\n",
            SourceLanguage::TypeScript,
            false,
        ),
        (
            "c/module.mts",
            "export const value: number = 1;\n",
            SourceLanguage::TypeScript,
            false,
        ),
        (
            "c/module.ts",
            "export const value: number = 1;\n",
            SourceLanguage::TypeScript,
            false,
        ),
        (
            "c/view.tsx",
            "export const value = <div>one</div>;\n",
            SourceLanguage::Tsx,
            false,
        ),
        (
            "d/main.go",
            "package main\nfunc value() int { return 1 }\n",
            SourceLanguage::Go,
            false,
        ),
        (
            "z/malformed.rs",
            "fn broken( {\n",
            SourceLanguage::Rust,
            true,
        ),
    ];
    for (path, source, _, _) in fixtures {
        write_file(root, path, source);
    }
    write_file(root, "notes.txt", "unsupported\n");
    write_file(root, "ignored/hidden.py", "def hidden(): return 1\n");
    write_file(root, ".gitignore", "ignored/\n");

    let expected_paths: Vec<_> = fixtures.iter().map(|(path, _, _, _)| *path).collect();
    let expected_enumerated = fixtures.len() + 1;
    let expected_skipped = 1;
    let expected_analyzed = fixtures.len();

    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("a/rust.rs"), root.join("linked.rs"))
        .expect("create source symlink fixture");

    let tree = load_source_tree(root).expect("discover fixture source tree");
    let actual_paths: Vec<_> = tree.files.iter().map(|file| file.path.as_str()).collect();

    assert_eq!(
        actual_paths, expected_paths,
        "source paths must be lexical and exact"
    );
    assert_eq!(tree.enumerated, expected_enumerated);
    assert_eq!(tree.skipped, expected_skipped);
    assert_eq!(tree.files.len(), expected_analyzed);

    for (file, (expected_path, expected_bytes, expected_language, expected_error)) in
        tree.files.iter().zip(fixtures)
    {
        assert_eq!(file.path, expected_path);
        assert_eq!(file.bytes, expected_bytes.as_bytes());
        assert_eq!(file.language, expected_language);
        let parsed =
            parse_source(file).unwrap_or_else(|error| panic!("parse {expected_path}: {error}"));
        assert_eq!(
            parsed.has_syntax_errors, expected_error,
            "wrong syntax-error verdict for {expected_path}"
        );
    }
}

#[test]
fn direct_file_discovery_preserves_file_contract_and_unsupported_files_are_empty() {
    let directory = TempDir::new().expect("temporary direct-file fixtures");
    let supported = directory.path().join("single.go");
    let unsupported = directory.path().join("single.txt");
    fs::write(&supported, "package single\n").expect("write supported direct file");
    fs::write(&unsupported, "plain text\n").expect("write unsupported direct file");

    let expected_supported_paths = vec!["single.go"];
    let expected_unsupported_paths: Vec<&str> = Vec::new();

    let supported_tree = load_source_tree(&supported).expect("load supported direct file");
    assert_eq!(
        supported_tree
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        expected_supported_paths
    );
    assert_eq!(supported_tree.enumerated, 1);
    assert_eq!(supported_tree.skipped, 0);
    assert_eq!(supported_tree.files.len(), 1);

    let unsupported_tree = load_source_tree(&unsupported).expect("load unsupported direct file");
    assert_eq!(
        unsupported_tree
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        expected_unsupported_paths
    );
    assert_eq!(unsupported_tree.enumerated, 1);
    assert_eq!(unsupported_tree.skipped, 1);
    assert_eq!(unsupported_tree.files.len(), 0);
}

#[test]
fn missing_input_reports_the_missing_path() {
    let directory = TempDir::new().expect("temporary missing-path fixture");
    let missing = directory.path().join("absent.rs");

    let error = load_source_tree(&missing).expect_err("missing input must fail discovery");
    assert!(matches!(error, SourceError::Missing(path) if path == missing));
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed_and_direct_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new().expect("temporary symlink fixtures");
    let target = directory.path().join("target.rs");
    let link = directory.path().join("link.rs");
    fs::write(&target, "fn target() {}\n").expect("write symlink target");
    symlink(&target, &link).expect("create direct symlink fixture");

    let expected_paths = vec!["target.rs"];
    let tree = load_source_tree(directory.path()).expect("walk directory containing a symlink");
    assert_eq!(
        tree.files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        expected_paths
    );
    assert_eq!(tree.enumerated, 1);
    assert_eq!(tree.skipped, 0);

    let error = load_source_tree(&link).expect_err("direct symlink must be rejected");
    assert!(matches!(error, SourceError::Symlink(path) if path == link));
}

#[cfg(unix)]
#[test]
fn non_file_non_directory_input_is_rejected() {
    use std::os::unix::net::UnixListener;

    let directory = TempDir::new().expect("temporary special-file fixture");
    let socket = directory.path().join("source.sock");
    let _listener = UnixListener::bind(&socket).expect("create Unix socket fixture");

    let error = load_source_tree(&socket).expect_err("special input must be rejected");
    assert!(matches!(
        error,
        SourceError::Traverse { path, message }
            if path == socket && message == "input is neither a regular file nor a directory"
    ));
}
