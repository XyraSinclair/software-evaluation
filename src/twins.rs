//! Deterministic structural-twin census over the resolved internal file graph.
//!
//! Twin structure is the combinatorial residue of the adjudicated spectral
//! verdict in DETERMINANTS.md: identical neighborhoods are exactly what makes
//! code-graph eigenvectors non-unique (Davis-Kahan), and exactly what
//! template-stamped files produce. This instrument reports the equivalence
//! classes directly instead of the degenerate spectrum.
//!
//! Three findings, all witnesses:
//!
//! - **Open twin classes**: files with identical resolved in-neighborhoods and
//!   identical resolved out-neighborhoods. Grouping is an equivalence relation,
//!   so classes are canonical.
//! - **Closed twin classes**: files whose neighborhoods become identical after
//!   adding the file itself to both sides. Closed-key equality between two
//!   distinct files forces mutual edges, so these are mutually linked siblings
//!   that are otherwise interchangeable.
//! - **Near-twin pairs**: pairs whose tagged (direction, neighbor) sets, after
//!   excluding each other, meet a declared exact Jaccard threshold. The
//!   threshold, floor, and pair cap are configuration, not discovered
//!   structure, and are reported beside the result.
//!
//! A twin class establishes parallel declared structure. It cannot establish
//! semantic duplication, nor that consolidation is desirable: plugin
//! registries, protocol handlers, and fixture families legitimately produce
//! twin classes. Neighborhoods are resolved internal declaration edges only;
//! resolver blind spots truncate them, and twins of the truncated graph may
//! not be twins of the true graph.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::deps::{
    DependencyClassification, DependencyError, DependencyNodeKind, analyze_dependencies,
};

pub const TWINS_DEFAULT_NEAR_PERCENT: u32 = 80;
pub const TWINS_DEFAULT_MIN_CLASS_SHARED: usize = 2;
pub const TWINS_DEFAULT_MIN_SHARED: usize = 3;
pub const TWINS_DEFAULT_MAX_PAIRS: usize = 100;
/// Hard node bound for the O(n^2) near-pair scan. Beyond it the scan is
/// skipped and reported as skipped rather than sampled.
pub const TWINS_NEAR_PAIR_NODE_LIMIT: usize = 4096;

const DIRECTION_IN: u64 = 0;
const DIRECTION_OUT: u64 = 1;

#[derive(Debug, Clone)]
pub struct TwinsConfig {
    /// Exact Jaccard threshold for near-twin pairs, as an integer percent.
    pub near_percent: u32,
    /// Minimum number of distinct shared neighbor files (union of shared in
    /// and out sets, members removed) for a twin class to be reported. A
    /// neighbor linked in both directions counts once, so a lone Rust `mod.rs`
    /// parent cannot satisfy a floor of 2. Classes below the floor are
    /// counted, not shown.
    pub min_class_shared: usize,
    /// Minimum tagged-union size for a near-twin pair to be considered.
    pub min_shared: usize,
    /// Maximum reported near-twin pairs; exceeding it censors the list.
    pub max_pairs: usize,
}

impl Default for TwinsConfig {
    fn default() -> Self {
        Self {
            near_percent: TWINS_DEFAULT_NEAR_PERCENT,
            min_class_shared: TWINS_DEFAULT_MIN_CLASS_SHARED,
            min_shared: TWINS_DEFAULT_MIN_SHARED,
            max_pairs: TWINS_DEFAULT_MAX_PAIRS,
        }
    }
}

#[derive(Debug, Error)]
pub enum TwinsError {
    #[error(transparent)]
    Dependency(#[from] DependencyError),
    #[error("near-percent must be between 1 and 100")]
    InvalidNearPercent,
    #[error("min-shared must be greater than zero")]
    InvalidMinShared,
    #[error("max-pairs must be greater than zero")]
    InvalidMaxPairs,
}

#[derive(Debug, Clone, Serialize)]
pub struct TwinsReport {
    pub root: String,
    pub analyzer: String,
    /// Analyzed files at the dependency walker's denominator.
    pub analyzed_files: usize,
    /// Unique directed internal edges between analyzed files.
    pub internal_edges: usize,
    /// Analyzed files with no internal edge in either direction. Isolated
    /// files are never grouped as twins of each other.
    pub isolated_files: usize,
    pub config: TwinsConfigReport,
    pub open_twin_classes: Vec<TwinClass>,
    pub closed_twin_classes: Vec<TwinClass>,
    pub open_twin_class_count: usize,
    pub closed_twin_class_count: usize,
    /// Twin classes found but below `min_class_shared`; the ledger still
    /// closes: found = reported + suppressed.
    pub suppressed_class_count: usize,
    /// Distinct files that appear only in suppressed classes.
    pub suppressed_member_files: usize,
    /// Distinct files appearing in any reported twin class.
    pub twin_member_files: usize,
    /// Exact `twin_member_files / analyzed_files`; the f64 is display-only.
    pub twin_member_numerator: usize,
    pub twin_member_denominator: usize,
    pub twin_member_fraction: f64,
    pub near_pair_scan: NearPairScanStatus,
    pub near_twin_pairs: Vec<NearTwinPair>,
    /// True when more qualifying pairs existed than `max_pairs`; the list is
    /// then censored, not complete.
    pub near_twin_pairs_censored: bool,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TwinsConfigReport {
    pub near_percent: u32,
    pub min_class_shared: usize,
    pub min_shared: usize,
    pub max_pairs: usize,
    pub near_pair_node_limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NearPairScanStatus {
    /// Every unordered pair of non-isolated files was examined.
    Complete,
    /// Non-isolated files exceeded the node limit; the scan did not run.
    SkippedNodeLimit,
}

/// One exact twin equivalence class. `shared_in`/`shared_out` are the common
/// neighborhoods with class members removed, so closed classes read the same
/// way as open ones; `members_mutually_linked` distinguishes them.
#[derive(Debug, Clone, Serialize)]
pub struct TwinClass {
    pub members: Vec<String>,
    pub member_count: usize,
    pub shared_in: Vec<String>,
    pub shared_out: Vec<String>,
    pub shared_in_count: usize,
    pub shared_out_count: usize,
    /// Distinct files in the union of `shared_in` and `shared_out`; the
    /// class-reporting floor applies to this count.
    pub shared_distinct_count: usize,
    /// True for closed classes: every pair of members carries mutual edges.
    pub members_mutually_linked: bool,
}

/// One near-twin pair under the declared threshold. The exact Jaccard is
/// `intersection / union` over tagged (direction, neighbor) elements after
/// excluding the two files from each other's neighborhoods; the f64 is
/// display-only.
#[derive(Debug, Clone, Serialize)]
pub struct NearTwinPair {
    pub left: String,
    pub right: String,
    pub intersection: usize,
    pub union: usize,
    pub jaccard: f64,
    /// Tagged elements (`in:path` / `out:path`) present only on the left side.
    pub left_only: Vec<String>,
    /// Tagged elements present only on the right side.
    pub right_only: Vec<String>,
}

pub fn analyze_twins(input: &Path, config: &TwinsConfig) -> Result<TwinsReport, TwinsError> {
    if config.near_percent == 0 || config.near_percent > 100 {
        return Err(TwinsError::InvalidNearPercent);
    }
    if config.min_shared == 0 {
        return Err(TwinsError::InvalidMinShared);
    }
    if config.max_pairs == 0 {
        return Err(TwinsError::InvalidMaxPairs);
    }

    let dependency_report = analyze_dependencies(input)?;

    let files: Vec<String> = dependency_report
        .nodes
        .iter()
        .filter(|node| node.kind == DependencyNodeKind::AnalyzedFile)
        .map(|node| node.id.clone())
        .collect();
    let index_of: BTreeMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(index, path)| (path.as_str(), index))
        .collect();

    let mut in_sets: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); files.len()];
    let mut out_sets: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); files.len()];
    let mut internal_edges: BTreeSet<(usize, usize)> = BTreeSet::new();
    for edge in &dependency_report.edges {
        if edge.classification != DependencyClassification::Internal {
            continue;
        }
        let (Some(&source), Some(&target)) = (
            index_of.get(edge.source.as_str()),
            index_of.get(edge.target.as_str()),
        ) else {
            continue;
        };
        if source == target {
            continue;
        }
        if internal_edges.insert((source, target)) {
            out_sets[source].insert(target);
            in_sets[target].insert(source);
        }
    }

    let non_isolated: Vec<usize> = (0..files.len())
        .filter(|&index| !in_sets[index].is_empty() || !out_sets[index].is_empty())
        .collect();
    let isolated_files = files.len() - non_isolated.len();

    // Open census: identical raw neighborhoods.
    let mut open_groups: BTreeMap<(Vec<usize>, Vec<usize>), Vec<usize>> = BTreeMap::new();
    for &index in &non_isolated {
        let key = (
            in_sets[index].iter().copied().collect::<Vec<_>>(),
            out_sets[index].iter().copied().collect::<Vec<_>>(),
        );
        open_groups.entry(key).or_default().push(index);
    }

    // Closed census: identical neighborhoods after self-inclusion. Equality of
    // closed keys between distinct files forces mutual edges.
    let mut closed_groups: BTreeMap<(Vec<usize>, Vec<usize>), Vec<usize>> = BTreeMap::new();
    for &index in &non_isolated {
        let mut in_key: Vec<usize> = in_sets[index].iter().copied().collect();
        let mut out_key: Vec<usize> = out_sets[index].iter().copied().collect();
        insert_sorted(&mut in_key, index);
        insert_sorted(&mut out_key, index);
        closed_groups
            .entry((in_key, out_key))
            .or_default()
            .push(index);
    }

    let open_member_sets: BTreeSet<Vec<usize>> = open_groups
        .values()
        .filter(|members| members.len() >= 2)
        .cloned()
        .collect();

    let all_open_classes: Vec<TwinClass> = open_groups
        .into_iter()
        .filter(|(_, members)| members.len() >= 2)
        .map(|((in_key, out_key), members)| build_class(&files, &members, &in_key, &out_key, false))
        .collect();
    let all_closed_classes: Vec<TwinClass> = closed_groups
        .into_iter()
        .filter(|(_, members)| members.len() >= 2 && !open_member_sets.contains(&members.clone()))
        .map(|((in_key, out_key), members)| build_class(&files, &members, &in_key, &out_key, true))
        .collect();
    let meets_floor = |class: &TwinClass| class.shared_distinct_count >= config.min_class_shared;
    let mut suppressed_members: BTreeSet<String> = BTreeSet::new();
    let mut suppressed_class_count = 0usize;
    let mut open_twin_classes: Vec<TwinClass> = Vec::new();
    let mut closed_twin_classes: Vec<TwinClass> = Vec::new();
    for class in all_open_classes {
        if meets_floor(&class) {
            open_twin_classes.push(class);
        } else {
            suppressed_class_count += 1;
            suppressed_members.extend(class.members.iter().cloned());
        }
    }
    for class in all_closed_classes {
        if meets_floor(&class) {
            closed_twin_classes.push(class);
        } else {
            suppressed_class_count += 1;
            suppressed_members.extend(class.members.iter().cloned());
        }
    }
    sort_classes(&mut open_twin_classes);
    sort_classes(&mut closed_twin_classes);

    let mut twin_members: BTreeSet<&str> = BTreeSet::new();
    for class in open_twin_classes.iter().chain(closed_twin_classes.iter()) {
        for member in &class.members {
            twin_members.insert(member.as_str());
        }
    }
    let suppressed_member_files = suppressed_members
        .iter()
        .filter(|member| !twin_members.contains(member.as_str()))
        .count();
    let mut class_partners: BTreeSet<(usize, usize)> = BTreeSet::new();
    for class in open_twin_classes.iter().chain(closed_twin_classes.iter()) {
        let indices: Vec<usize> = class
            .members
            .iter()
            .map(|member| index_of[member.as_str()])
            .collect();
        for (position, &left) in indices.iter().enumerate() {
            for &right in &indices[position + 1..] {
                class_partners.insert((left.min(right), left.max(right)));
            }
        }
    }

    // Near-pair scan over tagged neighborhoods with co-member exclusion.
    let mut near_twin_pairs: Vec<NearTwinPair> = Vec::new();
    let mut near_twin_pairs_censored = false;
    let near_pair_scan = if non_isolated.len() > TWINS_NEAR_PAIR_NODE_LIMIT {
        NearPairScanStatus::SkippedNodeLimit
    } else {
        let tagged: Vec<Vec<u64>> = (0..files.len())
            .map(|index| {
                let mut elements: Vec<u64> = in_sets[index]
                    .iter()
                    .map(|&neighbor| tag(DIRECTION_IN, neighbor))
                    .chain(
                        out_sets[index]
                            .iter()
                            .map(|&neighbor| tag(DIRECTION_OUT, neighbor)),
                    )
                    .collect();
                elements.sort_unstable();
                elements
            })
            .collect();
        let mut qualifying: Vec<NearTwinPair> = Vec::new();
        for (position, &left) in non_isolated.iter().enumerate() {
            for &right in &non_isolated[position + 1..] {
                if class_partners.contains(&(left, right)) {
                    continue;
                }
                let (intersection, union, left_only, right_only) =
                    tagged_overlap(&tagged[left], &tagged[right], left, right);
                if union < config.min_shared || intersection == 0 {
                    continue;
                }
                if (intersection as u128) * 100 < (config.near_percent as u128) * (union as u128) {
                    continue;
                }
                qualifying.push(NearTwinPair {
                    left: files[left].clone(),
                    right: files[right].clone(),
                    intersection,
                    union,
                    jaccard: intersection as f64 / union as f64,
                    left_only: render_tagged(&files, &left_only),
                    right_only: render_tagged(&files, &right_only),
                });
            }
        }
        qualifying.sort_by(|a, b| {
            let a_cross = (a.intersection as u128) * (b.union as u128);
            let b_cross = (b.intersection as u128) * (a.union as u128);
            b_cross
                .cmp(&a_cross)
                .then(b.union.cmp(&a.union))
                .then(a.left.cmp(&b.left))
                .then(a.right.cmp(&b.right))
        });
        if qualifying.len() > config.max_pairs {
            near_twin_pairs_censored = true;
            qualifying.truncate(config.max_pairs);
        }
        near_twin_pairs = qualifying;
        NearPairScanStatus::Complete
    };

    let analyzed_files = files.len();
    let twin_member_files = twin_members.len();
    Ok(TwinsReport {
        root: dependency_report.root.clone(),
        analyzer: "structural twin census over resolved internal dependency edges".to_owned(),
        analyzed_files,
        internal_edges: internal_edges.len(),
        isolated_files,
        config: TwinsConfigReport {
            near_percent: config.near_percent,
            min_class_shared: config.min_class_shared,
            min_shared: config.min_shared,
            max_pairs: config.max_pairs,
            near_pair_node_limit: TWINS_NEAR_PAIR_NODE_LIMIT,
        },
        open_twin_class_count: open_twin_classes.len(),
        closed_twin_class_count: closed_twin_classes.len(),
        suppressed_class_count,
        suppressed_member_files,
        open_twin_classes,
        closed_twin_classes,
        twin_member_files,
        twin_member_numerator: twin_member_files,
        twin_member_denominator: analyzed_files,
        twin_member_fraction: if analyzed_files == 0 {
            0.0
        } else {
            twin_member_files as f64 / analyzed_files as f64
        },
        near_pair_scan,
        near_twin_pairs,
        near_twin_pairs_censored,
        limitations: vec![
            "Neighborhoods are resolved internal declaration edges only; resolver blind spots (aliases, dynamic import, build conditions) truncate them, and twins of the truncated graph may not be twins of the true graph.".to_owned(),
            "Twin structure establishes parallel declared shape, not semantic duplication and not that consolidation is desirable; plugin registries, protocol handlers, and fixture families legitimately produce twin classes.".to_owned(),
            "Files with empty neighborhoods are counted as isolated and never grouped as twins of each other.".to_owned(),
            "Near-twin threshold, the class-shared floor, the pair union floor, and the pair cap are configuration, not discovered structure; suppressed classes are counted, and a censored pair list is incomplete, not complete.".to_owned(),
            "Rust parent<->child mod declarations make the files of one parent module trivial one-neighbor twins; the distinct-neighbor class floor exists for exactly this confounder.".to_owned(),
        ],
    })
}

fn tag(direction: u64, neighbor: usize) -> u64 {
    (direction << 32) | neighbor as u64
}

fn insert_sorted(sorted: &mut Vec<usize>, value: usize) {
    if let Err(position) = sorted.binary_search(&value) {
        sorted.insert(position, value);
    }
}

fn build_class(
    files: &[String],
    members: &[usize],
    in_key: &[usize],
    out_key: &[usize],
    members_mutually_linked: bool,
) -> TwinClass {
    let member_set: BTreeSet<usize> = members.iter().copied().collect();
    let shared_in: Vec<String> = in_key
        .iter()
        .filter(|index| !member_set.contains(index))
        .map(|&index| files[index].clone())
        .collect();
    let shared_out: Vec<String> = out_key
        .iter()
        .filter(|index| !member_set.contains(index))
        .map(|&index| files[index].clone())
        .collect();
    let shared_distinct_count = shared_in
        .iter()
        .chain(shared_out.iter())
        .collect::<BTreeSet<_>>()
        .len();
    TwinClass {
        members: members.iter().map(|&index| files[index].clone()).collect(),
        member_count: members.len(),
        shared_in_count: shared_in.len(),
        shared_out_count: shared_out.len(),
        shared_in,
        shared_out,
        shared_distinct_count,
        members_mutually_linked,
    }
}

fn sort_classes(classes: &mut [TwinClass]) {
    classes.sort_by(|a, b| {
        let a_weight = a.member_count * (a.shared_distinct_count + 1);
        let b_weight = b.member_count * (b.shared_distinct_count + 1);
        b_weight
            .cmp(&a_weight)
            .then(b.member_count.cmp(&a.member_count))
            .then(a.members.cmp(&b.members))
    });
}

/// Two-pointer overlap over sorted tagged element vectors, excluding the two
/// files from each other's neighborhoods. Returns intersection size, union
/// size, and the difference witnesses.
fn tagged_overlap(
    left_elements: &[u64],
    right_elements: &[u64],
    left_index: usize,
    right_index: usize,
) -> (usize, usize, Vec<u64>, Vec<u64>) {
    let excluded_from_left = [
        tag(DIRECTION_IN, right_index),
        tag(DIRECTION_OUT, right_index),
    ];
    let excluded_from_right = [
        tag(DIRECTION_IN, left_index),
        tag(DIRECTION_OUT, left_index),
    ];
    let left_filtered = left_elements
        .iter()
        .copied()
        .filter(|element| !excluded_from_left.contains(element));
    let right_filtered = right_elements
        .iter()
        .copied()
        .filter(|element| !excluded_from_right.contains(element));
    let left_vec: Vec<u64> = left_filtered.collect();
    let right_vec: Vec<u64> = right_filtered.collect();

    let mut intersection = 0usize;
    let mut left_only = Vec::new();
    let mut right_only = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < left_vec.len() && j < right_vec.len() {
        match left_vec[i].cmp(&right_vec[j]) {
            std::cmp::Ordering::Equal => {
                intersection += 1;
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => {
                left_only.push(left_vec[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                right_only.push(right_vec[j]);
                j += 1;
            }
        }
    }
    left_only.extend_from_slice(&left_vec[i..]);
    right_only.extend_from_slice(&right_vec[j..]);
    let union = intersection + left_only.len() + right_only.len();
    (intersection, union, left_only, right_only)
}

fn render_tagged(files: &[String], elements: &[u64]) -> Vec<String> {
    elements
        .iter()
        .map(|&element| {
            let direction = element >> 32;
            let index = (element & 0xFFFF_FFFF) as usize;
            let prefix = if direction == DIRECTION_IN {
                "in"
            } else {
                "out"
            };
            format!("{prefix}:{}", files[index])
        })
        .collect()
}
