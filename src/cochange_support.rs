//! Static–historical support and commit-Jaccard coupling on one pinned file universe.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

use serde::Serialize;
use thiserror::Error;

use crate::cochange::{
    CochangeHistoryCoverage, CochangeLayoutError, CochangeSourceProvenance, WEIGHT_SCALE,
    load_pinned_cochange, unit_weight, validate_history_commits,
};
use crate::deps::{
    DependencyClassification, DependencyError, DependencyNodeKind, REACHABILITY_NODE_LIMIT,
    REACHABILITY_WORK_LIMIT, ReachabilityStatus, analyze_dependencies,
    queried_undirected_reachability,
};
use crate::kernel::ArtifactSnapshot;
use crate::repo::{RepoError, read_committed_blobs};
use crate::source::language_for_path;

const ANALYZER: &str = "seval-cochange-support-v1";
const JACCARD_MIN_COOCCURRENCE: u64 = 2;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CochangeSupportConfig {
    pub history_commits: usize,
    pub top: usize,
}

impl Default for CochangeSupportConfig {
    fn default() -> Self {
        Self {
            history_commits: 500,
            top: 30,
        }
    }
}

#[derive(Debug, Error)]
pub enum CochangeSupportError {
    #[error("invalid co-change support configuration: {0}")]
    InvalidConfig(String),
    #[error(transparent)]
    Cochange(#[from] CochangeLayoutError),
    #[error(transparent)]
    Dependency(#[from] DependencyError),
    #[error(transparent)]
    Repository(#[from] RepoError),
    #[error("cannot materialize pinned source file {path}: {source}")]
    Materialize {
        path: String,
        source: std::io::Error,
    },
    #[error("co-change support invariant failed: {0}")]
    Invariant(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct CochangeSupportReport {
    pub artifact: ArtifactSnapshot,
    pub analyzer: String,
    pub static_analyzer: String,
    pub history_coverage: CochangeHistoryCoverage,
    pub source_provenance: CochangeSourceProvenance,
    pub static_snapshot_provenance: StaticSnapshotProvenance,
    pub universe_coverage: SupportUniverseCoverage,
    pub weight_scale: u64,
    pub total_intersected_pair_mass: f64,
    pub total_intersected_pair_mass_ideal: f64,
    pub total_intersected_pair_mass_quantization_bound: f64,
    pub total_intersected_pair_mass_scaled: u128,
    pub total_intersected_pair_mass_ideal_scaled: u128,
    pub total_intersected_pair_mass_quantization_bound_scaled: u128,
    pub support_cross_tab: SupportCrossTab,
    pub reverse_static_edge_support: ReverseStaticEdgeSupport,
    pub commit_jaccard: CommitJaccardProfile,
    pub interpretation: String,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StaticSnapshotProvenance {
    pub git_version: String,
    pub cat_file_command: String,
    pub cat_file_request_sha256: String,
    pub cat_file_stdout_sha256: String,
    pub cat_file_stdout_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SupportUniverseCoverage {
    pub tracked_regular_files: usize,
    pub utf8_path_regular_files: usize,
    pub source_classified_tracked_blobs: usize,
    pub static_analyzed_files: usize,
    pub static_only_files: usize,
    pub cochange_only_files: usize,
    pub intersection_files: usize,
    pub union_files: usize,
    pub intersection_files_touched_in_history: usize,
    pub intersection_files_never_touched: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SupportCrossTab {
    pub reachability_status: ReachabilityStatus,
    pub reachability_node_limit: usize,
    pub reachability_work_limit: usize,
    pub reachability_work_upper_bound: Option<usize>,
    pub direct: SupportMassBin,
    pub transitive_only: Option<SupportMassBin>,
    pub unrelated: Option<SupportMassBin>,
    pub non_direct_uncomputed_mass: Option<SupportMassBin>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SupportMassBin {
    pub pairs: u64,
    pub mass: f64,
    pub mass_scaled: u128,
    pub fraction_of_total: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReverseStaticEdgeSupport {
    pub supported_cross_directory_edges: u64,
    pub cross_directory_edges: u64,
    pub fraction: Option<f64>,
    pub directory_granularity: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitJaccardProfile {
    pub cooccurring_pairs: u64,
    pub distribution_minimum_cooccurrence: u64,
    pub top_pairs_minimum_cooccurrence: u64,
    pub pairs_in_distribution: u64,
    pub distribution: RationalDistribution,
    pub top_pairs: Vec<JaccardPair>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RationalDistribution {
    pub p50: Option<ExactRatio>,
    pub p90: Option<ExactRatio>,
    pub max: Option<ExactRatio>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExactRatio {
    pub numerator: u64,
    pub denominator: u64,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct JaccardPair {
    pub left: String,
    pub right: String,
    pub cooccurrence_commits: u64,
    pub union_commits: u64,
    pub left_commits: u64,
    pub right_commits: u64,
    pub jaccard: ExactRatio,
}

#[derive(Default)]
struct PairHistory {
    mass: u128,
    cooccurrence: u64,
}

pub fn analyze_cochange_support(
    root: &Path,
    config: CochangeSupportConfig,
) -> Result<CochangeSupportReport, CochangeSupportError> {
    validate_history_commits(config.history_commits)
        .map_err(CochangeSupportError::InvalidConfig)?;
    let pinned = load_pinned_cochange(root, config.history_commits)?;
    let static_files = pinned
        .tree_files
        .iter()
        .filter(|entry| {
            std::str::from_utf8(&entry.path)
                .ok()
                .is_some_and(|path| language_for_path(Path::new(path)).is_some())
        })
        .cloned()
        .collect::<Vec<_>>();
    let blob_read = read_committed_blobs(&pinned.artifact, &static_files)?;
    let snapshot = tempfile::tempdir().map_err(|source| CochangeSupportError::Materialize {
        path: "temporary snapshot root".to_owned(),
        source,
    })?;
    for (entry, bytes) in static_files.iter().zip(&blob_read.blobs) {
        let path = std::str::from_utf8(&entry.path).map_err(|_| {
            CochangeSupportError::Invariant(
                "UTF-8-filtered committed path became non-UTF-8".to_owned(),
            )
        })?;
        let relative = Path::new(path);
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err(CochangeSupportError::Invariant(format!(
                "committed source path was not safely relative: {path}"
            )));
        }
        let destination = snapshot.path().join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| CochangeSupportError::Materialize {
                path: path.to_owned(),
                source,
            })?;
        }
        fs::write(&destination, bytes).map_err(|source| CochangeSupportError::Materialize {
            path: path.to_owned(),
            source,
        })?;
    }
    let static_report = analyze_dependencies(snapshot.path())?;
    let static_universe = static_report
        .nodes
        .iter()
        .filter(|node| node.kind == DependencyNodeKind::AnalyzedFile)
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let intersection = pinned
        .source_universe
        .intersection(&static_universe)
        .cloned()
        .collect::<BTreeSet<_>>();
    let accumulation = pinned.accumulate(&intersection)?;
    let path_index = accumulation
        .paths
        .iter()
        .enumerate()
        .map(|(index, path)| (path.as_str(), index))
        .collect::<BTreeMap<_, _>>();

    let mut file_commit_counts = vec![0u64; accumulation.paths.len()];
    let mut pair_history: BTreeMap<(usize, usize), PairHistory> = BTreeMap::new();
    for commit in &accumulation.eligible_commits {
        for &member in &commit.members {
            file_commit_counts[member] =
                file_commit_counts[member].checked_add(1).ok_or_else(|| {
                    CochangeSupportError::Invariant("file commit count overflowed u64".to_owned())
                })?;
        }
        for left_position in 0..commit.members.len() {
            for right_position in left_position + 1..commit.members.len() {
                let left = commit.members[left_position];
                let right = commit.members[right_position];
                let pair = (left.min(right), left.max(right));
                let history = pair_history.entry(pair).or_default();
                history.mass = history.mass.checked_add(commit.unit_mass).ok_or_else(|| {
                    CochangeSupportError::Invariant("pair mass overflowed u128".to_owned())
                })?;
                history.cooccurrence = history.cooccurrence.checked_add(1).ok_or_else(|| {
                    CochangeSupportError::Invariant("pair co-occurrence overflowed u64".to_owned())
                })?;
            }
        }
    }
    let pair_mass_total = pair_history.values().try_fold(0u128, |sum, pair| {
        sum.checked_add(pair.mass).ok_or_else(|| {
            CochangeSupportError::Invariant("pair mass total overflowed u128".to_owned())
        })
    })?;
    if pair_mass_total != accumulation.total_mass {
        return Err(CochangeSupportError::Invariant(
            "pair masses did not close to total intersected mass".to_owned(),
        ));
    }

    let mut adjacency = vec![Vec::new(); accumulation.paths.len()];
    let mut direct_pairs = BTreeSet::new();
    let mut cross_directory_edges = 0u64;
    let mut supported_cross_directory_edges = 0u64;
    for edge in static_report
        .edges
        .iter()
        .filter(|edge| edge.classification == DependencyClassification::Internal)
    {
        let (Some(&source), Some(&target)) = (
            path_index.get(edge.source.as_str()),
            path_index.get(edge.target.as_str()),
        ) else {
            continue;
        };
        adjacency[source].push(target);
        if source != target {
            let pair = (source.min(target), source.max(target));
            direct_pairs.insert(pair);
            if parent_directory(&edge.source) != parent_directory(&edge.target) {
                cross_directory_edges = cross_directory_edges.checked_add(1).ok_or_else(|| {
                    CochangeSupportError::Invariant(
                        "cross-directory static edge count overflowed u64".to_owned(),
                    )
                })?;
                if pair_history.contains_key(&pair) {
                    supported_cross_directory_edges = supported_cross_directory_edges
                        .checked_add(1)
                        .ok_or_else(|| {
                            CochangeSupportError::Invariant(
                                "supported static edge count overflowed u64".to_owned(),
                            )
                        })?;
                }
            }
        }
    }
    for targets in &mut adjacency {
        targets.sort_unstable();
        targets.dedup();
    }
    let non_direct_queries = pair_history
        .keys()
        .filter(|pair| !direct_pairs.contains(pair))
        .copied()
        .collect::<BTreeSet<_>>();
    let reachability = queried_undirected_reachability(&adjacency, &non_direct_queries);

    let mut direct_mass = 0u128;
    let mut direct_count = 0u64;
    for (pair, history) in &pair_history {
        if direct_pairs.contains(pair) {
            direct_mass = direct_mass.checked_add(history.mass).ok_or_else(|| {
                CochangeSupportError::Invariant("direct mass overflowed u128".to_owned())
            })?;
            direct_count = direct_count.checked_add(1).ok_or_else(|| {
                CochangeSupportError::Invariant("direct pair count overflowed u64".to_owned())
            })?;
        }
    }
    let direct = mass_bin(direct_count, direct_mass, pair_mass_total);
    let (transitive_only, unrelated, non_direct_uncomputed_mass) = if let Some(reachable_pairs) =
        &reachability.reachable_pairs
    {
        let mut transitive_mass = 0u128;
        let mut transitive_count = 0u64;
        let mut unrelated_mass = 0u128;
        let mut unrelated_count = 0u64;
        for (pair, history) in &pair_history {
            if direct_pairs.contains(pair) {
                continue;
            }
            if reachable_pairs.contains(pair) {
                transitive_mass = transitive_mass.checked_add(history.mass).ok_or_else(|| {
                    CochangeSupportError::Invariant("transitive mass overflowed u128".to_owned())
                })?;
                transitive_count = transitive_count.checked_add(1).ok_or_else(|| {
                    CochangeSupportError::Invariant(
                        "transitive pair count overflowed u64".to_owned(),
                    )
                })?;
            } else {
                unrelated_mass = unrelated_mass.checked_add(history.mass).ok_or_else(|| {
                    CochangeSupportError::Invariant("unrelated mass overflowed u128".to_owned())
                })?;
                unrelated_count = unrelated_count.checked_add(1).ok_or_else(|| {
                    CochangeSupportError::Invariant(
                        "unrelated pair count overflowed u64".to_owned(),
                    )
                })?;
            }
        }
        let closed = direct_mass
            .checked_add(transitive_mass)
            .and_then(|mass| mass.checked_add(unrelated_mass))
            .ok_or_else(|| {
                CochangeSupportError::Invariant("cross-tab mass overflowed u128".to_owned())
            })?;
        if closed != pair_mass_total {
            return Err(CochangeSupportError::Invariant(
                "cross-tab bins did not close to total intersected mass".to_owned(),
            ));
        }
        (
            Some(mass_bin(transitive_count, transitive_mass, pair_mass_total)),
            Some(mass_bin(unrelated_count, unrelated_mass, pair_mass_total)),
            None,
        )
    } else {
        let pending_mass = pair_mass_total.checked_sub(direct_mass).ok_or_else(|| {
            CochangeSupportError::Invariant("direct mass exceeded total mass".to_owned())
        })?;
        let pending_count = u64::try_from(non_direct_queries.len()).map_err(|_| {
            CochangeSupportError::Invariant("non-direct pair count exceeded u64".to_owned())
        })?;
        (
            None,
            None,
            Some(mass_bin(pending_count, pending_mass, pair_mass_total)),
        )
    };

    let mut jaccard_pairs = pair_history
        .iter()
        .filter_map(|(&(left, right), history)| {
            let left_commits = file_commit_counts[left];
            let right_commits = file_commit_counts[right];
            let union_commits = left_commits
                .checked_add(right_commits)?
                .checked_sub(history.cooccurrence)?;
            Some(JaccardPair {
                left: accumulation.paths[left].clone(),
                right: accumulation.paths[right].clone(),
                cooccurrence_commits: history.cooccurrence,
                union_commits,
                left_commits,
                right_commits,
                jaccard: exact_ratio(history.cooccurrence, union_commits),
            })
        })
        .collect::<Vec<_>>();
    if jaccard_pairs.len() != pair_history.len() {
        return Err(CochangeSupportError::Invariant(
            "Jaccard union count overflowed or underflowed".to_owned(),
        ));
    }
    let mut distribution_values = jaccard_pairs
        .iter()
        .filter(|pair| pair.cooccurrence_commits >= JACCARD_MIN_COOCCURRENCE)
        .map(|pair| pair.jaccard.clone())
        .collect::<Vec<_>>();
    distribution_values.sort_by(ratio_cmp);
    let distribution = RationalDistribution {
        p50: nearest_rank(&distribution_values, 50),
        p90: nearest_rank(&distribution_values, 90),
        max: distribution_values.last().cloned(),
    };
    jaccard_pairs.sort_by(|left, right| {
        ratio_cmp(&right.jaccard, &left.jaccard)
            .then_with(|| right.cooccurrence_commits.cmp(&left.cooccurrence_commits))
            .then_with(|| left.left.cmp(&right.left))
            .then_with(|| left.right.cmp(&right.right))
    });
    jaccard_pairs.truncate(config.top);

    pinned.verify_unchanged()?;
    let history_coverage = pinned.history_coverage(&accumulation);
    let source_provenance = pinned.source_provenance();
    let static_only_files = static_universe.difference(&pinned.source_universe).count();
    let cochange_only_files = pinned.source_universe.difference(&static_universe).count();
    let union_files = static_universe.union(&pinned.source_universe).count();
    let intersection_files = intersection.len();

    Ok(CochangeSupportReport {
        artifact: pinned.artifact,
        analyzer: ANALYZER.to_owned(),
        static_analyzer: static_report.analyzer,
        history_coverage,
        source_provenance: source_provenance.clone(),
        static_snapshot_provenance: StaticSnapshotProvenance {
            git_version: source_provenance.git_version,

            cat_file_command: blob_read.command,
            cat_file_request_sha256: blob_read.request_sha256,
            cat_file_stdout_sha256: blob_read.stdout_sha256,
            cat_file_stdout_bytes: blob_read.stdout_bytes,
        },
        universe_coverage: SupportUniverseCoverage {
            tracked_regular_files: pinned.tracked_regular_files,
            utf8_path_regular_files: pinned.utf8_path_regular_files,
            source_classified_tracked_blobs: pinned.source_universe.len(),
            static_analyzed_files: static_universe.len(),
            static_only_files,
            cochange_only_files,
            intersection_files,
            union_files,
            intersection_files_touched_in_history: accumulation.files_touched_in_history,
            intersection_files_never_touched: intersection_files
                - accumulation.files_touched_in_history,
        },
        weight_scale: WEIGHT_SCALE as u64,
        total_intersected_pair_mass: unit_weight(pair_mass_total),
        total_intersected_pair_mass_ideal: accumulation.eligible_commits.len() as f64,
        total_intersected_pair_mass_quantization_bound: unit_weight(
            accumulation.ideal_mass - pair_mass_total,
        ),
        total_intersected_pair_mass_scaled: pair_mass_total,
        total_intersected_pair_mass_ideal_scaled: accumulation.ideal_mass,
        total_intersected_pair_mass_quantization_bound_scaled: accumulation.ideal_mass
            - pair_mass_total,
        support_cross_tab: SupportCrossTab {
            reachability_status: reachability.status,
            reachability_node_limit: REACHABILITY_NODE_LIMIT,
            reachability_work_limit: REACHABILITY_WORK_LIMIT,
            reachability_work_upper_bound: reachability.work_upper_bound,
            direct,
            transitive_only,
            unrelated,
            non_direct_uncomputed_mass,
        },
        reverse_static_edge_support: ReverseStaticEdgeSupport {
            supported_cross_directory_edges,
            cross_directory_edges,
            fraction: (cross_directory_edges != 0).then(|| {
                supported_cross_directory_edges as f64 / cross_directory_edges as f64
            }),
            directory_granularity: "parent_directory".to_owned(),
        },
        commit_jaccard: CommitJaccardProfile {
            cooccurring_pairs: u64::try_from(pair_history.len()).map_err(|_| {
                CochangeSupportError::Invariant("co-occurring pair count exceeded u64".to_owned())
            })?,
            distribution_minimum_cooccurrence: JACCARD_MIN_COOCCURRENCE,
            top_pairs_minimum_cooccurrence: 1,
            pairs_in_distribution: u64::try_from(distribution_values.len()).map_err(|_| {
                CochangeSupportError::Invariant("Jaccard distribution count exceeded u64".to_owned())
            })?,
            distribution,
            top_pairs: jaccard_pairs,
        },
        interpretation: "Two repositories with identical static and co-change Q×Q coordinates can have opposite support tables; this profile tests whether the same file pairs carry both declared and historical coupling.".to_owned(),
        limitations: limitations(config.history_commits),
    })
}

fn mass_bin(pairs: u64, mass_scaled: u128, total_scaled: u128) -> SupportMassBin {
    SupportMassBin {
        pairs,
        mass: unit_weight(mass_scaled),
        mass_scaled,
        fraction_of_total: (total_scaled != 0).then(|| mass_scaled as f64 / total_scaled as f64),
    }
}

fn exact_ratio(numerator: u64, denominator: u64) -> ExactRatio {
    ExactRatio {
        numerator,
        denominator,
        value: numerator as f64 / denominator as f64,
    }
}

fn ratio_cmp(left: &ExactRatio, right: &ExactRatio) -> Ordering {
    ((left.numerator as u128) * (right.denominator as u128))
        .cmp(&((right.numerator as u128) * (left.denominator as u128)))
}

fn nearest_rank(values: &[ExactRatio], percentile: usize) -> Option<ExactRatio> {
    if values.is_empty() {
        return None;
    }
    let rank = values.len().saturating_mul(percentile).div_ceil(100);
    values.get(rank.saturating_sub(1)).cloned()
}

fn parent_directory(path: &str) -> &str {
    path.rsplit_once('/')
        .map_or(".", |(directory, _)| directory)
}

fn limitations(requested: usize) -> Vec<String> {
    vec![
        format!(
            "History is a bounded window of at most {requested} non-merge commits ending at the pinned revision; requested, streamed, and truncation are reported explicitly."
        ),
        "Squash and rebase rewrite the observed co-change history, and rename detection is disabled, so renamed files do not retain one historical identity.".to_owned(),
        "The inherited broad-commit cap excludes commits touching more than 100 intersection files rather than flooding all pairs with diffuse mass.".to_owned(),
        "The static resolver is conservative; unresolved path aliases and other blind spots make the unrelated bin an over-count because a real dependency can appear unrelated.".to_owned(),
        "Co-change can reflect shared release cadence or commit practice rather than causal coupling; support is evidence of correlated edits, not a causal claim.".to_owned(),
        "Jaccard values on low counts are noisy; the distribution requires at least two co-occurring eligible commits, while the ranked tail includes all co-occurring pairs and shows every co-occurrence and union denominator.".to_owned(),
        "Pair mass uses fixed-point scale 2^40; exact u128 masses are authoritative and displayed f64 values are secondary.".to_owned(),
        "The joined universe is the intersection of source-classified tracked blobs and files analyzed by the static resolver at the same pinned revision; static-only and cochange-only exclusions are reported explicitly.".to_owned(),
    ]
}
