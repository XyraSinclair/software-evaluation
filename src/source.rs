//! Shared deterministic source discovery and tree-sitter parsing.

use std::fs;
use std::path::{Component, Path, PathBuf};

use ignore::WalkBuilder;
use serde::Serialize;
use thiserror::Error;
use tree_sitter::{Parser, Tree};

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
    pub bytes: Vec<u8>,
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

pub fn load_source_tree(input: &Path) -> Result<SourceTree, SourceError> {
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

    let (root, mut candidates) = if metadata.is_file() {
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

    candidates.sort_by_key(|path| normalized_relative(&root, path));
    let enumerated = candidates.len();
    let mut skipped = 0;
    let mut files = Vec::new();
    for absolute_path in candidates {
        let Some(language) = language_for_path(&absolute_path) else {
            skipped += 1;
            continue;
        };
        let path = relative_path(&root, &absolute_path)?;
        let bytes = fs::read(&absolute_path).map_err(|source| SourceError::Read {
            path: absolute_path.clone(),
            source,
        })?;
        files.push(SourceFile {
            absolute_path,
            path,
            language,
            bytes,
        });
    }

    Ok(SourceTree {
        root: normalized_path(input)?,
        files,
        enumerated,
        skipped,
    })
}

pub fn parse_source(file: &SourceFile) -> Result<ParsedSource<'_>, SourceError> {
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
        .parse(&file.bytes, None)
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
