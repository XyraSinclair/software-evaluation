//! Deterministic, evidence-producing observations about committed repository shape.
//!
//! These programs intentionally report separate proxy measurements. They do not
//! combine them into a quality score or treat repository shape as proof of
//! correctness, maintainability, or value.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::kernel::{
    Applicability, ArtifactSnapshot, CriterionProgram, EpistemicClass, EvidenceItem,
    ProgramContext, ProgramDescriptor, ProgramFailure, ProgramOutput, ResourceVector,
};

const CLASSIFIER_VERSION: &str = "repo-lexical-v1";
const STATIC_LIMIT_MS: u64 = 2_000;
const CHANGE_LIMIT_MS: u64 = 5_000;

const CATEGORY_SOURCE: &str = "source";
const CATEGORY_TEST: &str = "test";
const CATEGORY_DOCUMENTATION: &str = "documentation";
const CATEGORY_CONFIGURATION: &str = "configuration";
const CATEGORY_GENERATED: &str = "generated_or_vendor";
const CATEGORY_OTHER: &str = "other";

const ALL_CATEGORIES: [&str; 6] = [
    CATEGORY_SOURCE,
    CATEGORY_TEST,
    CATEGORY_DOCUMENTATION,
    CATEGORY_CONFIGURATION,
    CATEGORY_GENERATED,
    CATEGORY_OTHER,
];

/// Failures encountered while identifying or reading a Git repository.
#[derive(Debug, Error)]
pub enum RepoError {
    #[error("invalid repository root {path}: {reason}")]
    InvalidRoot { path: PathBuf, reason: String },

    #[error("repository has tracked uncommitted changes: {root}")]
    DirtyWorktree { root: PathBuf },

    #[error("failed to invoke {command}: {source}")]
    GitInvocation {
        command: String,
        #[source]
        source: io::Error,
    },

    #[error("{command} exited unsuccessfully ({status}): {stderr}")]
    GitExit {
        command: String,
        status: String,
        stderr: String,
    },

    #[error("could not parse output from {command}: {reason}")]
    GitParse { command: String, reason: String },

    #[error("invalid repository profile configuration: {0}")]
    InvalidConfig(String),
}

/// Capture the immutable commit and tree identity used by repository programs.
pub fn snapshot_git_repo(root: &Path) -> Result<ArtifactSnapshot, RepoError> {
    let metadata = fs::metadata(root).map_err(|source| RepoError::InvalidRoot {
        path: root.to_path_buf(),
        reason: if source.kind() == io::ErrorKind::NotFound {
            "path does not exist".to_owned()
        } else {
            source.to_string()
        },
    })?;
    if !metadata.is_dir() {
        return Err(RepoError::InvalidRoot {
            path: root.to_path_buf(),
            reason: "path is not a directory".to_owned(),
        });
    }

    let top_output = run_git(Some(root), &["rev-parse", "--show-toplevel"])?;
    let top_text = single_text_line(&top_output.stdout, &top_output.command)?;
    let reported_root = PathBuf::from(top_text);
    let canonical_root =
        fs::canonicalize(&reported_root).map_err(|source| RepoError::InvalidRoot {
            path: reported_root.clone(),
            reason: format!(
                "Git reported a repository root that cannot be canonicalized: {source}"
            ),
        })?;

    let status = run_git(
        Some(&canonical_root),
        &["status", "--porcelain", "--untracked-files=no"],
    )?;
    if !status.stdout.is_empty() {
        return Err(RepoError::DirtyWorktree {
            root: canonical_root,
        });
    }

    let revision_output = run_git(Some(&canonical_root), &["rev-parse", "HEAD"])?;
    let revision = single_oid(&revision_output.stdout, &revision_output.command)?;
    let tree_spec = format!("{revision}^{{tree}}");
    let tree_output = run_git(Some(&canonical_root), &["rev-parse", &tree_spec])?;
    let tree_digest = single_oid(&tree_output.stdout, &tree_output.command)?;

    let directory_name = canonical_root
        .file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| RepoError::InvalidRoot {
            path: canonical_root.clone(),
            reason: "canonical repository root has no directory name".to_owned(),
        })?;
    let short_revision = revision.get(..12).ok_or_else(|| RepoError::GitParse {
        command: revision_output.command,
        reason: "revision is shorter than 12 hexadecimal characters".to_owned(),
    })?;

    Ok(ArtifactSnapshot {
        id: format!("{directory_name}@{short_revision}"),
        root: canonical_root,
        revision,
        tree_digest,
        kind: "git-repository".to_owned(),
    })
}

/// Configuration for the bounded, no-merge Git history sample.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoProfileConfig {
    pub history_commits: usize,
}

impl Default for RepoProfileConfig {
    fn default() -> Self {
        Self {
            history_commits: 200,
        }
    }
}

impl RepoProfileConfig {
    pub fn validate(&self) -> Result<(), RepoError> {
        if (1..=10_000).contains(&self.history_commits) {
            Ok(())
        } else {
            Err(RepoError::InvalidConfig(format!(
                "history_commits must be in 1..=10_000, got {}",
                self.history_commits
            )))
        }
    }
}

/// Static byte and path-shape observations for the committed Git tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StaticRepoShape {
    pub classifier_version: String,
    pub tracked_files: u64,
    pub tracked_bytes: u64,
    pub category_files: BTreeMap<String, u64>,
    pub category_bytes: BTreeMap<String, u64>,
    pub source_file_size_median: Option<u64>,
    pub source_file_size_p90: Option<u64>,
    pub largest_source_file_bytes: u64,
    pub largest_source_file_path: Option<String>,
    pub largest_source_file_share: Option<f64>,
    pub top_decile_source_byte_share: Option<f64>,
    pub normalized_source_size_entropy: Option<f64>,
    pub effective_source_files: Option<f64>,
    pub normalized_top_level_source_entropy: Option<f64>,
    pub effective_top_level_components: Option<f64>,
    pub test_to_source_bytes: Option<f64>,
    pub documentation_to_source_bytes: Option<f64>,
    pub generated_or_vendor_byte_share: f64,
    pub max_path_depth: u64,
    pub limitations: Vec<String>,
}

/// A leaf criterion program for the committed tree's lexical and byte shape.
#[derive(Debug, Clone, Copy, Default)]
pub struct StaticRepoShapeProgram;

impl StaticRepoShapeProgram {
    pub const fn new() -> Self {
        Self
    }
}

impl CriterionProgram for StaticRepoShapeProgram {
    fn descriptor(&self) -> ProgramDescriptor {
        ProgramDescriptor {
            id: "repo.static-shape".to_owned(),
            version: "1".to_owned(),
            criterion: "repository-shape".to_owned(),
            epistemic_class: EpistemicClass::Proxy,
            deterministic: true,
            description: "Deterministic committed-tree byte, path, and lexical-category shape; no overall quality score".to_owned(),
        }
    }

    fn applicability(&self, artifact: &ArtifactSnapshot) -> Applicability {
        git_repository_applicability(artifact)
    }

    fn estimate(&self, _artifact: &ArtifactSnapshot) -> Result<ResourceVector, ProgramFailure> {
        Ok(resource_estimate(STATIC_LIMIT_MS))
    }

    fn run(&self, context: &ProgramContext<'_>) -> Result<ProgramOutput, ProgramFailure> {
        let artifact = context.artifact;
        let args = vec![
            OsString::from("ls-tree"),
            OsString::from("-r"),
            OsString::from("-z"),
            OsString::from("--long"),
            OsString::from(&artifact.revision),
        ];
        let command = run_git_os(Some(&artifact.root), &args).map_err(repo_tool_failure)?;
        let git_version = read_git_version().map_err(repo_tool_failure)?;
        let entries =
            parse_ls_tree(&command.stdout, &command.command).map_err(repo_tool_failure)?;
        let limitations = static_limitations(&git_version);
        let observation = build_static_shape(entries, limitations.clone())?;
        let observation = serde_json::to_value(observation).map_err(|error| {
            ProgramFailure::invariant(format!(
                "could not serialize static repository shape: {error}"
            ))
        })?;

        Ok(ProgramOutput {
            observation,
            evidence: vec![git_evidence(
                &command,
                &git_version,
                "Raw NUL-delimited Git tree listing used for committed blob sizes and paths",
            )],
            belief_updates: Vec::new(),
            resources: output_resources(command.stdout.len())?,
            continuation_hints: Vec::new(),
            limitations,
        })
    }
}

/// Bounded Git-history observations about change concentration and cochange.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GitChangeShape {
    pub requested_commits: usize,
    pub commits_analyzed: u64,
    pub commits_with_binary_changes: u64,
    pub unique_changed_files: u64,
    pub total_line_change_mass: u64,
    pub files_changed_mean: f64,
    pub files_changed_median: u64,
    pub files_changed_p90: u64,
    pub files_changed_max: u64,
    pub normalized_change_mass_entropy: Option<f64>,
    pub effective_hotspot_files: Option<f64>,
    pub top_decile_change_mass_share: Option<f64>,
    pub largest_hotspot_path: Option<String>,
    pub largest_hotspot_mass: u64,
    pub cross_top_level_pair_ratio: Option<f64>,
    pub broad_commit_rate: f64,
    pub source_commits: u64,
    pub source_test_cochange_rate: Option<f64>,
    pub source_documentation_cochange_rate: Option<f64>,
    pub limitations: Vec<String>,
}

/// A leaf criterion program for bounded, committed Git change topology.
#[derive(Debug, Clone)]
pub struct GitChangeShapeProgram {
    config: RepoProfileConfig,
}

impl GitChangeShapeProgram {
    pub fn new(config: RepoProfileConfig) -> Result<Self, RepoError> {
        config.validate()?;
        Ok(Self { config })
    }
}

impl CriterionProgram for GitChangeShapeProgram {
    fn descriptor(&self) -> ProgramDescriptor {
        ProgramDescriptor {
            id: "repo.git-change-shape".to_owned(),
            version: "2".to_owned(),
            criterion: "evolvability.change-topology".to_owned(),
            epistemic_class: EpistemicClass::Proxy,
            deterministic: true,
            description: "Deterministic bounded Git change-mass and layout-cochange proxies; no overall quality score".to_owned(),
        }
    }

    fn applicability(&self, artifact: &ArtifactSnapshot) -> Applicability {
        git_repository_applicability(artifact)
    }

    fn estimate(&self, _artifact: &ArtifactSnapshot) -> Result<ResourceVector, ProgramFailure> {
        Ok(resource_estimate(CHANGE_LIMIT_MS))
    }

    fn run(&self, context: &ProgramContext<'_>) -> Result<ProgramOutput, ProgramFailure> {
        let artifact = context.artifact;
        let history =
            scan_file_history(artifact, self.config.history_commits).map_err(repo_tool_failure)?;
        let limitations = change_limitations(&history.git_version, self.config.history_commits);
        let observation = build_change_shape(
            self.config.history_commits,
            history.commits,
            limitations.clone(),
        )?;
        let observation = serde_json::to_value(observation).map_err(|error| {
            ProgramFailure::invariant(format!("could not serialize Git change shape: {error}"))
        })?;

        Ok(ProgramOutput {
            observation,
            evidence: vec![EvidenceItem {
                kind: "git-command-stdout".to_owned(),
                locator: history.command.clone(),
                digest: Some(history.stdout_sha256.clone()),
                description: format!(
                    "Raw commit-separated Git numstat history used for change-shape metrics; executable reported {}",
                    history.git_version
                ),
            }],
            belief_updates: Vec::new(),
            resources: output_resources(usize::try_from(history.stdout_bytes).map_err(|_| {
                ProgramFailure::invariant("Git stdout length cannot be represented as usize")
            })?)?,
            continuation_hints: Vec::new(),
            limitations,
        })
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct CommittedRegularFile {
    pub(crate) path: Vec<u8>,
    pub(crate) size: u64,
    pub(crate) mode: Vec<u8>,
    pub(crate) oid: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct FileHistoryScan {
    pub(crate) commits: Vec<ParsedCommit>,
    pub(crate) truncated: bool,
    pub(crate) git_version: String,
    pub(crate) command: String,
    pub(crate) stdout_sha256: String,
    pub(crate) stdout_bytes: u64,
}
#[derive(Debug)]
pub(crate) struct CommittedTreeScan {
    pub(crate) files: Vec<CommittedRegularFile>,
    pub(crate) command: String,
    pub(crate) stdout_sha256: String,
    pub(crate) stdout_bytes: u64,
    pub(crate) git_version: String,
}

#[derive(Debug)]
pub(crate) struct CommittedBlobRead {
    pub(crate) blobs: Vec<Vec<u8>>,
    pub(crate) command: String,
    pub(crate) request_sha256: String,
    pub(crate) stdout_sha256: String,
    pub(crate) stdout_bytes: u64,
}

pub(crate) fn scan_committed_regular_files(
    artifact: &ArtifactSnapshot,
) -> Result<CommittedTreeScan, RepoError> {
    let args = vec![
        OsString::from("ls-tree"),
        OsString::from("-r"),
        OsString::from("-z"),
        OsString::from("--long"),
        OsString::from(&artifact.revision),
    ];
    let output = run_git_os(Some(&artifact.root), &args)?;
    let stdout_bytes = u64::try_from(output.stdout.len())
        .map_err(|_| git_parse(&output.command, "ls-tree stdout length overflowed u64"))?;
    let files = parse_ls_tree(&output.stdout, &output.command)?
        .into_iter()
        .filter(|entry| matches!(entry.mode.as_slice(), b"100644" | b"100755"))
        .map(|entry| CommittedRegularFile {
            path: entry.path,
            size: entry.size,
            mode: entry.mode,
            oid: entry.oid,
        })
        .collect();
    Ok(CommittedTreeScan {
        files,
        command: output.command,
        stdout_sha256: sha256_hex(&output.stdout),
        stdout_bytes,
        git_version: read_git_version()?,
    })
}

pub(crate) fn read_committed_blobs(
    artifact: &ArtifactSnapshot,
    files: &[CommittedRegularFile],
) -> Result<CommittedBlobRead, RepoError> {
    let command_name = argv_locator(&[
        OsString::from("git"),
        OsString::from("-C"),
        artifact.root.as_os_str().to_owned(),
        OsString::from("cat-file"),
        OsString::from("--batch"),
    ]);
    let mut request = Vec::new();
    for file in files {
        request.extend_from_slice(&file.oid);
        request.push(b'\n');
    }
    let request_sha256 = sha256_hex(&request);
    let mut child = Command::new("git")
        .arg("-C")
        .arg(&artifact.root)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| RepoError::GitInvocation {
            command: command_name.clone(),
            source,
        })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| git_parse(&command_name, "cat-file stdin was unavailable"))?;
    // `cat-file` streams a response per request. Write stdin while
    // `wait_with_output` drains stdout, or large batches can deadlock when both
    // pipes fill.
    let writer = std::thread::spawn(move || stdin.write_all(&request));
    let output = child.wait_with_output();
    let write_result = writer
        .join()
        .map_err(|_| git_parse(&command_name, "cat-file stdin writer panicked"))?;
    write_result.map_err(|source| RepoError::GitInvocation {
        command: command_name.clone(),
        source,
    })?;
    let output = output.map_err(|source| RepoError::GitInvocation {
        command: command_name.clone(),
        source,
    })?;
    let output = successful_git_output(command_name.clone(), output)?;
    let mut remaining = output.stdout.as_slice();
    let mut blobs = Vec::with_capacity(files.len());
    for file in files {
        let newline = remaining
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(|| git_parse(&command_name, "cat-file response header was unterminated"))?;
        let header = &remaining[..newline];
        remaining = &remaining[newline + 1..];
        let fields = header.split(|byte| *byte == b' ').collect::<Vec<_>>();
        if fields.len() != 3 || fields[0] != file.oid || fields[1] != b"blob" {
            return Err(git_parse(
                &command_name,
                "cat-file response identity or type did not match request",
            ));
        }
        let size = parse_u64_ascii(fields[2])
            .ok_or_else(|| git_parse(&command_name, "cat-file response had invalid size"))?;
        if size != file.size {
            return Err(git_parse(
                &command_name,
                "cat-file blob size disagreed with committed tree",
            ));
        }
        let size = usize::try_from(size)
            .map_err(|_| git_parse(&command_name, "cat-file blob size exceeded usize"))?;
        let blob = remaining
            .get(..size)
            .ok_or_else(|| git_parse(&command_name, "cat-file blob was truncated"))?;
        if remaining.get(size) != Some(&b'\n') {
            return Err(git_parse(
                &command_name,
                "cat-file blob lacked trailing delimiter",
            ));
        }
        blobs.push(blob.to_vec());
        remaining = &remaining[size + 1..];
    }
    if !remaining.is_empty() {
        return Err(git_parse(&command_name, "cat-file returned trailing bytes"));
    }
    let stdout_bytes = u64::try_from(output.stdout.len())
        .map_err(|_| git_parse(&command_name, "cat-file stdout length overflowed u64"))?;
    Ok(CommittedBlobRead {
        blobs,
        command: command_name,
        request_sha256,
        stdout_sha256: sha256_hex(&output.stdout),
        stdout_bytes,
    })
}

pub(crate) fn scan_file_history(
    artifact: &ArtifactSnapshot,
    requested_commits: usize,
) -> Result<FileHistoryScan, RepoError> {
    let fetch = requested_commits.checked_add(1).ok_or_else(|| {
        RepoError::InvalidConfig("history commit count overflowed usize".to_owned())
    })?;
    let args = vec![
        OsString::from("log"),
        OsString::from("--no-merges"),
        OsString::from("--no-renames"),
        OsString::from("--format=%x1e%H%x00%ct%x00"),
        OsString::from("--numstat"),
        OsString::from("-n"),
        OsString::from(fetch.to_string()),
        OsString::from(&artifact.revision),
        OsString::from("--"),
    ];
    let output = run_git_os(Some(&artifact.root), &args)?;
    let mut commits = parse_git_log(&output.stdout, &output.command)?;
    let truncated = commits.len() > requested_commits;
    commits.truncate(requested_commits);
    let stdout_bytes = u64::try_from(output.stdout.len())
        .map_err(|_| git_parse(&output.command, "stdout byte length overflowed u64"))?;
    Ok(FileHistoryScan {
        commits,
        truncated,
        git_version: read_git_version()?,
        command: output.command,
        stdout_sha256: sha256_hex(&output.stdout),
        stdout_bytes,
    })
}

#[derive(Debug)]
struct GitCommandOutput {
    command: String,
    stdout: Vec<u8>,
}

fn run_git(root: Option<&Path>, args: &[&str]) -> Result<GitCommandOutput, RepoError> {
    let owned = args.iter().map(OsString::from).collect::<Vec<_>>();
    run_git_os(root, &owned)
}

fn run_git_os(root: Option<&Path>, args: &[OsString]) -> Result<GitCommandOutput, RepoError> {
    let mut argv = Vec::with_capacity(args.len() + if root.is_some() { 3 } else { 1 });
    argv.push(OsString::from("git"));
    if let Some(root) = root {
        argv.push(OsString::from("-C"));
        argv.push(root.as_os_str().to_owned());
    }
    argv.extend(args.iter().cloned());
    let locator = argv_locator(&argv);

    let mut command = Command::new("git");
    if let Some(root) = root {
        command.arg("-C").arg(root);
    }
    command.args(args);
    let output = command
        .output()
        .map_err(|source| RepoError::GitInvocation {
            command: locator.clone(),
            source,
        })?;
    successful_git_output(locator, output)
}

fn successful_git_output(command: String, output: Output) -> Result<GitCommandOutput, RepoError> {
    if !output.status.success() {
        return Err(RepoError::GitExit {
            command,
            status: output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "terminated by signal".to_owned()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(GitCommandOutput {
        command,
        stdout: output.stdout,
    })
}

fn read_git_version() -> Result<String, RepoError> {
    let output = run_git(None, &["--version"])?;
    single_text_line(&output.stdout, &output.command)
}

fn single_text_line(bytes: &[u8], command: &str) -> Result<String, RepoError> {
    let bytes = strip_one_line_ending(bytes);
    if bytes.is_empty() {
        return Err(RepoError::GitParse {
            command: command.to_owned(),
            reason: "stdout was empty".to_owned(),
        });
    }
    if bytes.contains(&b'\n') || bytes.contains(&b'\r') {
        return Err(RepoError::GitParse {
            command: command.to_owned(),
            reason: "expected exactly one output line".to_owned(),
        });
    }
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|error| RepoError::GitParse {
            command: command.to_owned(),
            reason: format!("stdout was not UTF-8: {error}"),
        })
}

fn single_oid(bytes: &[u8], command: &str) -> Result<String, RepoError> {
    let oid = single_text_line(bytes, command)?;
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RepoError::GitParse {
            command: command.to_owned(),
            reason: "expected a full 40- or 64-character hexadecimal object id".to_owned(),
        });
    }
    Ok(oid.to_ascii_lowercase())
}

fn strip_one_line_ending(mut bytes: &[u8]) -> &[u8] {
    if let Some(stripped) = bytes.strip_suffix(b"\n") {
        bytes = stripped;
    }
    if let Some(stripped) = bytes.strip_suffix(b"\r") {
        bytes = stripped;
    }
    bytes
}

fn argv_locator(argv: &[OsString]) -> String {
    argv.iter()
        .map(|arg| format!("{:?}", arg.as_os_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn repo_tool_failure(error: RepoError) -> ProgramFailure {
    ProgramFailure::tool(error.to_string())
}

fn git_repository_applicability(artifact: &ArtifactSnapshot) -> Applicability {
    if artifact.kind == "git-repository" {
        Applicability::Applicable
    } else {
        Applicability::Inapplicable {
            reason: format!("artifact kind {:?} is not git-repository", artifact.kind),
        }
    }
}

fn resource_estimate(wall_time_ms: u64) -> ResourceVector {
    ResourceVector {
        usd: 0.0,
        wall_time_ms,
        cpu_time_ms: None,
        peak_memory_bytes: None,
        bytes_read: 0,
        bytes_written: 0,
        programs: 0,
    }
}

fn output_resources(bytes_read: usize) -> Result<ResourceVector, ProgramFailure> {
    let bytes_read = u64::try_from(bytes_read)
        .map_err(|_| ProgramFailure::invariant("Git stdout length cannot be represented as u64"))?;
    Ok(ResourceVector {
        usd: 0.0,
        wall_time_ms: 0,
        cpu_time_ms: None,
        peak_memory_bytes: None,
        bytes_read,
        bytes_written: 0,
        programs: 0,
    })
}

fn git_evidence(output: &GitCommandOutput, git_version: &str, description: &str) -> EvidenceItem {
    EvidenceItem {
        kind: "git-command-stdout".to_owned(),
        locator: output.command.clone(),
        digest: Some(sha256_hex(&output.stdout)),
        description: format!("{description}; executable reported {git_version}"),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

#[derive(Debug)]
struct TreeEntry {
    path: Vec<u8>,
    size: u64,
    mode: Vec<u8>,
    oid: Vec<u8>,
}

fn parse_ls_tree(bytes: &[u8], command: &str) -> Result<Vec<TreeEntry>, RepoError> {
    let mut entries = Vec::new();
    for record in bytes.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        let (header, path) = split_once_byte(record, b'\t')
            .ok_or_else(|| git_parse(command, "tree entry has no tab-delimited path"))?;
        if path.is_empty() {
            return Err(git_parse(command, "tree entry has an empty path"));
        }
        let fields = header
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty())
            .collect::<Vec<_>>();
        if fields.len() != 4 {
            return Err(git_parse(
                command,
                "tree entry header does not contain mode, type, object id, and size",
            ));
        }
        if fields[0].is_empty() || !fields[0].iter().all(|byte| matches!(byte, b'0'..=b'7')) {
            return Err(git_parse(command, "tree entry has an invalid mode"));
        }
        if !matches!(fields[2].len(), 40 | 64) || !fields[2].iter().all(u8::is_ascii_hexdigit) {
            return Err(git_parse(command, "tree entry has an invalid object id"));
        }

        match fields[1] {
            b"blob" => {
                let size = parse_u64_ascii(fields[3])
                    .ok_or_else(|| git_parse(command, "blob entry has an invalid size"))?;
                entries.push(TreeEntry {
                    path: path.to_vec(),
                    size,
                    mode: fields[0].to_vec(),
                    oid: fields[2].to_vec(),
                });
            }
            b"commit" | b"tree" => {
                if fields[3] != b"-" && parse_u64_ascii(fields[3]).is_none() {
                    return Err(git_parse(command, "non-blob entry has an invalid size"));
                }
            }
            _ => return Err(git_parse(command, "tree entry has an unknown object type")),
        }
    }

    Ok(entries)
}

fn build_static_shape(
    entries: Vec<TreeEntry>,
    limitations: Vec<String>,
) -> Result<StaticRepoShape, ProgramFailure> {
    let mut category_files = zero_category_map();
    let mut category_bytes = zero_category_map();
    let mut tracked_files = 0_u64;
    let mut tracked_bytes = 0_u64;
    let mut max_path_depth = 0_u64;
    let mut source_entries = Vec::new();
    let mut source_component_bytes: BTreeMap<Vec<u8>, u64> = BTreeMap::new();

    for entry in entries {
        tracked_files = checked_add_u64(tracked_files, 1, "tracked file count")?;
        tracked_bytes = checked_add_u64(tracked_bytes, entry.size, "tracked byte count")?;
        let depth = path_depth(&entry.path)?;
        max_path_depth = max_path_depth.max(depth);

        let category = classify_path(&entry.path);
        increment_map(&mut category_files, category, 1, "category file count")?;
        increment_map(
            &mut category_bytes,
            category,
            entry.size,
            "category byte count",
        )?;
        if category == CATEGORY_SOURCE {
            let component = top_level_component(&entry.path);
            increment_bytes_map(
                &mut source_component_bytes,
                component,
                entry.size,
                "top-level source byte count",
            )?;
            source_entries.push(entry);
        }
    }

    source_entries.sort_by(|left, right| {
        left.size
            .cmp(&right.size)
            .then_with(|| left.path.cmp(&right.path))
    });
    let source_sizes = source_entries
        .iter()
        .map(|entry| entry.size)
        .collect::<Vec<_>>();
    let source_bytes = map_value(&category_bytes, CATEGORY_SOURCE)?;
    let test_bytes = map_value(&category_bytes, CATEGORY_TEST)?;
    let documentation_bytes = map_value(&category_bytes, CATEGORY_DOCUMENTATION)?;
    let generated_bytes = map_value(&category_bytes, CATEGORY_GENERATED)?;

    let source_file_size_median = nearest_rank(&source_sizes, 1, 2)?;
    let source_file_size_p90 = nearest_rank(&source_sizes, 9, 10)?;
    let (largest_source_file_bytes, largest_source_file_path) = match source_entries.last() {
        Some(largest) => {
            let largest_size = largest.size;
            let chosen = source_entries
                .iter()
                .filter(|entry| entry.size == largest_size)
                .map(|entry| entry.path.as_slice())
                .min();
            (largest_size, chosen.map(display_git_path))
        }
        None => (0, None),
    };

    let largest_source_file_share = optional_ratio(largest_source_file_bytes, source_bytes)?;
    let top_decile_source_byte_share = top_decile_share(&source_sizes)?;
    let (normalized_source_size_entropy, effective_source_files) =
        distribution_metrics(&source_sizes)?;
    let component_weights = source_component_bytes.values().copied().collect::<Vec<_>>();
    let (normalized_top_level_source_entropy, effective_top_level_components) =
        distribution_metrics(&component_weights)?;

    Ok(StaticRepoShape {
        classifier_version: CLASSIFIER_VERSION.to_owned(),
        tracked_files,
        tracked_bytes,
        category_files,
        category_bytes,
        source_file_size_median,
        source_file_size_p90,
        largest_source_file_bytes,
        largest_source_file_path,
        largest_source_file_share,
        top_decile_source_byte_share,
        normalized_source_size_entropy,
        effective_source_files,
        normalized_top_level_source_entropy,
        effective_top_level_components,
        test_to_source_bytes: optional_ratio(test_bytes, source_bytes)?,
        documentation_to_source_bytes: optional_ratio(documentation_bytes, source_bytes)?,
        generated_or_vendor_byte_share: ratio_or_zero(generated_bytes, tracked_bytes)?,
        max_path_depth,
        limitations,
    })
}

#[derive(Debug)]
pub(crate) struct ParsedCommit {
    pub(crate) committer_unix_seconds: i64,
    pub(crate) files: BTreeMap<Vec<u8>, CommitFile>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CommitFile {
    pub(crate) line_mass: u64,
    pub(crate) text: bool,
    pub(crate) binary: bool,
}

fn parse_git_log(bytes: &[u8], command: &str) -> Result<Vec<ParsedCommit>, RepoError> {
    if bytes.is_empty() {
        return Err(git_parse(command, "stdout was empty"));
    }

    let mut sections = bytes.split(|byte| *byte == 0x1e);
    if let Some(prefix) = sections.next()
        && prefix.iter().any(|byte| !byte.is_ascii_whitespace())
    {
        return Err(git_parse(
            command,
            "unexpected data before first commit separator",
        ));
    }

    let mut commits = Vec::new();
    for section in sections {
        if section.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let (hash, remainder) = split_once_byte(section, 0)
            .ok_or_else(|| git_parse(command, "commit header has no hash terminator"))?;
        if !matches!(hash.len(), 40 | 64) || !hash.iter().all(u8::is_ascii_hexdigit) {
            return Err(git_parse(
                command,
                "commit section has an invalid object id",
            ));
        }
        let (timestamp, numstat) = split_once_byte(remainder, 0)
            .ok_or_else(|| git_parse(command, "commit header has no timestamp terminator"))?;
        let timestamp = std::str::from_utf8(timestamp)
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .ok_or_else(|| {
                git_parse(command, "commit section has an invalid committer timestamp")
            })?;
        let numstat = numstat.strip_prefix(b"\n").unwrap_or(numstat);
        let lines = numstat.split(|byte| *byte == b'\n');
        let mut files: BTreeMap<Vec<u8>, CommitFile> = BTreeMap::new();
        for raw_line in lines {
            let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
            if line.is_empty() {
                continue;
            }
            let mut fields = line.splitn(3, |byte| *byte == b'\t');
            let additions = fields.next().unwrap_or_default();
            let deletions = fields.next().ok_or_else(|| {
                git_parse(
                    command,
                    "numstat row has fewer than three tab-delimited fields",
                )
            })?;
            let encoded_path = fields.next().ok_or_else(|| {
                git_parse(
                    command,
                    "numstat row has fewer than three tab-delimited fields",
                )
            })?;
            if encoded_path.is_empty() {
                return Err(git_parse(command, "numstat row has an empty path"));
            }
            let path = decode_git_quoted_path(encoded_path)
                .map_err(|reason| git_parse(command, &format!("invalid numstat path: {reason}")))?;

            let (line_mass, binary) = if additions == b"-" || deletions == b"-" {
                if additions != b"-" || deletions != b"-" {
                    return Err(git_parse(
                        command,
                        "binary numstat row must use '-' for both additions and deletions",
                    ));
                }
                (0, true)
            } else {
                let additions = parse_u64_ascii(additions)
                    .ok_or_else(|| git_parse(command, "invalid numstat additions"))?;
                let deletions = parse_u64_ascii(deletions)
                    .ok_or_else(|| git_parse(command, "invalid numstat deletions"))?;
                let line_mass = additions.checked_add(deletions).ok_or_else(|| {
                    git_parse(command, "numstat additions plus deletions overflowed u64")
                })?;
                (line_mass, false)
            };

            if let Some(existing) = files.get_mut(&path) {
                existing.line_mass = existing
                    .line_mass
                    .checked_add(line_mass)
                    .ok_or_else(|| git_parse(command, "duplicate path line mass overflowed u64"))?;
                existing.binary |= binary;
                existing.text |= !binary;
            } else {
                files.insert(
                    path,
                    CommitFile {
                        line_mass,
                        text: !binary,
                        binary,
                    },
                );
            }
        }
        commits.push(ParsedCommit {
            committer_unix_seconds: timestamp,
            files,
        });
    }

    if commits.is_empty() {
        return Err(git_parse(command, "zero commits were parsed"));
    }
    Ok(commits)
}

fn build_change_shape(
    requested_commits: usize,
    commits: Vec<ParsedCommit>,
    limitations: Vec<String>,
) -> Result<GitChangeShape, ProgramFailure> {
    let commits_analyzed = usize_to_u64(commits.len(), "commit count")?;
    let mut commits_with_binary_changes = 0_u64;
    let mut all_changed_files = BTreeSet::new();
    let mut total_line_change_mass = 0_u64;
    let mut global_mass: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
    let mut changed_counts = Vec::with_capacity(commits.len());
    let mut total_pairs = 0_u128;
    let mut cross_pairs = 0_u128;
    let mut broad_commits = 0_u64;
    let mut source_commits = 0_u64;
    let mut source_test_commits = 0_u64;
    let mut source_documentation_commits = 0_u64;

    for commit in commits {
        let changed_count = usize_to_u64(commit.files.len(), "files changed in commit")?;
        changed_counts.push(changed_count);
        let mut component_counts: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
        let mut has_binary = false;
        let mut has_source = false;
        let mut has_test = false;
        let mut has_documentation = false;

        for (path, file) in commit.files {
            all_changed_files.insert(path.clone());
            increment_bytes_map(
                &mut global_mass,
                path.clone(),
                file.line_mass,
                "per-file change mass",
            )?;
            total_line_change_mass = checked_add_u64(
                total_line_change_mass,
                file.line_mass,
                "total line change mass",
            )?;
            has_binary |= file.binary;
            match classify_path(&path) {
                CATEGORY_SOURCE => has_source = true,
                CATEGORY_TEST => has_test = true,
                CATEGORY_DOCUMENTATION => has_documentation = true,
                _ => {}
            }
            increment_bytes_map(
                &mut component_counts,
                top_level_component(&path),
                1,
                "top-level component path count",
            )?;
        }

        if has_binary {
            commits_with_binary_changes = checked_add_u64(
                commits_with_binary_changes,
                1,
                "commits with binary changes",
            )?;
        }
        if component_counts.len() >= 3 {
            broad_commits = checked_add_u64(broad_commits, 1, "broad commit count")?;
        }
        if has_source {
            source_commits = checked_add_u64(source_commits, 1, "source commit count")?;
            if has_test {
                source_test_commits =
                    checked_add_u64(source_test_commits, 1, "source-test commit count")?;
            }
            if has_documentation {
                source_documentation_commits = checked_add_u64(
                    source_documentation_commits,
                    1,
                    "source-documentation commit count",
                )?;
            }
        }

        let commit_pairs = choose_two_u128(changed_count);
        let mut same_pairs = 0_u128;
        for count in component_counts.values() {
            same_pairs = same_pairs
                .checked_add(choose_two_u128(*count))
                .ok_or_else(|| {
                    ProgramFailure::invariant("same-component cochange pair count overflowed u128")
                })?;
        }
        let commit_cross = commit_pairs.checked_sub(same_pairs).ok_or_else(|| {
            ProgramFailure::invariant("same-component pairs exceeded total cochange pairs")
        })?;
        total_pairs = total_pairs.checked_add(commit_pairs).ok_or_else(|| {
            ProgramFailure::invariant("total cochange pair count overflowed u128")
        })?;
        cross_pairs = cross_pairs.checked_add(commit_cross).ok_or_else(|| {
            ProgramFailure::invariant("cross-component pair count overflowed u128")
        })?;
    }

    changed_counts.sort_unstable();
    let files_changed_total = changed_counts.iter().try_fold(0_u64, |total, count| {
        total.checked_add(*count).ok_or_else(|| {
            ProgramFailure::invariant("aggregate files-changed count overflowed u64")
        })
    })?;
    let files_changed_mean = checked_f64_ratio(
        files_changed_total as f64,
        commits_analyzed as f64,
        "files changed mean",
    )?;
    let files_changed_median = nearest_rank(&changed_counts, 1, 2)?.ok_or_else(|| {
        ProgramFailure::invariant("parsed commit collection unexpectedly became empty")
    })?;
    let files_changed_p90 = nearest_rank(&changed_counts, 9, 10)?.ok_or_else(|| {
        ProgramFailure::invariant("parsed commit collection unexpectedly became empty")
    })?;
    let files_changed_max = changed_counts.last().copied().ok_or_else(|| {
        ProgramFailure::invariant("parsed commit collection unexpectedly became empty")
    })?;

    let positive_hotspots = global_mass
        .iter()
        .filter(|(_, mass)| **mass > 0)
        .collect::<Vec<_>>();
    let positive_masses = positive_hotspots
        .iter()
        .map(|(_, mass)| **mass)
        .collect::<Vec<_>>();
    let (normalized_change_mass_entropy, effective_hotspot_files) =
        distribution_metrics(&positive_masses)?;
    let top_decile_change_mass_share = top_decile_share(&positive_masses)?;
    let largest_hotspot_mass = positive_hotspots
        .iter()
        .map(|(_, mass)| **mass)
        .max()
        .unwrap_or(0);
    let largest_hotspot_path = if largest_hotspot_mass == 0 {
        None
    } else {
        positive_hotspots
            .iter()
            .filter(|(_, mass)| **mass == largest_hotspot_mass)
            .map(|(path, _)| path.as_slice())
            .min()
            .map(display_git_path)
    };

    Ok(GitChangeShape {
        requested_commits,
        commits_analyzed,
        commits_with_binary_changes,
        unique_changed_files: usize_to_u64(all_changed_files.len(), "unique changed files")?,
        total_line_change_mass,
        files_changed_mean,
        files_changed_median,
        files_changed_p90,
        files_changed_max,
        normalized_change_mass_entropy,
        effective_hotspot_files,
        top_decile_change_mass_share,
        largest_hotspot_path,
        largest_hotspot_mass,
        cross_top_level_pair_ratio: optional_u128_ratio(cross_pairs, total_pairs)?,
        broad_commit_rate: checked_unit_ratio(
            broad_commits as f64,
            commits_analyzed as f64,
            "broad commit rate",
        )?,
        source_commits,
        source_test_cochange_rate: optional_ratio(source_test_commits, source_commits)?,
        source_documentation_cochange_rate: optional_ratio(
            source_documentation_commits,
            source_commits,
        )?,
        limitations,
    })
}

fn classify_path(path: &[u8]) -> &'static str {
    let lower = path.iter().map(u8::to_ascii_lowercase).collect::<Vec<_>>();
    let segments = lower.split(|byte| *byte == b'/').collect::<Vec<_>>();
    let filename = segments.last().copied().unwrap_or_default();
    let original_filename = path.rsplit(|byte| *byte == b'/').next().unwrap_or_default();

    if segments.iter().any(is_generated_segment) || is_generated_filename(filename) {
        CATEGORY_GENERATED
    } else if segments.iter().any(is_test_segment) || is_test_filename(filename, original_filename)
    {
        CATEGORY_TEST
    } else if segments.iter().any(is_documentation_segment) || is_documentation_filename(filename) {
        CATEGORY_DOCUMENTATION
    } else if is_configuration_filename(filename) {
        CATEGORY_CONFIGURATION
    } else if extension(filename).is_some_and(is_source_extension) {
        CATEGORY_SOURCE
    } else {
        CATEGORY_OTHER
    }
}

fn is_generated_segment(segment: &&[u8]) -> bool {
    matches!(
        *segment,
        b"vendor"
            | b"vendors"
            | b"third_party"
            | b"third-party"
            | b"node_modules"
            | b"generated"
            | b"gen"
            | b"build"
            | b"dist"
            | b"target"
            | b"out"
            | b".next"
    )
}

fn is_generated_filename(filename: &[u8]) -> bool {
    filename.ends_with(b".min.js")
        || filename.ends_with(b".min.css")
        || filename
            .windows(b".generated.".len())
            .any(|window| window == b".generated.")
        || filename
            .windows(b"_generated.".len())
            .any(|window| window == b"_generated.")
        || filename
            .windows(b".g.".len())
            .any(|window| window == b".g.")
        || filename
            .windows(b".pb.".len())
            .any(|window| window == b".pb.")
}

fn is_test_segment(segment: &&[u8]) -> bool {
    matches!(
        *segment,
        b"test" | b"tests" | b"testing" | b"spec" | b"specs" | b"__tests__"
    )
}

fn is_test_filename(filename: &[u8], original_filename: &[u8]) -> bool {
    let stem = stem_before_extension(filename);
    let original_stem = stem_before_extension(original_filename);
    stem.starts_with(b"test_")
        || matches!(stem, b"test" | b"tests" | b"spec" | b"specs")
        || stem.ends_with(b"_test")
        || stem.ends_with(b"_tests")
        || stem.ends_with(b"-test")
        || stem.ends_with(b".test")
        || stem.ends_with(b"_spec")
        || stem.ends_with(b"-spec")
        || stem.ends_with(b".spec")
        || original_stem.ends_with(b"Test")
        || original_stem.ends_with(b"Tests")
        || original_stem.ends_with(b"Spec")
}

fn is_documentation_segment(segment: &&[u8]) -> bool {
    matches!(*segment, b"doc" | b"docs" | b"documentation")
}

fn is_documentation_filename(filename: &[u8]) -> bool {
    matches!(
        extension(filename),
        Some(b"md" | b"markdown" | b"rst" | b"adoc" | b"asciidoc")
    ) || matches!(
        filename,
        b"readme"
            | b"changelog"
            | b"changes"
            | b"contributing"
            | b"authors"
            | b"license"
            | b"licence"
    ) || filename.starts_with(b"readme.")
        || filename.starts_with(b"changelog.")
        || filename.starts_with(b"contributing.")
        || filename.starts_with(b"license.")
        || filename.starts_with(b"licence.")
}

fn is_configuration_filename(filename: &[u8]) -> bool {
    matches!(
        extension(filename),
        Some(
            b"json"
                | b"yaml"
                | b"yml"
                | b"toml"
                | b"xml"
                | b"ini"
                | b"cfg"
                | b"conf"
                | b"properties"
                | b"lock"
                | b"gradle"
        )
    ) || matches!(
        filename,
        b"makefile"
            | b"gnumakefile"
            | b"dockerfile"
            | b"cargo.lock"
            | b"go.mod"
            | b"go.sum"
            | b"package-lock.json"
            | b"npm-shrinkwrap.json"
            | b"yarn.lock"
            | b"pnpm-lock.yaml"
            | b"gemfile.lock"
            | b"podfile.lock"
    ) || filename.starts_with(b".")
        && matches!(
            filename,
            b".gitignore"
                | b".gitattributes"
                | b".editorconfig"
                | b".npmrc"
                | b".prettierrc"
                | b".eslintrc"
                | b".rubocop.yml"
        )
}

fn is_source_extension(extension: &[u8]) -> bool {
    matches!(
        extension,
        b"rs"
            | b"go"
            | b"py"
            | b"pyw"
            | b"js"
            | b"jsx"
            | b"mjs"
            | b"cjs"
            | b"ts"
            | b"tsx"
            | b"java"
            | b"c"
            | b"h"
            | b"cc"
            | b"cpp"
            | b"cxx"
            | b"hh"
            | b"hpp"
            | b"hxx"
            | b"cs"
            | b"rb"
            | b"swift"
            | b"kt"
            | b"kts"
            | b"sh"
            | b"bash"
            | b"zsh"
            | b"fish"
    )
}

fn extension(filename: &[u8]) -> Option<&[u8]> {
    let index = filename.iter().rposition(|byte| *byte == b'.')?;
    if index == 0 || index + 1 == filename.len() {
        None
    } else {
        filename.get(index + 1..)
    }
}

fn stem_before_extension(filename: &[u8]) -> &[u8] {
    filename
        .iter()
        .rposition(|byte| *byte == b'.')
        .and_then(|index| filename.get(..index))
        .unwrap_or(filename)
}

fn top_level_component(path: &[u8]) -> Vec<u8> {
    match path.iter().position(|byte| *byte == b'/') {
        Some(index) => path[..index].to_vec(),
        None => b".".to_vec(),
    }
}

fn path_depth(path: &[u8]) -> Result<u64, ProgramFailure> {
    let slash_count = path.iter().filter(|byte| **byte == b'/').count();
    usize_to_u64(slash_count, "path depth")?
        .checked_add(1)
        .ok_or_else(|| ProgramFailure::invariant("path depth overflowed u64"))
}

fn display_git_path(path: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(path) {
        return text.to_owned();
    }
    let mut output = String::new();
    for byte in path {
        if byte.is_ascii_graphic() && *byte != b'\\' {
            output.push(char::from(*byte));
        } else if *byte == b'\\' {
            output.push_str("\\\\");
        } else {
            use std::fmt::Write as _;
            let _ = write!(&mut output, "\\x{byte:02x}");
        }
    }
    output
}

fn decode_git_quoted_path(path: &[u8]) -> Result<Vec<u8>, String> {
    if !path.starts_with(b"\"") {
        return Ok(path.to_vec());
    }
    if path.len() < 2 || !path.ends_with(b"\"") {
        return Err("opening quote has no closing quote".to_owned());
    }
    let body = &path[1..path.len() - 1];
    let mut decoded = Vec::with_capacity(body.len());
    let mut index = 0;
    while index < body.len() {
        let byte = body[index];
        if byte != b'\\' {
            decoded.push(byte);
            index += 1;
            continue;
        }
        index += 1;
        let escaped = *body
            .get(index)
            .ok_or_else(|| "path ends in an incomplete escape".to_owned())?;
        match escaped {
            b'\\' | b'"' => {
                decoded.push(escaped);
                index += 1;
            }
            b'a' => {
                decoded.push(0x07);
                index += 1;
            }
            b'b' => {
                decoded.push(0x08);
                index += 1;
            }
            b't' => {
                decoded.push(b'\t');
                index += 1;
            }
            b'n' => {
                decoded.push(b'\n');
                index += 1;
            }
            b'v' => {
                decoded.push(0x0b);
                index += 1;
            }
            b'f' => {
                decoded.push(0x0c);
                index += 1;
            }
            b'r' => {
                decoded.push(b'\r');
                index += 1;
            }
            b'0'..=b'7' => {
                let mut value = u16::from(escaped - b'0');
                let mut digits = 1;
                while digits < 3 {
                    match body.get(index + digits) {
                        Some(next @ b'0'..=b'7') => {
                            value = value * 8 + u16::from(*next - b'0');
                            digits += 1;
                        }
                        _ => break,
                    }
                }
                if value > u16::from(u8::MAX) {
                    return Err("octal escape exceeds one byte".to_owned());
                }
                decoded.push(value as u8);
                index += digits;
            }
            _ => return Err(format!("unsupported escape \\{}", char::from(escaped))),
        }
    }
    if decoded.is_empty() {
        Err("decoded path is empty".to_owned())
    } else {
        Ok(decoded)
    }
}

fn nearest_rank(
    sorted: &[u64],
    numerator: usize,
    denominator: usize,
) -> Result<Option<u64>, ProgramFailure> {
    if sorted.is_empty() {
        return Ok(None);
    }
    if numerator == 0 || denominator == 0 || numerator > denominator {
        return Err(ProgramFailure::invariant("invalid nearest-rank quantile"));
    }
    let product = sorted
        .len()
        .checked_mul(numerator)
        .ok_or_else(|| ProgramFailure::invariant("nearest-rank numerator overflowed usize"))?;
    let rank = product
        .checked_add(denominator - 1)
        .ok_or_else(|| ProgramFailure::invariant("nearest-rank rounding overflowed usize"))?
        / denominator;
    sorted
        .get(rank - 1)
        .copied()
        .map(Some)
        .ok_or_else(|| ProgramFailure::invariant("nearest-rank index was unavailable"))
}

fn distribution_metrics(weights: &[u64]) -> Result<(Option<f64>, Option<f64>), ProgramFailure> {
    if weights.is_empty() {
        return Ok((None, None));
    }
    if weights.len() == 1 {
        return Ok((Some(0.0), Some(1.0)));
    }
    let total = weights.iter().try_fold(0_u64, |sum, value| {
        sum.checked_add(*value)
            .ok_or_else(|| ProgramFailure::invariant("distribution mass overflowed u64"))
    })?;
    if total == 0 {
        return Ok((None, None));
    }

    let total_f64 = total as f64;
    let mut entropy = 0.0;
    for weight in weights {
        if *weight == 0 {
            continue;
        }
        let probability = checked_unit_ratio(*weight as f64, total_f64, "entropy probability")?;
        entropy -= probability * probability.ln();
    }
    if !entropy.is_finite() || entropy < 0.0 {
        return Err(ProgramFailure::invariant(
            "Shannon entropy was nonfinite or negative",
        ));
    }
    let normalized = checked_unit_value(
        entropy / (weights.len() as f64).ln(),
        "normalized Shannon entropy",
    )?;
    let effective = entropy.exp();
    if !effective.is_finite() {
        return Err(ProgramFailure::invariant("effective count was nonfinite"));
    }
    Ok((Some(normalized), Some(effective)))
}

fn top_decile_share(weights: &[u64]) -> Result<Option<f64>, ProgramFailure> {
    if weights.is_empty() {
        return Ok(None);
    }
    let total = weights.iter().try_fold(0_u64, |sum, value| {
        sum.checked_add(*value)
            .ok_or_else(|| ProgramFailure::invariant("top-decile total overflowed u64"))
    })?;
    if total == 0 {
        return Ok(None);
    }
    let count = weights
        .len()
        .checked_add(9)
        .ok_or_else(|| ProgramFailure::invariant("top-decile count overflowed usize"))?
        / 10;
    let mut sorted = weights.to_vec();
    sorted.sort_unstable_by(|left, right| right.cmp(left));
    let mass = sorted.iter().take(count).try_fold(0_u64, |sum, value| {
        sum.checked_add(*value)
            .ok_or_else(|| ProgramFailure::invariant("top-decile mass overflowed u64"))
    })?;
    Ok(Some(checked_unit_ratio(
        mass as f64,
        total as f64,
        "top-decile share",
    )?))
}

fn optional_ratio(numerator: u64, denominator: u64) -> Result<Option<f64>, ProgramFailure> {
    if denominator == 0 {
        Ok(None)
    } else {
        checked_f64_ratio(numerator as f64, denominator as f64, "optional ratio").map(Some)
    }
}

fn ratio_or_zero(numerator: u64, denominator: u64) -> Result<f64, ProgramFailure> {
    if denominator == 0 {
        Ok(0.0)
    } else {
        checked_unit_ratio(numerator as f64, denominator as f64, "byte share")
    }
}

fn optional_u128_ratio(numerator: u128, denominator: u128) -> Result<Option<f64>, ProgramFailure> {
    if denominator == 0 {
        Ok(None)
    } else {
        checked_unit_ratio(numerator as f64, denominator as f64, "cochange pair ratio").map(Some)
    }
}

fn checked_f64_ratio(numerator: f64, denominator: f64, name: &str) -> Result<f64, ProgramFailure> {
    if !numerator.is_finite() || !denominator.is_finite() || denominator <= 0.0 {
        return Err(ProgramFailure::invariant(format!(
            "{name} has a nonfinite numerator or nonpositive denominator"
        )));
    }
    let value = numerator / denominator;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(ProgramFailure::invariant(format!("{name} was nonfinite")))
    }
}

fn checked_unit_ratio(numerator: f64, denominator: f64, name: &str) -> Result<f64, ProgramFailure> {
    let value = checked_f64_ratio(numerator, denominator, name)?;
    checked_unit_value(value, name)
}

fn checked_unit_value(value: f64, name: &str) -> Result<f64, ProgramFailure> {
    const TOLERANCE: f64 = 1.0e-12;
    if !value.is_finite() || !(-TOLERANCE..=1.0 + TOLERANCE).contains(&value) {
        return Err(ProgramFailure::invariant(format!(
            "{name} was outside the finite [0, 1] interval"
        )));
    }
    Ok(value.clamp(0.0, 1.0))
}

fn choose_two_u128(count: u64) -> u128 {
    let count = u128::from(count);
    count * count.saturating_sub(1) / 2
}

fn zero_category_map() -> BTreeMap<String, u64> {
    ALL_CATEGORIES
        .iter()
        .map(|category| ((*category).to_owned(), 0))
        .collect()
}

fn increment_map(
    map: &mut BTreeMap<String, u64>,
    key: &str,
    increment: u64,
    name: &str,
) -> Result<(), ProgramFailure> {
    let value = map.get_mut(key).ok_or_else(|| {
        ProgramFailure::invariant(format!("classifier produced unknown category {key:?}"))
    })?;
    *value = checked_add_u64(*value, increment, name)?;
    Ok(())
}

fn increment_bytes_map(
    map: &mut BTreeMap<Vec<u8>, u64>,
    key: Vec<u8>,
    increment: u64,
    name: &str,
) -> Result<(), ProgramFailure> {
    let previous = map.get(&key).copied().unwrap_or(0);
    map.insert(key, checked_add_u64(previous, increment, name)?);
    Ok(())
}

fn map_value(map: &BTreeMap<String, u64>, key: &str) -> Result<u64, ProgramFailure> {
    map.get(key)
        .copied()
        .ok_or_else(|| ProgramFailure::invariant(format!("category {key:?} was absent")))
}

fn checked_add_u64(left: u64, right: u64, name: &str) -> Result<u64, ProgramFailure> {
    left.checked_add(right)
        .ok_or_else(|| ProgramFailure::invariant(format!("{name} overflowed u64")))
}

fn usize_to_u64(value: usize, name: &str) -> Result<u64, ProgramFailure> {
    u64::try_from(value)
        .map_err(|_| ProgramFailure::invariant(format!("{name} cannot be represented as u64")))
}

fn parse_u64_ascii(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    text.parse().ok()
}

fn split_once_byte(bytes: &[u8], delimiter: u8) -> Option<(&[u8], &[u8])> {
    let index = bytes.iter().position(|byte| *byte == delimiter)?;
    Some((bytes.get(..index)?, bytes.get(index + 1..)?))
}

fn git_parse(command: &str, reason: &str) -> RepoError {
    RepoError::GitParse {
        command: command.to_owned(),
        reason: reason.to_owned(),
    }
}

fn static_limitations(git_version: &str) -> Vec<String> {
    vec![
        "Byte and path structure is a proxy for repository shape, not an overall quality measure.".to_owned(),
        "The versioned lexical classifier can misclassify unusual layouts, languages, generated files, documentation, configuration, and tests.".to_owned(),
        "Blob sizes and paths do not measure correctness, complexity, maintainability, security, or user value.".to_owned(),
        "Symlink entries are measured as committed blobs; worktree symlink targets are never read or followed.".to_owned(),
        "Tracked file and byte totals count blob entries; Git submodule/gitlink entries are excluded.".to_owned(),
        "Nearest-rank quantiles sort integer byte sizes and use rank ceil(p*N), with the first rank numbered one.".to_owned(),
        format!("Git is an external dependency ({git_version}); deterministic replay is scoped to the commit, classifier version, command, and Git behavior, not claimed byte-for-byte across Git versions."),
    ]
}

fn change_limitations(git_version: &str, requested_commits: usize) -> Vec<String> {
    vec![
        format!("History is truncated to at most {requested_commits} non-merge commits, so older change patterns are not represented."),
        "Merge commits are excluded; squash merges, rebases, and other history rewriting can substantially change the observed topology.".to_owned(),
        "Rename detection is disabled, so renames and file-identity continuity are not recovered; identity and attribution across paths have gaps.".to_owned(),
        "Binary changes are counted by commit and path but excluded from line-change mass because Git numstat supplies no line counts.".to_owned(),
        "Files-changed quantiles use nearest-rank over sorted integer counts: rank ceil(p*N), with the first rank numbered one.".to_owned(),
        "Top-level layout boundaries are a leakage proxy only; cochange across them is not proof of modularity, coupling, or architecture quality.".to_owned(),
        "The versioned lexical classifier and path-based source/test/documentation interpretation can misclassify unusual repository layouts.".to_owned(),
        "Commit topology and line counts do not measure correctness, complexity, maintainability, security, or user value.".to_owned(),
        format!("Git is an external dependency ({git_version}); deterministic replay is scoped to the commit, requested history, classifier, command, and Git behavior, not claimed byte-for-byte across Git versions."),
    ]
}

#[cfg(test)]
mod tests {
    use super::{parse_git_log, parse_ls_tree};

    #[test]
    fn duplicate_text_and_binary_rows_retain_both_observations_once_per_commit() {
        let mut stdout = Vec::new();
        stdout.extend_from_slice(b"\x1eaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\0978397200\0\n");
        stdout.extend_from_slice(b"2\t3\tsrc/mixed.rs\n");
        stdout.extend_from_slice(b"-\t-\tsrc/mixed.rs\n");

        let commits = parse_git_log(&stdout, "synthetic git log")
            .expect("synthetic timestamp-aware numstat must parse");
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].committer_unix_seconds, 978_397_200);
        // One map entry is the independent per-commit dedup oracle: downstream
        // commits_touched iterates this map and therefore counts this path once.
        assert_eq!(commits[0].files.len(), 1);
        let observation = commits[0]
            .files
            .get(b"src/mixed.rs".as_slice())
            .expect("raw path identity retained");
        assert_eq!(observation.line_mass, 5, "2 additions + 3 deletions");
        assert!(
            observation.text,
            "textual mass observation must survive binary row"
        );
        assert!(
            observation.binary,
            "binary observation must survive textual row"
        );
    }

    #[test]
    fn tree_parser_preserves_non_utf8_path_bytes_for_explicit_filtering() {
        let raw_path = b"src/raw-\xff.rs";
        let mut stdout = b"100644 blob aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 17\t".to_vec();
        stdout.extend_from_slice(raw_path);
        stdout.push(0);

        let entries = parse_ls_tree(&stdout, "synthetic git ls-tree")
            .expect("raw-byte tree record must parse");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, raw_path);
        assert_eq!(entries[0].size, 17);
        assert_eq!(entries[0].mode, b"100644");
        assert!(
            std::str::from_utf8(&entries[0].path).is_err(),
            "non-UTF8 Git path is excluded from current metrics even when its blob contents are ASCII"
        );
    }
}
