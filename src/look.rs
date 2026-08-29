//! `seval look` — the one front door for "where should a reader look first".
//!
//! Sloppy code is code that was written and never re-read. Every lens here
//! measures evidence of that, on the repository's own distribution, and the
//! report ranks by *agreement* (how many lenses cite the same place), never by
//! a weighted composite. The lenses are the instruments the rest of the crate
//! already computes; this module only composes and orders them.
//!
//! With `--base REV` the same report runs against a detached worktree of
//! `REV` and the output names what the working tree *introduced*: the ratchet
//! form a writer can steer by mid-change.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::liveness::{LivenessError, NameStatus, analyze_liveness};
use crate::shape::{FunctionShape, ShapeError, analyze_shape};
use crate::source::SourceCorpusSession;
use crate::tests_analysis::{FileRole, classify_file};

#[derive(Debug, thiserror::Error)]
pub enum LookError {
    #[error(transparent)]
    Shape(#[from] ShapeError),
    #[error(transparent)]
    Liveness(#[from] LivenessError),
    #[error(transparent)]
    Source(#[from] crate::source::SourceError),
    #[error("git {argv} failed: {stderr}")]
    Git { argv: String, stderr: String },
    #[error("could not run git: {0}")]
    GitInvocation(std::io::Error),
    #[error("could not create a temporary worktree: {0}")]
    TempDir(std::io::Error),
}

/// One thing to look at. `key` is stable across revisions — path, name, and
/// the name's ordinal within the file, never a line number — so base/head
/// rows can be joined.
#[derive(Debug, Clone, Serialize)]
pub struct LookRow {
    pub lens: Lens,
    pub key: String,
    pub path: String,
    pub line: usize,
    /// Human-readable evidence, e.g. `cognitive 165 cyclomatic 53 gap 112 nesting 5`.
    pub evidence: String,
    /// The lens's own ordering value; larger is worse. Comparable only within a lens.
    pub magnitude: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Lens {
    /// Cognitive − cyclomatic gap joined with nesting: the nested tangle a
    /// flat many-arm match never trips.
    Tangle,
    /// Branch asymmetry: one arm dwarfs its siblings, or a no-else `if`
    /// carries the whole happy path.
    Asymmetry,
    /// Lexically public definitions with no mention anywhere in the tree.
    DeadPublic,
    /// Files at or above the line ceiling.
    Bulk,
    /// Files whose longest line runs past every mainstream formatter's wrap.
    Squash,
    /// Files `rustfmt --check` would rewrite: written and never re-read.
    FormatDrift,
}

impl Lens {
    pub fn label(self) -> &'static str {
        match self {
            Lens::Tangle => "tangle",
            Lens::Asymmetry => "asymmetry",
            Lens::DeadPublic => "dead-public",
            Lens::Bulk => "bulk",
            Lens::Squash => "squash",
            Lens::FormatDrift => "format-drift",
        }
    }

    /// What the number means and how it fails, in one line.
    pub fn reading(self) -> &'static str {
        match self {
            Lens::Tangle => {
                "cognitive − cyclomatic > 0 with nesting; exonerates flat dispatch; misses tangles hidden in macros"
            }
            Lens::Asymmetry => {
                "largest arm / smallest sibling arm, and no-else ifs with ≥ 8-statement then-arms; a legitimate error-arm can fire it"
            }
            Lens::DeadPublic => {
                "name-level census; consumers outside the tree (downstream crates, FFI, reflection) are invisible"
            }
            Lens::Bulk => {
                "line count against the ceiling; generated and data files fire it legitimately"
            }
            Lens::Squash => {
                "longest line over 120 columns; minified vendored files and long string literals fire it legitimately"
            }
            Lens::FormatDrift => {
                "external rustfmt evidence, reaching only files cargo fmt visits; a deliberate rustfmt.toml policy is not drift"
            }
        }
    }
}

/// A place cited by more than one lens: the strongest signal this report has.
#[derive(Debug, Clone, Serialize)]
pub struct Agreement {
    pub path: String,
    pub lenses: Vec<Lens>,
    pub rows: Vec<LookRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LensSection {
    pub lens: Lens,
    pub reading: &'static str,
    /// Rows the lens considered at all (e.g. every function with a positive gap).
    pub candidates: usize,
    /// Nearest-rank p90 magnitude among candidates; rows at or above it are the tail.
    /// Binary lenses (dead-public, bulk, format-drift) have every candidate in the tail.
    pub tail_floor: f64,
    /// Tail rows found, before `top` truncation.
    pub found: usize,
    pub rows: Vec<LookRow>,
    /// Set when the lens could not run; the reason is the whole story.
    pub unavailable: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LookReport {
    pub root: String,
    pub analyzer: String,
    pub file_line_ceiling: usize,
    pub supported_files: usize,
    pub functions: usize,
    pub agreements: Vec<Agreement>,
    pub sections: Vec<LensSection>,
    pub limitations: Vec<String>,
}

/// Head tail rows that are absent at base or worse than at base, joined on
/// `LookRow::key` over every candidate (not just base's tail), so a function
/// that merely entered the tail because neighbours improved is not blamed.
#[derive(Debug, Clone, Serialize)]
pub struct LookDelta {
    pub base: String,
    pub head: LookReport,
    pub introduced: Vec<LookRow>,
    /// Tail-row count, head − base, per lens.
    pub found_delta: BTreeMap<Lens, i64>,
}

/// Full report: every row of every lens. `LookReport::truncated` bounds it.
pub fn look(input: &Path, file_line_ceiling: usize) -> Result<LookReport, LookError> {
    look_with_candidates(input, file_line_ceiling).map(|(report, _)| report)
}

/// Every candidate row per lens, keyed for base/head joins.
type Candidates = BTreeMap<Lens, BTreeMap<String, f64>>;

fn look_with_candidates(
    input: &Path,
    file_line_ceiling: usize,
) -> Result<(LookReport, Candidates), LookError> {
    let session = SourceCorpusSession::activate(input)?;
    session.scope(|| look_in_session(input, file_line_ceiling))
}

impl LookReport {
    pub fn truncated(mut self, top: usize) -> Self {
        for section in &mut self.sections {
            section.rows.truncate(top);
        }
        self
    }
}

fn look_in_session(
    input: &Path,
    file_line_ceiling: usize,
) -> Result<(LookReport, Candidates), LookError> {
    let shape = analyze_shape(input)?;
    let liveness = analyze_liveness(input)?;
    let tree = crate::source::load_source_tree(input)?;

    let test_paths: BTreeSet<&str> = tree
        .files
        .iter()
        .filter(|file| classify_file(file) == FileRole::Test)
        .map(|file| file.path.as_str())
        .collect();
    let non_test = |function: &&FunctionShape| !test_paths.contains(function.path.as_str());
    // `shape` rows are in (path, line) order, so the ordinal is source order.
    let mut seen: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    let keys: Vec<String> = shape
        .functions
        .iter()
        .map(|f| {
            let ordinal = seen.entry((f.path.as_str(), f.name.as_str())).or_insert(0);
            *ordinal += 1;
            format!("{}::{}#{ordinal}", f.path, f.name)
        })
        .collect();
    let function_rows = || {
        shape
            .functions
            .iter()
            .zip(&keys)
            .filter(|(f, _)| non_test(f))
    };

    let mut tangle: Vec<LookRow> = function_rows()
        .filter(|(f, _)| f.cognitive_gap > 0)
        .map(|(f, key)| LookRow {
            lens: Lens::Tangle,
            key: key.clone(),
            path: f.path.clone(),
            line: f.start_line,
            evidence: format!(
                "cognitive {} cyclomatic {} gap {} nesting {} lines {}",
                f.cognitive,
                f.cyclomatic,
                f.cognitive_gap,
                f.max_nesting_depth,
                f.end_line.saturating_sub(f.start_line) + 1
            ),
            magnitude: f.cognitive_gap as f64 + f64::from(f.max_nesting_depth) / 100.0,
        })
        .collect();

    let mut asymmetry: Vec<LookRow> = shape
        .functions
        .iter()
        .filter(non_test)
        .filter(|f| f.max_arm_size_ratio.is_some_and(|r| r >= 4.0) || f.no_else_large_then_arms > 0)
        .map(|f| LookRow {
            lens: Lens::Asymmetry,
            key: format!("{}::{}", f.path, f.name),
            path: f.path.clone(),
            line: f.start_line,
            evidence: format!(
                "arm ratio {} no-else large then-arms {} cognitive {}",
                f.max_arm_size_ratio
                    .map(|r| format!("{r:.1}"))
                    .unwrap_or_else(|| "n/a".to_owned()),
                f.no_else_large_then_arms,
                f.cognitive
            ),
            magnitude: f.max_arm_size_ratio.unwrap_or(0.0)
                + f64::from(f.no_else_large_then_arms) * 8.0,
        })
        .collect();

    let mut dead_public: Vec<LookRow> = liveness
        .rows
        .iter()
        .filter(|row| row.status == NameStatus::DeadPublic)
        .map(|row| {
            let site = &row.definitions[0];
            LookRow {
                lens: Lens::DeadPublic,
                key: format!("{}::{}", site.path, row.name),
                path: site.path.clone(),
                line: site.line,
                evidence: format!("{} {} has no mention in the tree", site.kind, row.name),
                magnitude: 1.0,
            }
        })
        .collect();

    let (line_counts, bulk_unavailable) = line_counts(input, &tree);
    let mut bulk: Vec<LookRow> = line_counts
        .iter()
        .filter(|(_, lines)| **lines >= file_line_ceiling)
        .map(|(path, lines)| LookRow {
            lens: Lens::Bulk,
            key: path.clone(),
            path: path.clone(),
            line: 1,
            evidence: format!("{lines} lines (ceiling {file_line_ceiling})"),
            magnitude: *lines as f64,
        })
        .collect();

    let mut squash: Vec<LookRow> = tree
        .files
        .iter()
        .filter_map(|file| {
            let (line, columns) = longest_line(&file.bytes);
            (columns > 120).then(|| LookRow {
                lens: Lens::Squash,
                key: file.path.clone(),
                path: file.path.clone(),
                line,
                evidence: format!("longest line {columns} columns"),
                magnitude: columns as f64,
            })
        })
        .collect();

    let (mut drift, drift_unavailable) = format_drift(input, &line_counts);

    for rows in [
        &mut tangle,
        &mut asymmetry,
        &mut dead_public,
        &mut bulk,
        &mut squash,
        &mut drift,
    ] {
        rows.sort_by(|a, b| {
            b.magnitude
                .total_cmp(&a.magnitude)
                .then_with(|| a.key.cmp(&b.key))
        });
    }

    let mut candidates: Candidates = BTreeMap::new();
    let mut section = |lens: Lens, rows: Vec<LookRow>, unavailable: Option<String>| {
        candidates.insert(
            lens,
            rows.iter().map(|r| (r.key.clone(), r.magnitude)).collect(),
        );
        let tail_floor = tail_floor(&rows);
        let tail: Vec<LookRow> = rows
            .iter()
            .filter(|r| r.magnitude >= tail_floor)
            .cloned()
            .collect();
        LensSection {
            lens,
            reading: lens.reading(),
            candidates: rows.len(),
            tail_floor,
            found: tail.len(),
            rows: tail,
            unavailable,
        }
    };
    let sections = vec![
        section(Lens::Tangle, tangle, None),
        section(Lens::Asymmetry, asymmetry, None),
        section(Lens::DeadPublic, dead_public, None),
        section(Lens::Bulk, bulk, bulk_unavailable),
        section(Lens::Squash, squash, None),
        section(Lens::FormatDrift, drift, drift_unavailable),
    ];
    let agreements = agreements(&sections);

    let report = LookReport {
        root: shape.root.clone(),
        analyzer: "look-v1 (shape + liveness + line census + longest line + rustfmt)".to_owned(),
        file_line_ceiling,
        supported_files: tree.files.len(),
        functions: shape.functions.len(),
        agreements,
        sections,
        limitations: vec![
            "Every lens is a proxy for reader effort on the error-tolerant AST; a lens cites only its own nearest-rank p90 tail, and agreement between lenses is the ranking, never a weighted score.".to_owned(),
            "Test-classified files are excluded from tangle and asymmetry; nothing here judges whether a cited function is wrong or worth changing.".to_owned(),
            "Dead-public and bulk carry the liveness and line-census limitations verbatim; see those instruments' reports.".to_owned(),
        ],
    };
    Ok((report, candidates))
}

/// (1-based line, columns as UTF-8 characters) of the longest line.
fn longest_line(bytes: &[u8]) -> (usize, usize) {
    bytes
        .split(|b| *b == b'\n')
        .enumerate()
        .map(|(index, line)| (index + 1, String::from_utf8_lossy(line).chars().count()))
        .max_by_key(|(_, columns)| *columns)
        .unwrap_or((1, 0))
}

/// Nearest-rank p90 of magnitude over rows sorted descending; the minimum
/// magnitude when fewer than ten rows exist, so small lenses cite everything.
fn tail_floor(sorted_desc: &[LookRow]) -> f64 {
    if sorted_desc.is_empty() {
        return f64::INFINITY;
    }
    let tail_len = sorted_desc.len().div_ceil(10);
    sorted_desc[tail_len - 1].magnitude
}

/// Files cited by ≥ 2 lens tails, ordered by lens count then path, carrying
/// the worst row per lens. A function row cites its file; file rows cite
/// themselves.
fn agreements(sections: &[LensSection]) -> Vec<Agreement> {
    let mut by_path: BTreeMap<&str, Vec<LookRow>> = BTreeMap::new();
    for section in sections {
        for row in &section.rows {
            let rows = by_path.entry(row.path.as_str()).or_default();
            if !rows.iter().any(|r| r.lens == row.lens) {
                rows.push(row.clone());
            }
        }
    }
    let mut out: Vec<Agreement> = by_path
        .into_iter()
        .filter(|(_, rows)| rows.len() >= 2)
        .map(|(path, rows)| Agreement {
            path: path.to_owned(),
            lenses: rows.iter().map(|r| r.lens).collect(),
            rows,
        })
        .collect();
    out.sort_by(|a, b| {
        b.lenses
            .len()
            .cmp(&a.lenses.len())
            .then_with(|| a.path.cmp(&b.path))
    });
    out
}

/// Line counts over tracked files when `input` is a git worktree (the
/// ceiling law counts every tracked blob), else over supported source files.
fn line_counts(
    input: &Path,
    tree: &crate::source::SourceTree,
) -> (BTreeMap<String, usize>, Option<String>) {
    let mut counts = BTreeMap::new();
    match git(input, &["ls-files", "-z"]) {
        Ok(stdout) => {
            for raw in stdout.split(|byte| *byte == 0).filter(|p| !p.is_empty()) {
                let rel = String::from_utf8_lossy(raw).into_owned();
                let Ok(bytes) = std::fs::read(input.join(&rel)) else {
                    continue;
                };
                counts.insert(rel, bytes.iter().filter(|b| **b == b'\n').count());
            }
            (counts, None)
        }
        Err(error) => {
            for file in &tree.files {
                counts.insert(
                    file.path.clone(),
                    file.bytes.iter().filter(|b| **b == b'\n').count(),
                );
            }
            (
                counts,
                Some(format!(
                    "not a git worktree ({error}); counted supported source files only"
                )),
            )
        }
    }
}

fn format_drift(
    input: &Path,
    line_counts: &BTreeMap<String, usize>,
) -> (Vec<LookRow>, Option<String>) {
    if !input.join("Cargo.toml").is_file() {
        return (
            Vec::new(),
            Some("no Cargo.toml at root; rustfmt lens applies to Rust workspaces only".to_owned()),
        );
    }
    let output = Command::new("cargo")
        .arg("fmt")
        .arg("--check")
        .arg("--")
        .arg("-l")
        .current_dir(input)
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            return (
                Vec::new(),
                Some(format!("cargo fmt could not run: {error}")),
            );
        }
    };
    // rustfmt exits 1 when files differ; any other failure is a real error.
    if !output.status.success() && output.status.code() != Some(1) {
        return (
            Vec::new(),
            Some(format!(
                "cargo fmt --check failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
        );
    }
    let root = std::fs::canonicalize(input).unwrap_or_else(|_| input.to_owned());
    let rows = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let rel = Path::new(line)
                .strip_prefix(&root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| line.to_owned());
            let lines = line_counts.get(&rel).copied().unwrap_or(0);
            LookRow {
                lens: Lens::FormatDrift,
                key: rel.clone(),
                path: rel,
                line: 1,
                evidence: format!("rustfmt would rewrite this file ({lines} lines)"),
                magnitude: lines as f64,
            }
        })
        .collect();
    (rows, None)
}

/// Run `look` at head and at `base` (a detached temporary worktree), and
/// name what head introduced. The head report is returned in full.
pub fn look_delta(
    input: &Path,
    base: &str,
    file_line_ceiling: usize,
) -> Result<LookDelta, LookError> {
    let (head, _) = look_with_candidates(input, file_line_ceiling)?;
    let temp = tempfile::tempdir().map_err(LookError::TempDir)?;
    let base_dir: PathBuf = temp.path().join("base");
    git(
        input,
        &[
            "worktree",
            "add",
            "--detach",
            &base_dir.to_string_lossy(),
            base,
        ],
    )?;
    let base_result = look_with_candidates(&base_dir, file_line_ceiling);
    let removed = git(
        input,
        &["worktree", "remove", "--force", &base_dir.to_string_lossy()],
    );
    let (base_report, base_candidates) = base_result?;
    removed?;

    let mut introduced = Vec::new();
    let mut found_delta = BTreeMap::new();
    for section in &head.sections {
        let base_found = base_report
            .sections
            .iter()
            .find(|s| s.lens == section.lens)
            .map_or(0, |s| s.found as i64);
        found_delta.insert(section.lens, section.found as i64 - base_found);
        let known = base_candidates.get(&section.lens);
        introduced.extend(
            section
                .rows
                .iter()
                .filter(|row| {
                    known
                        .and_then(|map| map.get(&row.key))
                        .is_none_or(|base_magnitude| row.magnitude > *base_magnitude)
                })
                .cloned(),
        );
    }
    Ok(LookDelta {
        base: base.to_owned(),
        head,
        introduced,
        found_delta,
    })
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, LookError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(LookError::GitInvocation)?;
    if !output.status.success() {
        return Err(LookError::Git {
            argv: format!("git -C {} {}", root.display(), args.join(" ")),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(output.stdout)
}
