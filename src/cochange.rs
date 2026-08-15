//! Deterministic, evidence-first weighted co-change directory modularity.
//!
//! This instrument reads the *same* two directory partitions as the static
//! layout profile ([`crate::deps`]) but over a different graph: the git
//! co-change graph, where each non-merge commit that touches `k >= 2` in-universe
//! source files contributes total pair mass `1`, split as `1 / C(k,2)` over every
//! unordered pair of those files (Geipel & Schweitzer 2012). The reported number
//! is the weighted Newman modularity `Q` of the directory partition over that
//! graph, read as a coordinate beside the static `Q`, never subtracted into a
//! single congruence score.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::kernel::ArtifactSnapshot;
use crate::repo::{
    RepoError, classified_source, scan_committed_regular_files, scan_file_history, snapshot_git_repo,
};

const ANALYZER: &str = "seval-cochange-layout-v1";

/// Fixed-point unit of one commit's total pair mass. Each pair weight `1/C(k,2)`
/// is stored as `WEIGHT_SCALE / C(k,2)` truncated to an integer, so intra- and
/// cross-community masses close exactly under integer addition while the
/// per-pair truncation error stays below `C(k,2) / WEIGHT_SCALE`.
const WEIGHT_SCALE: u128 = 1 << 40;

/// Commits touching more than this many in-universe source files are counted and
/// excluded as broad commits rather than flooding every pair with `1/C(k,2)`.
const BROAD_COMMIT_CAP: usize = 100;

/// Configuration for the bounded, no-merge co-change history sample.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CochangeLayoutConfig {
    pub history_commits: usize,
}

impl Default for CochangeLayoutConfig {
    fn default() -> Self {
        Self {
            history_commits: 500,
        }
    }
}

impl CochangeLayoutConfig {
    fn validate(&self) -> Result<(), CochangeLayoutError> {
        if (1..=10_000).contains(&self.history_commits) {
            Ok(())
        } else {
            Err(CochangeLayoutError::InvalidConfig(format!(
                "history_commits must be in 1..=10_000, got {}",
                self.history_commits
            )))
        }
    }
}

#[derive(Debug, Error)]
pub enum CochangeLayoutError {
    #[error("invalid co-change layout configuration: {0}")]
    InvalidConfig(String),
    #[error(transparent)]
    Repository(#[from] RepoError),
    #[error(
        "repository snapshot changed during analysis (before {before_revision}/{before_tree}, after {after_revision}/{after_tree})"
    )]
    SnapshotDrift {
        before_revision: String,
        before_tree: String,
        after_revision: String,
        after_tree: String,
    },
    #[error("co-change layout invariant failed: {0}")]
    Invariant(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct CochangeLayoutReport {
    pub artifact: ArtifactSnapshot,
    pub analyzer: String,
    pub history_coverage: CochangeHistoryCoverage,
    pub source_provenance: CochangeSourceProvenance,
    pub universe_coverage: UniverseCoverage,
    /// Fixed-point denominator: a pair mass of `1` is `WEIGHT_SCALE` internal
    /// units. Exposed so consumers can reconstruct the exact rational weights.
    pub weight_scale: u64,
    /// Total pair mass `W` accumulated over eligible commits, in unit weight.
    pub total_pair_weight: f64,
    /// The ideal total pair mass: exactly one per eligible commit.
    pub total_pair_weight_ideal: f64,
    /// Upper bound on `total_pair_weight_ideal - total_pair_weight`, the mass lost
    /// to per-pair fixed-point truncation. Zero for two-file commits.
    pub total_pair_weight_quantization_bound: f64,
    pub partitions: Vec<CochangePartition>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CochangeHistoryCoverage {
    pub requested_commits: usize,
    pub commits_streamed: usize,
    pub truncated: bool,
    /// Commits contributing pair mass: `2 <= k <= broad_commit_cap` in-universe.
    pub eligible_commits: usize,
    /// Commits excluded because they touch more than `broad_commit_cap`.
    pub broad_commits_excluded: usize,
    pub broad_commit_cap: usize,
    /// Commits touching fewer than two in-universe source files (no pairs).
    pub below_pair_threshold_commits: usize,
    pub earliest_committer_unix_seconds: Option<i64>,
    pub latest_committer_unix_seconds: Option<i64>,
    pub git_version: String,
    pub command: String,
    pub stdout_sha256: String,
    pub stdout_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CochangeSourceProvenance {
    pub git_version: String,
    pub ls_tree_command: String,
    pub ls_tree_stdout_sha256: String,
    pub ls_tree_stdout_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UniverseCoverage {
    pub tracked_regular_files: usize,
    pub utf8_path_regular_files: usize,
    pub source_classified_files: usize,
    /// Source-classified files touched by at least one streamed commit.
    pub files_touched_in_history: usize,
    /// Source-classified files never touched in the streamed window.
    pub files_never_touched: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CochangePartition {
    pub granularity: String,
    pub communities: usize,
    pub intra_weight: f64,
    pub cross_weight: f64,
    /// `cross / (intra + cross)`; `None` when the total pair mass is zero.
    pub cross_weight_fraction: Option<f64>,
    /// Weighted Newman modularity `Q = Σ_c [ e_c/W − (d_c/2W)² ]`; `None` when
    /// the total pair mass `W` is zero.
    pub modularity: Option<f64>,
    /// Per-community rows, descending by crossing weight then path.
    pub rows: Vec<CochangeCommunity>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CochangeCommunity {
    pub path: String,
    pub files: usize,
    /// Mass of pairs with both endpoints in this community (`e_c`).
    pub intra_weight: f64,
    /// Mass of pairs with exactly one endpoint in this community (`d_c − 2·e_c`);
    /// each crossing pair is counted at both of its communities.
    pub cross_weight: f64,
}

#[derive(Default)]
struct CommunityMass {
    /// `e_c`: mass of pairs fully inside the community, in fixed-point units.
    intra: u128,
    /// `d_c`: mass incident to the community (twice-intra plus crossing).
    strength: u128,
}

struct PartitionAccumulator {
    granularity: &'static str,
    community_of: fn(&str) -> String,
    file_counts: BTreeMap<String, usize>,
    masses: BTreeMap<String, CommunityMass>,
    intra: u128,
}

impl PartitionAccumulator {
    fn new(
        granularity: &'static str,
        community_of: fn(&str) -> String,
        paths: &[String],
    ) -> Self {
        let mut file_counts: BTreeMap<String, usize> = BTreeMap::new();
        for path in paths {
            *file_counts.entry(community_of(path)).or_default() += 1;
        }
        Self {
            granularity,
            community_of,
            file_counts,
            masses: BTreeMap::new(),
            intra: 0,
        }
    }

    /// Fold one eligible commit: `members` are the communities of its `k`
    /// in-universe files with per-community counts, `unit` is `WEIGHT_SCALE /
    /// C(k,2)`, and `k` is the in-universe file count.
    fn add_commit(
        &mut self,
        members: &BTreeMap<String, u128>,
        unit: u128,
        k: u128,
    ) -> Result<(), CochangeLayoutError> {
        for (community, &count) in members {
            // e_c += unit * C(count, 2); d_c += unit * count * (k - 1).
            let intra_pairs = count * (count - 1) / 2;
            let mass = self.masses.entry(community.clone()).or_default();
            mass.intra = checked_add(mass.intra, unit * intra_pairs, "community intra mass")?;
            mass.strength =
                checked_add(mass.strength, unit * count * (k - 1), "community strength")?;
            self.intra = checked_add(self.intra, unit * intra_pairs, "partition intra mass")?;
        }
        Ok(())
    }

    fn finish(self, total: u128) -> Result<CochangePartition, CochangeLayoutError> {
        let cross = total
            .checked_sub(self.intra)
            .ok_or_else(|| CochangeLayoutError::Invariant("intra mass exceeded total".to_owned()))?;
        let modularity = (total != 0).then(|| {
            let w = total as f64;
            self.masses
                .values()
                .map(|mass| {
                    mass.intra as f64 / w - (mass.strength as f64 / (2.0 * w)).powi(2)
                })
                .sum()
        });
        let cross_weight_fraction = (total != 0).then(|| cross as f64 / total as f64);
        let mut rows = self
            .file_counts
            .iter()
            .map(|(path, &files)| {
                let mass = self.masses.get(path);
                let intra = mass.map_or(0, |m| m.intra);
                let strength = mass.map_or(0, |m| m.strength);
                let crossing = strength.checked_sub(2 * intra).ok_or_else(|| {
                    CochangeLayoutError::Invariant(format!(
                        "community {path} strength was below twice its intra mass"
                    ))
                })?;
                Ok(CochangeCommunity {
                    path: path.clone(),
                    files,
                    intra_weight: unit_weight(intra),
                    cross_weight: unit_weight(crossing),
                })
            })
            .collect::<Result<Vec<_>, CochangeLayoutError>>()?;
        rows.sort_by(|a, b| {
            b.cross_weight
                .total_cmp(&a.cross_weight)
                .then_with(|| a.path.cmp(&b.path))
        });
        Ok(CochangePartition {
            granularity: self.granularity.to_owned(),
            communities: self.file_counts.len(),
            intra_weight: unit_weight(self.intra),
            cross_weight: unit_weight(cross),
            cross_weight_fraction,
            modularity,
            rows,
        })
    }
}

pub fn analyze_cochange_layout(
    root: &Path,
    config: CochangeLayoutConfig,
) -> Result<CochangeLayoutReport, CochangeLayoutError> {
    config.validate()?;
    let artifact = snapshot_git_repo(root)?;

    // Universe: every tracked regular blob with a UTF-8 path that the repo-lexical
    // classifier calls source. Unlike the change-profile current side, no
    // language-parser support is required — a source-classified path in any
    // language participates.
    let tree_scan = scan_committed_regular_files(&artifact)?;
    let tracked_regular_files = tree_scan.files.len();
    let mut utf8_path_regular_files = 0usize;
    let mut universe: BTreeSet<String> = BTreeSet::new();
    for entry in &tree_scan.files {
        if let Ok(path) = std::str::from_utf8(&entry.path) {
            utf8_path_regular_files += 1;
            if classified_source(&entry.path) {
                universe.insert(path.to_owned());
            }
        }
    }
    let paths = universe.iter().cloned().collect::<Vec<_>>();
    let index: BTreeMap<&str, usize> = paths
        .iter()
        .enumerate()
        .map(|(i, path)| (path.as_str(), i))
        .collect();

    let mut partitions = [
        PartitionAccumulator::new("top_level", top_level_community, &paths),
        PartitionAccumulator::new("parent_directory", parent_directory_community, &paths),
    ];

    let history = scan_file_history(&artifact, config.history_commits)?;
    let commits_streamed = history.commits.len();
    let mut eligible_commits = 0usize;
    let mut broad_commits_excluded = 0usize;
    let mut below_pair_threshold_commits = 0usize;
    let mut earliest = None;
    let mut latest = None;
    let mut touched = vec![false; paths.len()];
    let mut total_mass: u128 = 0;
    let mut ideal_mass: u128 = 0;

    for commit in &history.commits {
        earliest = Some(earliest.map_or(commit.committer_unix_seconds, |v: i64| {
            v.min(commit.committer_unix_seconds)
        }));
        latest = Some(latest.map_or(commit.committer_unix_seconds, |v: i64| {
            v.max(commit.committer_unix_seconds)
        }));

        let mut members: BTreeSet<usize> = BTreeSet::new();
        for path in commit.files.keys() {
            if let Ok(text) = std::str::from_utf8(path)
                && let Some(&i) = index.get(text)
            {
                members.insert(i);
            }
        }
        for &i in &members {
            touched[i] = true;
        }
        let k = members.len();
        if k < 2 {
            below_pair_threshold_commits += 1;
            continue;
        }
        if k > BROAD_COMMIT_CAP {
            broad_commits_excluded += 1;
            continue;
        }
        eligible_commits += 1;
        let k_u128 = k as u128;
        let pairs = k_u128 * (k_u128 - 1) / 2;
        let unit = WEIGHT_SCALE / pairs;
        total_mass = checked_add(total_mass, unit * pairs, "total pair mass")?;
        ideal_mass = checked_add(ideal_mass, WEIGHT_SCALE, "ideal pair mass")?;
        for accumulator in &mut partitions {
            let mut counts: BTreeMap<String, u128> = BTreeMap::new();
            for &i in &members {
                *counts.entry((accumulator.community_of)(&paths[i])).or_default() += 1;
            }
            accumulator.add_commit(&counts, unit, k_u128)?;
        }
    }

    if eligible_commits + broad_commits_excluded + below_pair_threshold_commits != commits_streamed {
        return Err(CochangeLayoutError::Invariant(
            "commit disposition counts do not close to the streamed total".to_owned(),
        ));
    }
    let files_touched_in_history = touched.iter().filter(|t| **t).count();

    let partitions = partitions
        .into_iter()
        .map(|accumulator| accumulator.finish(total_mass))
        .collect::<Result<Vec<_>, _>>()?;
    for partition in &partitions {
        let closed = unit_weight(total_mass);
        if (partition.intra_weight + partition.cross_weight - closed).abs() > 1.0e-6 {
            return Err(CochangeLayoutError::Invariant(format!(
                "partition {} intra plus cross did not close to total pair mass",
                partition.granularity
            )));
        }
    }

    let after = snapshot_git_repo(&artifact.root)?;
    if after != artifact {
        return Err(CochangeLayoutError::SnapshotDrift {
            before_revision: artifact.revision,
            before_tree: artifact.tree_digest,
            after_revision: after.revision,
            after_tree: after.tree_digest,
        });
    }

    Ok(CochangeLayoutReport {
        artifact,
        analyzer: ANALYZER.to_owned(),
        history_coverage: CochangeHistoryCoverage {
            requested_commits: config.history_commits,
            commits_streamed,
            truncated: history.truncated,
            eligible_commits,
            broad_commits_excluded,
            broad_commit_cap: BROAD_COMMIT_CAP,
            below_pair_threshold_commits,
            earliest_committer_unix_seconds: earliest,
            latest_committer_unix_seconds: latest,
            git_version: history.git_version,
            command: history.command,
            stdout_sha256: history.stdout_sha256,
            stdout_bytes: history.stdout_bytes,
        },
        source_provenance: CochangeSourceProvenance {
            git_version: tree_scan.git_version,
            ls_tree_command: tree_scan.command,
            ls_tree_stdout_sha256: tree_scan.stdout_sha256,
            ls_tree_stdout_bytes: tree_scan.stdout_bytes,
        },
        universe_coverage: UniverseCoverage {
            tracked_regular_files,
            utf8_path_regular_files,
            source_classified_files: paths.len(),
            files_touched_in_history,
            files_never_touched: paths.len() - files_touched_in_history,
        },
        weight_scale: WEIGHT_SCALE as u64,
        total_pair_weight: unit_weight(total_mass),
        total_pair_weight_ideal: eligible_commits as f64,
        total_pair_weight_quantization_bound: unit_weight(ideal_mass - total_mass),
        partitions,
        limitations: limitations(config.history_commits),
    })
}

fn unit_weight(mass: u128) -> f64 {
    mass as f64 / WEIGHT_SCALE as f64
}

fn checked_add(left: u128, right: u128, name: &str) -> Result<u128, CochangeLayoutError> {
    left.checked_add(right)
        .ok_or_else(|| CochangeLayoutError::Invariant(format!("{name} overflowed u128")))
}

fn top_level_community(path: &str) -> String {
    match path.split_once('/') {
        Some((first, _)) => first.to_owned(),
        None => ".".to_owned(),
    }
}

fn parent_directory_community(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((directory, _)) => directory.to_owned(),
        None => ".".to_owned(),
    }
}

fn limitations(requested: usize) -> Vec<String> {
    vec![
        format!(
            "History is a bounded window of at most {requested} non-merge commits ending at the pinned revision; requested, streamed, and truncation are reported explicitly."
        ),
        "Co-change is a historical proxy on the Evolvability axis, never a beauty claim; it measures whether the tree contains maintenance activity, not whether the layout is good.".to_owned(),
        "Co-change records correlation of edits, not causal or logical coupling; two files changing together may share only a release cadence.".to_owned(),
        "Merge commits are excluded upstream by git log --no-merges and are not separately counted; squashes and rebases collapse or rewrite the co-change history that would otherwise be observed.".to_owned(),
        "Rename detection is disabled, so a renamed file is two identities and its co-change continuity across the rename is not recovered.".to_owned(),
        format!(
            "Commits touching more than {BROAD_COMMIT_CAP} in-universe source files are counted and excluded as broad commits rather than adding a diffuse 1/C(k,2) to every pair."
        ),
        "Commit practice is Goodhart-exposed: splitting one coupled edit across several commits lowers the crossing mass and inflates Q without changing the code.".to_owned(),
        "Pair weights are fixed-point (scale 2^40); intra plus cross close exactly under integer addition, while each pair weight is truncated by less than C(k,2)/2^40 and the total quantization bound is reported.".to_owned(),
        "The universe is every source-classified tracked blob with a UTF-8 path at the pinned revision, independent of language-parser support; the lexical classifier can misjudge unusual layouts.".to_owned(),
        "Modularity Q compares the directory partition to a configuration null model; a single community scores near zero and over-splitting inflates crossing mass, so Q is a coordinate to read beside the static layout Q, never subtracted into one score.".to_owned(),
    ]
}
