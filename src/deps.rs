//! Deterministic, evidence-first static dependency graph analysis.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use thiserror::Error;
use tree_sitter::Node;

use crate::conductance::{
    CONDUCTANCE_DENOMINATOR_POWER, CONDUCTANCE_NODE_LIMIT, ConductanceCertificate,
    conductance_certificates,
};
use crate::source::{SourceError, SourceFile, SourceLanguage, load_source_tree, parse_source};

#[derive(Debug, Error)]
pub enum DependencyError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error("cannot read dependency manifest {path}: {source}")]
    ManifestRead {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot parse dependency manifest {path}: {message}")]
    ManifestParse { path: PathBuf, message: String },
    #[error("dependency graph invariant failed: {0}")]
    Invariant(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyReport {
    pub root: String,
    pub analyzer: String,
    pub coverage: DependencyCoverage,
    pub limitations: Vec<String>,
    pub syntax_error_files: usize,
    pub unreadable_manifests: Vec<UnreadableManifest>,
    pub manifest_dependencies: Vec<ManifestDependency>,
    pub manifest_dependency_count: usize,
    pub non_registry_manifest_dependency_count: usize,
    pub risky_manifest_dependency_count: usize,
    pub manifest_source_kind_counts: BTreeMap<String, usize>,
    pub nodes: Vec<DependencyNode>,
    pub edges: Vec<DependencyEdge>,
    pub node_count: usize,
    pub edge_count: usize,
    pub internal_edges: usize,
    pub external_edges: usize,
    pub unresolved_edges: usize,
    pub strongly_connected_components: Vec<Vec<String>>,
    pub cycles: Vec<Vec<String>>,
    pub weak_components: Vec<Vec<String>>,
    pub condensation_maximum_depth: Option<usize>,
    pub condensation_depth: Option<CondensationDepthProfile>,
    pub propagation: DependencyPropagation,
    pub layout: DependencyLayout,
    pub conductance_certificate_node_limit: usize,
    pub conductance_certificate_denominator_power: u32,
    /// Per-component negative evidence that no cut is sparser than its exact
    /// Cheeger lower bound; this is a cohesion coordinate, not a design verdict.
    pub conductance_certificates: Vec<ConductanceCertificate>,
}

/// Longest-path depth profile of the SCC-condensation DAG: the
/// sequential-abstraction-boundary count that reachability cannot see. A deep
/// thin chain, a shallow wide fan, and a high-fan-in stable kernel have
/// different depth shapes at equal reach. All integers; percentiles are
/// nearest-rank over per-file values (each file inherits its SCC's depths).
#[derive(Debug, Clone, Serialize)]
pub struct CondensationDepthProfile {
    pub condensation_nodes: usize,
    pub condensation_edges: usize,
    /// Denominator for the distributions below.
    pub source_files: usize,
    /// Longest path (in condensation edges) from any source to this file's SCC.
    pub depth_in_p50: usize,
    pub depth_in_p90: usize,
    pub depth_in_max: usize,
    /// Longest path from this file's SCC to any sink.
    pub depth_out_p50: usize,
    pub depth_out_p90: usize,
    pub depth_out_max: usize,
    /// One maximum-length path through the condensation, as witness: the
    /// sorted-first file of each SCC on it, with the SCC's file count.
    pub longest_path: Vec<CondensationPathStep>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CondensationPathStep {
    pub file: String,
    pub scc_files: usize,
}

pub const REACHABILITY_NODE_LIMIT: usize = 10_000;
pub const REACHABILITY_WORK_LIMIT: usize = 100_000_000;

#[derive(Debug, Clone, Serialize)]
pub struct DependencyPropagation {
    pub source_files: usize,
    pub reachability_status: ReachabilityStatus,
    pub reachability_node_limit: usize,
    pub reachability_work_limit: usize,
    pub reachability_work_upper_bound: Option<usize>,
    pub reachable_nonself_pairs: Option<usize>,
    pub possible_nonself_pairs: Option<usize>,
    pub nonself_propagation_fraction: Option<f64>,
    /// Ordered pairs (u, v), u != v, with u and v in the same strongly connected
    /// component: sum over SCC sizes s of s * (s - 1). Exact from Tarjan
    /// output; u128 so the integers are architecture-independent.
    pub mutually_reachable_pairs: u128,
    /// n * (n - 1), the denominator of the fraction below; None for fewer than
    /// two files. The integers are authoritative; the f64 is display.
    pub mutual_possible_pairs: Option<u128>,
    /// mutually_reachable_pairs over n * (n - 1); None for fewer than two files.
    /// The fraction of ordered file pairs with no one-directional reading order.
    pub mutual_reachability_fraction: Option<f64>,
    pub cyclic_components: usize,
    pub cyclic_source_files: usize,
    pub cyclic_source_file_fraction: Option<f64>,
    pub largest_cyclic_component_files: usize,
    pub largest_cyclic_component_fraction: Option<f64>,
    /// Weakly connected components of the internal dependency graph.
    pub weak_components: usize,
    pub largest_weak_component_files: usize,
    pub largest_weak_component_fraction: Option<f64>,
    /// Mutual reachability restated inside the weak component where it is
    /// worst: the global n*(n-1) denominator dilutes a tangle embedded in a
    /// large repository of unrelated files, so the same sum s*(s-1) is also
    /// reported over W*(W-1) of its own weak component. Argmax is exact
    /// (integer cross-multiplication), ties broken toward the larger
    /// component.
    pub worst_weak_component_mutually_reachable_pairs: u128,
    pub worst_weak_component_files: usize,
    pub worst_weak_component_mutual_reachability_fraction: Option<f64>,
}

/// Directory-layout profile over the internal dependency graph: how well the
/// on-disk tree partitions the graph, as a coordinate rather than a verdict.
#[derive(Debug, Clone, Serialize)]
pub struct DependencyLayout {
    /// Analyzed source files, i.e. the nodes considered for the layout graph.
    pub analyzed_files: usize,
    /// Unique internal edges after dropping self-loops and collapsing each
    /// `a ↔ b` reciprocal pair into one undirected edge; the modularity `m`.
    pub internal_undirected_edges: usize,
    /// Three deterministic rows: `top_level` (community = first path
    /// component; root files share community `"."`), `parent_directory`
    /// (community = immediate parent directory path; root files share
    /// community `"."`), and `detected_louvain` (a heuristic witness found by
    /// fixed-order Louvain with exact-integer move comparisons).
    pub partitions: Vec<LayoutPartition>,
    /// Exact witness headroom `Q_detected_louvain - Q_parent_directory` over
    /// the common `4m²` denominator. This establishes an attainable
    /// improvement over the directory partition, not an optimum.
    pub headroom: LayoutHeadroom,
    /// Honest constraints on what the layout profile establishes.
    pub limitations: Vec<String>,
}

/// One fixed or detected partition of the internal graph and its
/// Newman–Girvan score.
#[derive(Debug, Clone, Serialize)]
pub struct LayoutPartition {
    /// Partition granularity: `top_level`, `parent_directory`, or
    /// `detected_louvain`.
    pub granularity: String,
    /// `fixed_directory_partition` for observed directory buckets;
    /// `heuristic_witness` for Louvain. The latter uses exact arithmetic to
    /// evaluate a heuristically found partition: its Q is a witness lower
    /// bound on attainable modularity, never a claim of optimality.
    pub epistemic_class: String,
    /// Distinct communities (directory buckets) present in this partition.
    pub communities: usize,
    /// Undirected internal edges whose endpoints share a community.
    pub intra_community_edges: usize,
    /// Undirected internal edges whose endpoints lie in different communities.
    pub cross_community_edges: usize,
    /// `cross / (intra + cross)`; `None` when there are no undirected edges.
    pub cross_community_edge_fraction: Option<f64>,
    /// Newman–Girvan modularity `Q = Σ_c [ e_c/m − (d_c/2m)² ]` with `m`
    /// undirected edges, `e_c` edges inside `c`, `d_c` the degree sum of nodes
    /// in `c`; `None` when `m = 0`. Derived from the exact rational below.
    pub modularity: Option<f64>,
    /// Exact rational form of Q: signed numerator `Σ_c (4m·e_c − d_c²)` over
    /// denominator `4m²`. The integers are authoritative; the f64 is display.
    pub modularity_numerator: Option<i128>,
    pub modularity_denominator: Option<u128>,
    /// Sum of `min(e_ab, e_ba)` over unordered crossing community pairs.
    /// The integer is authoritative; see `direction_inconsistency_denominator`.
    pub direction_inconsistency_numerator: usize,
    /// Sum of `e_ab + e_ba` over unordered crossing community pairs. This is
    /// exactly the number of directed internal edges crossing this partition.
    pub direction_inconsistency_denominator: usize,
    /// Quotient-level two-way coupling: numerator / denominator above; `None`
    /// when no directed internal edge crosses this partition. Display only.
    pub direction_inconsistency: Option<f64>,
    /// Per-unordered-community-pair directed crossing counts and witnesses,
    /// descending by `min(e_ab, e_ba)` then paths.
    pub direction_pairs: Vec<LayoutDirectionPair>,
    /// Per-community rows, descending by crossing-edge count then path.
    pub rows: Vec<LayoutCommunity>,
}

/// Directed crossing counts for one unordered pair of layout communities.
#[derive(Debug, Clone, Serialize)]
pub struct LayoutDirectionPair {
    /// Lexicographically first community path.
    pub path_a: String,
    /// Lexicographically second community path.
    pub path_b: String,
    /// Directed edges from `path_a` to `path_b`.
    pub e_ab: usize,
    /// Directed edges from `path_b` to `path_a`.
    pub e_ba: usize,
    /// Up to five deterministic crossing-edge witnesses. Empty when either
    /// direction has zero edges, because the pair then adds no inconsistency.
    pub edge_witnesses: Vec<LayoutEdgeWitness>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct LayoutEdgeWitness {
    pub source: String,
    pub target: String,
}

/// One community (directory bucket) within a layout partition.
#[derive(Debug, Clone, Serialize)]
pub struct LayoutCommunity {
    /// Community key: first path component (`top_level`) or immediate parent
    /// directory path (`parent_directory`), with `"."` for root-level files;
    /// detected communities use stable path-order ordinals.
    pub path: String,
    /// Analyzed files assigned to this community.
    pub files: usize,
    /// Undirected internal edges with both endpoints inside this community.
    pub intra_edges: usize,
    /// Directed internal edges with source inside and target outside.
    pub out_edges: usize,
    /// Directed internal edges with target inside and source outside.
    pub in_edges: usize,
    /// Member files targeted by at least one cross-community directed edge.
    /// The denominator is `files`.
    pub boundary_in_files: usize,
    /// Member files sourcing at least one cross-community directed edge. The
    /// denominator is `files`.
    pub boundary_out_files: usize,
    /// Size of the deterministic greedy upper-bound witness covering at least
    /// 90% of this community's cross-boundary directed-edge endpoints.
    pub boundary_cover_90_files: usize,
    /// Member file paths in that greedy witness, descending by crossing count
    /// with path as the deterministic tie-break.
    pub boundary_cover_90_file_paths: Vec<String>,
    /// Top-level-directory membership counts for a detected community, sorted
    /// by directory. Empty for the two directory partitions.
    pub top_level_directory_membership: Vec<LayoutDirectoryMembership>,
    /// Lexicographically first directory among those tied for the largest
    /// detected-community membership. `None` for directory partitions.
    pub majority_top_level_directory: Option<String>,
    /// Exact majority-directory purity numerator and denominator. Both are
    /// `None` for directory partitions; the f64 is display only.
    pub directory_purity_numerator: Option<usize>,
    pub directory_purity_denominator: Option<usize>,
    pub directory_purity: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LayoutDirectoryMembership {
    pub directory: String,
    pub files: usize,
}

/// Exact signed layout-headroom difference. Sign plus magnitude avoids an
/// overflowing `i128` subtraction when two individually representable Q
/// numerators have opposite signs.
#[derive(Debug, Clone, Serialize)]
pub struct LayoutHeadroom {
    pub witness_granularity: String,
    pub baseline_granularity: String,
    pub modularity_difference: Option<f64>,
    pub numerator_negative: Option<bool>,
    pub numerator_magnitude: Option<u128>,
    pub denominator: Option<u128>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityStatus {
    Computed,
    NotApplicable,
    SizeLimit,
    WorkLimit,
}

struct ReachabilityComputation {
    status: ReachabilityStatus,
    work_upper_bound: Option<usize>,
    incoming: Option<Vec<usize>>,
    outgoing: Option<Vec<usize>>,
}

pub(crate) struct QueriedReachability {
    pub(crate) status: ReachabilityStatus,
    pub(crate) work_upper_bound: Option<usize>,
    pub(crate) reachable_pairs: Option<BTreeSet<(usize, usize)>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyCoverage {
    pub filesystem_entries_enumerated: usize,
    pub source_files_analyzed: usize,
    pub unsupported_entries_skipped: usize,
    pub declarations_extracted: usize,
    pub unique_edges: usize,
    pub manifests_analyzed: usize,
    /// Manifests found but skipped because they could not be read or parsed;
    /// each skip is named with its path and reason in `unreadable_manifests`.
    pub manifests_unreadable: usize,
}

/// A manifest the inventory found but could not read or parse. Recorded and
/// skipped rather than aborting the whole analysis: broken fixture manifests
/// are common in real repositories, and an abort would make the denominator
/// zero instead of honest.
#[derive(Debug, Clone, Serialize)]
pub struct UnreadableManifest {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyNode {
    pub id: String,
    pub kind: DependencyNodeKind,
    pub fan_in: usize,
    pub fan_out: usize,
    pub direct_internal_in_degree: Option<usize>,
    pub direct_internal_out_degree: Option<usize>,
    pub transitive_internal_in_count: Option<usize>,
    pub transitive_internal_out_count: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyNodeKind {
    AnalyzedFile,
    ExternalSpecifier,
    UnresolvedSpecifier,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyEdge {
    pub source: String,
    pub target: String,
    pub classification: DependencyClassification,
    pub evidence: Vec<DependencyEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyClassification {
    Internal,
    External,
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct DependencyEvidence {
    pub source_path: String,
    pub line: usize,
    pub raw_specifier: String,
    pub kind: String,
    pub resolved_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ManifestDependency {
    pub manifest: String,
    pub ecosystem: String,
    pub scope: String,
    pub name: String,
    pub requirement: String,
    pub source_kind: ManifestSourceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManifestSourceKind {
    Registry,
    Path,
    Git,
    Workspace,
    Wildcard,
    Unknown,
}

#[derive(Debug)]
struct Declaration {
    line: usize,
    specifier: String,
    kind: &'static str,
    hint: ResolutionHint,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolutionHint {
    Path,
    Package,
    RustModule,
    RustUse,
    GoPackage,
}

pub fn analyze_dependencies(input: &Path) -> Result<DependencyReport, DependencyError> {
    let source_tree = load_source_tree(input)?;
    let known: BTreeSet<String> = source_tree.files.iter().map(|f| f.path.clone()).collect();
    let mut syntax_error_files = 0;
    let mut declarations = Vec::new();
    for file in &source_tree.files {
        let parsed = parse_source(file)?;
        syntax_error_files += usize::from(parsed.has_syntax_errors);
        let mut found = Vec::new();
        walk(parsed.tree.root_node(), file, &mut found);
        found.sort_by(|a, b| (a.line, &a.kind, &a.specifier).cmp(&(b.line, &b.kind, &b.specifier)));
        declarations.extend(found.into_iter().map(|d| (file, d)));
    }

    let mut grouped: BTreeMap<(String, String, DependencyClassification), Vec<DependencyEvidence>> =
        BTreeMap::new();
    for (file, declaration) in &declarations {
        let resolved = resolve(file, declaration, &known);
        let (target, class) = match resolved {
            Some(path) => (path, DependencyClassification::Internal),
            None if is_external(declaration) => (
                format!("external:{}", declaration.specifier),
                DependencyClassification::External,
            ),
            None => (
                format!("unresolved:{}", declaration.specifier),
                DependencyClassification::Unresolved,
            ),
        };
        grouped
            .entry((file.path.clone(), target.clone(), class))
            .or_default()
            .push(DependencyEvidence {
                source_path: file.path.clone(),
                line: declaration.line,
                raw_specifier: declaration.specifier.clone(),
                kind: declaration.kind.to_owned(),
                resolved_target: (class == DependencyClassification::Internal).then_some(target),
            });
    }
    let edges: Vec<_> = grouped
        .into_iter()
        .map(|((source, target, classification), mut evidence)| {
            evidence.sort();
            evidence.dedup();
            DependencyEdge {
                source,
                target,
                classification,
                evidence,
            }
        })
        .collect();

    let mut node_kinds: BTreeMap<String, DependencyNodeKind> = known
        .iter()
        .cloned()
        .map(|p| (p, DependencyNodeKind::AnalyzedFile))
        .collect();
    for edge in &edges {
        node_kinds
            .entry(edge.target.clone())
            .or_insert(match edge.classification {
                DependencyClassification::Internal => DependencyNodeKind::AnalyzedFile,
                DependencyClassification::External => DependencyNodeKind::ExternalSpecifier,
                DependencyClassification::Unresolved => DependencyNodeKind::UnresolvedSpecifier,
            });
    }
    let mut incoming: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut outgoing: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for edge in &edges {
        outgoing
            .entry(edge.source.clone())
            .or_default()
            .insert(edge.target.clone());
        incoming
            .entry(edge.target.clone())
            .or_default()
            .insert(edge.source.clone());
    }
    let source_paths = known.iter().cloned().collect::<Vec<_>>();
    let source_index = source_paths
        .iter()
        .enumerate()
        .map(|(index, path)| (path.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut indexed_outgoing = vec![Vec::new(); source_paths.len()];
    let mut indexed_incoming = vec![Vec::new(); source_paths.len()];
    for edge in edges
        .iter()
        .filter(|edge| edge.classification == DependencyClassification::Internal)
    {
        let source = source_index[edge.source.as_str()];
        let target = source_index[edge.target.as_str()];
        indexed_outgoing[source].push(target);
        indexed_incoming[target].push(source);
    }
    let reachability = transitive_internal_degrees(&indexed_outgoing);
    let nodes = node_kinds
        .iter()
        .map(|(id, kind)| {
            let source = source_index.get(id.as_str()).copied();
            DependencyNode {
                id: id.clone(),
                kind: *kind,
                fan_in: incoming.get(id).map_or(0, BTreeSet::len),
                fan_out: outgoing.get(id).map_or(0, BTreeSet::len),
                direct_internal_in_degree: source.map(|index| indexed_incoming[index].len()),
                direct_internal_out_degree: source.map(|index| indexed_outgoing[index].len()),
                transitive_internal_in_count: source
                    .and_then(|index| reachability.incoming.as_ref().map(|values| values[index])),
                transitive_internal_out_count: source
                    .and_then(|index| reachability.outgoing.as_ref().map(|values| values[index])),
            }
        })
        .collect::<Vec<_>>();

    let internal_adjacency = adjacency(&known, &edges, true);
    let sccs = tarjan(&known, &internal_adjacency);
    let cycles: Vec<Vec<String>> = sccs
        .iter()
        .filter(|c| {
            c.len() > 1
                || internal_adjacency
                    .get(&c[0])
                    .is_some_and(|n| n.contains(&c[0]))
        })
        .cloned()
        .collect();
    let internal_weak = weak_components(&known, &internal_adjacency);
    let propagation =
        dependency_propagation(source_paths.len(), &cycles, &internal_weak, &reachability);
    let undirected_internal_edges = internal_undirected_edges(&edges);
    let layout = dependency_layout(&known, &edges, &undirected_internal_edges);
    let conductance_certificates = conductance_certificates(
        &known,
        &undirected_internal_edges,
        CONDUCTANCE_DENOMINATOR_POWER,
        CONDUCTANCE_NODE_LIMIT,
    )
    .map_err(DependencyError::Invariant)?;
    let all_ids: BTreeSet<_> = node_kinds.keys().cloned().collect();
    let all_adjacency = adjacency(&all_ids, &edges, false);
    let weak_components = weak_components(&all_ids, &all_adjacency);
    let depth_profile = condensation_depth_profile(&sccs, &internal_adjacency);
    let internal_edges = edges
        .iter()
        .filter(|e| e.classification == DependencyClassification::Internal)
        .count();
    let external_edges = edges
        .iter()
        .filter(|e| e.classification == DependencyClassification::External)
        .count();
    let unresolved_edges = edges.len() - internal_edges - external_edges;
    let evidence_count = edges.iter().map(|e| e.evidence.len()).sum();
    let (manifest_dependencies, manifest_count, unreadable_manifests) =
        inventory_manifests(input)?;
    let non_registry_manifest_dependency_count = manifest_dependencies
        .iter()
        .filter(|d| d.source_kind != ManifestSourceKind::Registry)
        .count();
    let risky_manifest_dependency_count = manifest_dependencies
        .iter()
        .filter(|d| {
            matches!(
                d.source_kind,
                ManifestSourceKind::Path
                    | ManifestSourceKind::Git
                    | ManifestSourceKind::Wildcard
                    | ManifestSourceKind::Unknown
            )
        })
        .count();
    let mut manifest_source_kind_counts = BTreeMap::new();
    for dependency in &manifest_dependencies {
        let label = match dependency.source_kind {
            ManifestSourceKind::Registry => "registry",
            ManifestSourceKind::Path => "path",
            ManifestSourceKind::Git => "git",
            ManifestSourceKind::Workspace => "workspace",
            ManifestSourceKind::Wildcard => "wildcard",
            ManifestSourceKind::Unknown => "unknown",
        };
        *manifest_source_kind_counts
            .entry(label.to_owned())
            .or_insert(0) += 1;
    }

    Ok(DependencyReport {
        root: source_tree.root,
        analyzer: "tree-sitter dependency declarations; graph statistics are structural proxies, not quality measures".to_owned(),
        coverage: DependencyCoverage {
            filesystem_entries_enumerated: source_tree.enumerated,
            source_files_analyzed: source_tree.files.len(),
            unsupported_entries_skipped: source_tree.skipped,
            declarations_extracted: evidence_count,
            unique_edges: edges.len(),
            manifests_analyzed: manifest_count,
            manifests_unreadable: unreadable_manifests.len(),
        },
        limitations: {
            let mut limitations = if unreadable_manifests.is_empty() {
                Vec::new()
            } else {
                vec![format!(
                    "{} manifest(s) could not be read or parsed and are excluded from the manifest inventory; each is named with its reason in unreadable_manifests.",
                    unreadable_manifests.len()
                )]
            };
            limitations.extend(base_limitations());
            limitations
        },
        syntax_error_files,
        unreadable_manifests,
        manifest_dependency_count: manifest_dependencies.len(),
        non_registry_manifest_dependency_count,
        risky_manifest_dependency_count,
        manifest_source_kind_counts,
        manifest_dependencies,
        node_count: nodes.len(), edge_count: edges.len(), internal_edges, external_edges, unresolved_edges,
        nodes, edges, strongly_connected_components: sccs, cycles, weak_components,
        condensation_maximum_depth: depth_profile.as_ref().map(|profile| profile.depth_in_max),
        condensation_depth: depth_profile,
        propagation,
        layout,
        conductance_certificate_node_limit: CONDUCTANCE_NODE_LIMIT,
        conductance_certificate_denominator_power: CONDUCTANCE_DENOMINATOR_POWER,
        conductance_certificates,
    })
}

fn base_limitations() -> Vec<String> {
    vec![
            "Syntax-error trees are analyzed error-tolerantly; declarations from those files may be partial.".to_owned(),
            "Resolution is filesystem-only: no Cargo metadata, Python environment, package.json/tsconfig aliases, JavaScript package exports, Go modules, build tags, generated code, or conditional compilation are interpreted.".to_owned(),
            "Manifest inventory reads only direct declarations; it does not resolve lockfiles, target markers, feature activation, transitive dependencies, or registry defaults beyond the literal manifest syntax.".to_owned(),
            "Rust resolves only mod declarations and direct crate/self/super filesystem module paths; use aliases, re-exports, and extern-prelude names can remain unresolved.".to_owned(),
            "Python resolves only an exact matching .py file or package __init__.py; imported attributes and environment packages are not inferred.".to_owned(),
            "JavaScript and TypeScript resolve only relative paths using an explicit deterministic suffix/index search; bare specifiers are external.".to_owned(),
            "Go imports are external/unresolved without go.mod module-path knowledge; standard-library and third-party imports are not distinguished.".to_owned(),
            "Fan-in, fan-out, components, cycles, and depth are structural proxies and carry no quality verdict or weighting.".to_owned(),
            "Propagation is measured on the file-level internal dependency graph and depends on resolver completeness; exact transitive reachability is omitted above either the 10,000 analyzed-source-file node bound or the 100,000,000 edge-visit work upper bound while direct internal degrees and cycle measures remain available.".to_owned(),
            format!("Conductance certificates are exact for connected components of at least three files through the {CONDUCTANCE_NODE_LIMIT}-file component bound; larger components are reported with size_limit rather than approximated. They provide negative evidence that no sparse cut exists, not a design-quality verdict."),
    ]
}

fn transitive_internal_degrees(adjacency: &[Vec<usize>]) -> ReachabilityComputation {
    let (status, work_upper_bound) = reachability_budget(adjacency);
    if status != ReachabilityStatus::Computed {
        return ReachabilityComputation {
            status,
            work_upper_bound,
            incoming: None,
            outgoing: None,
        };
    }

    let mut incoming = vec![0usize; adjacency.len()];
    let mut outgoing = vec![0usize; adjacency.len()];
    let mut visited = vec![0usize; adjacency.len()];
    let mut generation = 0usize;
    let mut stack = Vec::new();
    for source in 0..adjacency.len() {
        generation += 1;
        visited[source] = generation;
        stack.extend(adjacency[source].iter().copied());
        while let Some(target) = stack.pop() {
            if visited[target] == generation {
                continue;
            }
            visited[target] = generation;
            outgoing[source] += 1;
            incoming[target] += 1;
            stack.extend(adjacency[target].iter().copied());
        }
    }
    ReachabilityComputation {
        status,
        work_upper_bound,
        incoming: Some(incoming),
        outgoing: Some(outgoing),
    }
}

fn reachability_budget(adjacency: &[Vec<usize>]) -> (ReachabilityStatus, Option<usize>) {
    let source_files = adjacency.len();
    let work_upper_bound = adjacency
        .iter()
        .try_fold(0usize, |sum, edges| sum.checked_add(edges.len()))
        .and_then(|internal_unique_edges| internal_unique_edges.checked_add(1))
        .and_then(|per_source| source_files.checked_mul(per_source));
    let status = if source_files == 0 {
        ReachabilityStatus::NotApplicable
    } else if source_files > REACHABILITY_NODE_LIMIT {
        ReachabilityStatus::SizeLimit
    } else if work_upper_bound.is_none_or(|bound| bound > REACHABILITY_WORK_LIMIT) {
        ReachabilityStatus::WorkLimit
    } else {
        ReachabilityStatus::Computed
    };
    (status, work_upper_bound)
}

pub(crate) fn queried_undirected_reachability(
    adjacency: &[Vec<usize>],
    queries: &BTreeSet<(usize, usize)>,
) -> QueriedReachability {
    let (status, work_upper_bound) = reachability_budget(adjacency);
    if status != ReachabilityStatus::Computed {
        return QueriedReachability {
            status,
            work_upper_bound,
            reachable_pairs: None,
        };
    }

    let mut targets_by_source: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for &(left, right) in queries {
        targets_by_source.entry(left).or_default().insert(right);
        targets_by_source.entry(right).or_default().insert(left);
    }
    let mut reachable_pairs = BTreeSet::new();
    let mut visited = vec![0usize; adjacency.len()];
    let mut generation = 0usize;
    let mut stack = Vec::new();
    for (source, targets) in targets_by_source {
        generation += 1;
        visited[source] = generation;
        stack.extend(adjacency[source].iter().copied());
        while let Some(target) = stack.pop() {
            if visited[target] == generation {
                continue;
            }
            visited[target] = generation;
            if targets.contains(&target) {
                reachable_pairs.insert((source.min(target), source.max(target)));
            }
            stack.extend(adjacency[target].iter().copied());
        }
    }
    QueriedReachability {
        status,
        work_upper_bound,
        reachable_pairs: Some(reachable_pairs),
    }
}

fn dependency_propagation(
    source_files: usize,
    cycles: &[Vec<String>],
    internal_weak: &[Vec<String>],
    reachability: &ReachabilityComputation,
) -> DependencyPropagation {
    let cyclic_source_files = cycles.iter().map(Vec::len).sum();
    let largest_cyclic_component_files = cycles.iter().map(Vec::len).max().unwrap_or(0);
    let (reachable_nonself_pairs, possible_nonself_pairs) =
        if reachability.status == ReachabilityStatus::Computed {
            let reachable = reachability.outgoing.as_deref().and_then(|outgoing| {
                outgoing
                    .iter()
                    .try_fold(0usize, |sum, count| sum.checked_add(*count))
            });
            let possible = source_files.checked_mul(source_files.saturating_sub(1));
            (reachable, possible)
        } else {
            (None, None)
        };
    let nonself_propagation_fraction = reachable_nonself_pairs
        .zip(possible_nonself_pairs)
        .and_then(|(reachable, possible)| {
            (possible != 0).then_some(reachable as f64 / possible as f64)
        });
    let source_fraction = |count| (source_files != 0).then_some(count as f64 / source_files as f64);
    let mutually_reachable_pairs = cycles
        .iter()
        .map(|component| (component.len() as u128) * (component.len() as u128 - 1))
        .sum::<u128>();
    let mutual_possible_pairs =
        (source_files >= 2).then(|| (source_files as u128) * (source_files as u128 - 1));
    let mutual_reachability_fraction = mutual_possible_pairs
        .map(|possible| mutually_reachable_pairs as f64 / possible as f64);

    let largest_weak_component_files = internal_weak.iter().map(Vec::len).max().unwrap_or(0);
    let member_component: BTreeMap<&str, usize> = internal_weak
        .iter()
        .enumerate()
        .flat_map(|(index, files)| files.iter().map(move |file| (file.as_str(), index)))
        .collect();
    let mut component_pairs = vec![0u128; internal_weak.len()];
    for component in cycles {
        if let Some(&index) = component.first().and_then(|f| member_component.get(f.as_str())) {
            component_pairs[index] += (component.len() as u128) * (component.len() as u128 - 1);
        }
    }
    // Argmax of pairs/(W*(W-1)) by exact integer cross-multiplication; ties
    // toward the larger component, then earlier (sorted) component order.
    let mut worst: Option<(u128, usize)> = None; // (pairs, files)
    for (index, files) in internal_weak.iter().enumerate() {
        let w = files.len();
        if w < 2 {
            continue;
        }
        let pairs = component_pairs[index];
        let better = match worst {
            None => true,
            Some((best_pairs, best_w)) => {
                let lhs = pairs * (best_w as u128) * (best_w as u128 - 1);
                let rhs = best_pairs * (w as u128) * (w as u128 - 1);
                lhs > rhs || (lhs == rhs && w > best_w)
            }
        };
        if better {
            worst = Some((pairs, w));
        }
    }
    let (worst_pairs, worst_files) = worst.unwrap_or((0, 0));
    let worst_fraction =
        (worst_files >= 2).then(|| worst_pairs as f64 / (worst_files * (worst_files - 1)) as f64);

    DependencyPropagation {
        source_files,
        reachability_status: reachability.status,
        reachability_node_limit: REACHABILITY_NODE_LIMIT,
        reachability_work_limit: REACHABILITY_WORK_LIMIT,
        reachability_work_upper_bound: reachability.work_upper_bound,
        reachable_nonself_pairs,
        possible_nonself_pairs,
        nonself_propagation_fraction,
        mutually_reachable_pairs,
        mutual_possible_pairs,
        mutual_reachability_fraction,
        cyclic_components: cycles.len(),
        cyclic_source_files,
        cyclic_source_file_fraction: source_fraction(cyclic_source_files),
        largest_cyclic_component_files,
        largest_cyclic_component_fraction: source_fraction(largest_cyclic_component_files),
        weak_components: internal_weak.len(),
        largest_weak_component_files,
        largest_weak_component_fraction: source_fraction(largest_weak_component_files),
        worst_weak_component_mutually_reachable_pairs: worst_pairs,
        worst_weak_component_files: worst_files,
        worst_weak_component_mutual_reachability_fraction: worst_fraction,
    }
}

fn internal_undirected_edges(edges: &[DependencyEdge]) -> BTreeSet<(&str, &str)> {
    let mut undirected = BTreeSet::new();
    for edge in edges
        .iter()
        .filter(|edge| edge.classification == DependencyClassification::Internal)
    {
        let (source, target) = (edge.source.as_str(), edge.target.as_str());
        if source != target {
            undirected.insert((source.min(target), source.max(target)));
        }
    }
    undirected
}

fn dependency_layout<'a>(
    analyzed: &BTreeSet<String>,
    edges: &'a [DependencyEdge],
    undirected: &BTreeSet<(&'a str, &'a str)>,
) -> DependencyLayout {
    let internal: Vec<(&str, &str)> = edges
        .iter()
        .filter(|edge| edge.classification == DependencyClassification::Internal)
        .map(|edge| (edge.source.as_str(), edge.target.as_str()))
        .collect();
    let m = undirected.len();
    let top_level = analyzed
        .iter()
        .map(|path| (path.as_str(), top_level_community(path)))
        .collect::<BTreeMap<_, _>>();
    let parent_directory = analyzed
        .iter()
        .map(|path| (path.as_str(), parent_directory_community(path)))
        .collect::<BTreeMap<_, _>>();
    let detected = detected_louvain_partition(analyzed, undirected, m);
    let top_level_partition = layout_partition(
        "top_level",
        analyzed,
        undirected,
        &internal,
        m,
        &top_level,
        false,
    );
    let parent_directory_partition = layout_partition(
        "parent_directory",
        analyzed,
        undirected,
        &internal,
        m,
        &parent_directory,
        false,
    );
    let detected_partition = layout_partition(
        "detected_louvain",
        analyzed,
        undirected,
        &internal,
        m,
        &detected,
        true,
    );
    let headroom = layout_headroom(&detected_partition, &parent_directory_partition);
    let partitions = vec![
        top_level_partition,
        parent_directory_partition,
        detected_partition,
    ];
    DependencyLayout {
        analyzed_files: analyzed.len(),
        internal_undirected_edges: m,
        partitions,
        headroom,
        limitations: vec![
            "The layout graph is file-granularity only; symbol- and declaration-level coupling inside a file is invisible to it.".to_owned(),
            "Import resolution is conservative, so unresolved edges are absent from the graph and heavily unresolved code makes the partition partial.".to_owned(),
            "Modularity Q compares the directory partition to a configuration null model; it says nothing about which partition is correct, only how much this one concentrates edges beyond chance.".to_owned(),
            "A single community scores near zero by construction, and over-splitting inflates cross-community edges; Q is a coordinate, not a target.".to_owned(),
            "Boundary endpoint dispersion counts file-level crossing endpoints, not exported symbols or interface information; a concentrated god façade can therefore look favorable and must be read beside interface evidence.".to_owned(),
            "Boundary direction inconsistency is a coordinate, not a target: bidirectional peer protocols are a legitimate source of quotient-level two-way coupling.".to_owned(),
            "Detected Louvain Q is a heuristic witness lower bound on attainable modularity, not an optimum; the resolution limit can merge small real communities, small graphs make Q noisy, and failure to find headroom proves nothing.".to_owned(),
        ],
    }
}

#[derive(Debug)]
struct LouvainNode {
    original_members: BTreeSet<usize>,
    self_edges: usize,
    degree: usize,
    neighbors: BTreeMap<usize, usize>,
}

fn detected_louvain_partition<'a>(
    analyzed: &'a BTreeSet<String>,
    undirected: &BTreeSet<(&str, &str)>,
    m: usize,
) -> BTreeMap<&'a str, String> {
    let paths = analyzed.iter().map(String::as_str).collect::<Vec<_>>();
    let path_index = paths
        .iter()
        .enumerate()
        .map(|(index, path)| (*path, index))
        .collect::<BTreeMap<_, _>>();
    let mut graph = paths
        .iter()
        .enumerate()
        .map(|(index, _)| {
            (
                index,
                LouvainNode {
                    original_members: BTreeSet::from([index]),
                    self_edges: 0,
                    degree: 0,
                    neighbors: BTreeMap::new(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    for &(left, right) in undirected {
        let (Some(&left_index), Some(&right_index)) =
            (path_index.get(left), path_index.get(right))
        else {
            unreachable!("layout edge endpoints come from the analyzed path set")
        };
        let Some(left_node) = graph.get_mut(&left_index) else {
            unreachable!("analyzed path index must name a Louvain node")
        };
        left_node.neighbors.insert(right_index, 1);
        left_node.degree += 1;
        let Some(right_node) = graph.get_mut(&right_index) else {
            unreachable!("analyzed path index must name a Louvain node")
        };
        right_node.neighbors.insert(left_index, 1);
        right_node.degree += 1;
    }

    loop {
        let (assignments, moved) = louvain_local_moves(&graph, m);
        if !moved {
            break;
        }
        graph = aggregate_louvain_graph(&graph, &assignments);
    }

    let mut detected = BTreeMap::new();
    for (ordinal, node) in graph.values().enumerate() {
        let community = format!("community_{:04}", ordinal + 1);
        for &member in &node.original_members {
            let Some(path) = paths.get(member) else {
                unreachable!("aggregated Louvain members retain original path indices")
            };
            detected.insert(*path, community.clone());
        }
    }
    detected
}

fn louvain_local_moves(
    graph: &BTreeMap<usize, LouvainNode>,
    m: usize,
) -> (BTreeMap<usize, usize>, bool) {
    let mut assignments = graph.keys().map(|&id| (id, id)).collect::<BTreeMap<_, _>>();
    let mut community_degrees = graph
        .iter()
        .map(|(&id, node)| (id, node.degree))
        .collect::<BTreeMap<_, _>>();
    let mut moved_at_all = false;
    loop {
        let mut moved_this_pass = false;
        for (&node_id, node) in graph {
            let Some(&current) = assignments.get(&node_id) else {
                unreachable!("every Louvain node has a current community")
            };
            let mut links_by_community = BTreeMap::<usize, usize>::new();
            for (&neighbor, &weight) in &node.neighbors {
                let Some(&neighbor_community) = assignments.get(&neighbor) else {
                    unreachable!("every Louvain neighbor has a current community")
                };
                *links_by_community.entry(neighbor_community).or_insert(0) += weight;
            }
            let links_current = links_by_community.get(&current).copied().unwrap_or(0);
            let Some(&current_degree) = community_degrees.get(&current) else {
                unreachable!("every active Louvain community has a degree sum")
            };
            let mut best_community = current;
            let mut best_delta = 0i128;
            // BTree iteration plus strict `>` retains the first (smallest-id)
            // community when exact gains tie.
            for (&candidate, &links_candidate) in &links_by_community {
                if candidate == current {
                    continue;
                }
                let Some(&candidate_degree) = community_degrees.get(&candidate) else {
                    unreachable!("neighbor community must have a degree sum")
                };
                let delta = louvain_move_delta(
                    m,
                    node.degree,
                    current_degree,
                    candidate_degree,
                    links_current,
                    links_candidate,
                );
                if delta > best_delta {
                    best_delta = delta;
                    best_community = candidate;
                }
            }
            if best_community != current {
                let Some(degree) = community_degrees.get_mut(&current) else {
                    unreachable!("source Louvain community must have a degree sum")
                };
                *degree -= node.degree;
                let Some(degree) = community_degrees.get_mut(&best_community) else {
                    unreachable!("target Louvain community must have a degree sum")
                };
                *degree += node.degree;
                assignments.insert(node_id, best_community);
                moved_this_pass = true;
                moved_at_all = true;
            }
        }
        if !moved_this_pass {
            break;
        }
    }
    (assignments, moved_at_all)
}

fn louvain_move_delta(
    m: usize,
    node_degree: usize,
    current_degree: usize,
    candidate_degree: usize,
    links_current: usize,
    links_candidate: usize,
) -> i128 {
    // This is the exact change in the existing 4m² modularity numerator.
    // No cross-product or f64 enters the comparison. The caller has already
    // materialized every directed edge in Vec<(&str, &str)>; because a Vec
    // allocation is at most isize::MAX bytes and that element is at least 8
    // bytes, m < 2^60 on 64-bit targets (far smaller on 32-bit). Each term
    // below is bounded by 8m² and their absolute sum by 24m² < i128::MAX.
    let m = m as i128;
    let degree = node_degree as i128;
    let current_degree = current_degree as i128;
    let candidate_degree = candidate_degree as i128;
    4 * m * (links_candidate as i128 - links_current as i128)
        + 2 * degree * (current_degree - candidate_degree)
        - 2 * degree * degree
}

fn aggregate_louvain_graph(
    graph: &BTreeMap<usize, LouvainNode>,
    assignments: &BTreeMap<usize, usize>,
) -> BTreeMap<usize, LouvainNode> {
    let mut assigned_nodes = BTreeMap::<usize, Vec<usize>>::new();
    for (&node, &community) in assignments {
        assigned_nodes.entry(community).or_default().push(node);
    }
    let mut old_to_new = BTreeMap::new();
    let mut aggregated = BTreeMap::new();
    for nodes in assigned_nodes.values() {
        let mut original_members = BTreeSet::new();
        for node_id in nodes {
            let Some(node) = graph.get(node_id) else {
                unreachable!("assigned Louvain node must exist")
            };
            original_members.extend(node.original_members.iter().copied());
        }
        let Some(&new_id) = original_members.first() else {
            unreachable!("a Louvain community cannot be empty")
        };
        for &old_id in nodes {
            old_to_new.insert(old_id, new_id);
        }
        aggregated.insert(
            new_id,
            LouvainNode {
                original_members,
                self_edges: 0,
                degree: 0,
                neighbors: BTreeMap::new(),
            },
        );
    }
    for (&old_id, node) in graph {
        let Some(&new_id) = old_to_new.get(&old_id) else {
            unreachable!("every old Louvain node maps to an aggregate")
        };
        let Some(new_node) = aggregated.get_mut(&new_id) else {
            unreachable!("aggregate Louvain node must exist")
        };
        new_node.self_edges += node.self_edges;
        for (&old_neighbor, &weight) in &node.neighbors {
            if old_id >= old_neighbor {
                continue;
            }
            let Some(&new_neighbor) = old_to_new.get(&old_neighbor) else {
                unreachable!("every old Louvain neighbor maps to an aggregate")
            };
            if new_id == new_neighbor {
                let Some(new_node) = aggregated.get_mut(&new_id) else {
                    unreachable!("aggregate Louvain node must exist")
                };
                new_node.self_edges += weight;
            } else {
                let Some(new_node) = aggregated.get_mut(&new_id) else {
                    unreachable!("aggregate Louvain node must exist")
                };
                *new_node.neighbors.entry(new_neighbor).or_insert(0) += weight;
                let Some(neighbor_node) = aggregated.get_mut(&new_neighbor) else {
                    unreachable!("neighbor aggregate Louvain node must exist")
                };
                *neighbor_node.neighbors.entry(new_id).or_insert(0) += weight;
            }
        }
    }
    for node in aggregated.values_mut() {
        node.degree = 2 * node.self_edges + node.neighbors.values().sum::<usize>();
    }
    aggregated
}

fn layout_headroom(
    detected: &LayoutPartition,
    parent_directory: &LayoutPartition,
) -> LayoutHeadroom {
    // All partitions score the same graph, so their exact Q denominators are
    // identical: subtract numerators directly instead of cross-multiplying.
    // Each numerator has magnitude at most 4m² and the difference at most
    // 8m². The materialized Vec<(&str, &str)> bounds m below 2^60 on 64-bit
    // targets, so the sign-plus-u128 magnitude is wider than this bound even
    // where a signed i128 subtraction of opposite-signed values could fail.
    let exact = detected
        .modularity_numerator
        .zip(parent_directory.modularity_numerator)
        .zip(detected.modularity_denominator)
        .map(|((detected_numerator, directory_numerator), denominator)| {
            let (negative, magnitude) = signed_i128_difference(
                detected_numerator,
                directory_numerator,
            );
            (negative, magnitude, denominator)
        });
    LayoutHeadroom {
        witness_granularity: detected.granularity.clone(),
        baseline_granularity: parent_directory.granularity.clone(),
        modularity_difference: exact.map(|(negative, magnitude, denominator)| {
            let value = magnitude as f64 / denominator as f64;
            if negative { -value } else { value }
        }),
        numerator_negative: exact.map(|(negative, _, _)| negative),
        numerator_magnitude: exact.map(|(_, magnitude, _)| magnitude),
        denominator: exact.map(|(_, _, denominator)| denominator),
    }
}

fn signed_i128_difference(left: i128, right: i128) -> (bool, u128) {
    match (left.is_negative(), right.is_negative()) {
        (false, false) => (
            left < right,
            (left as u128).abs_diff(right as u128),
        ),
        (true, true) => (
            left < right,
            left.unsigned_abs().abs_diff(right.unsigned_abs()),
        ),
        (false, true) => (false, left as u128 + right.unsigned_abs()),
        (true, false) => (true, left.unsigned_abs() + right as u128),
    }
}

#[derive(Default)]
struct CommunityAgg {
    files: usize,
    intra_edges: usize,
    degree_sum: usize,
    out_edges: usize,
    in_edges: usize,
    boundary_in_files: BTreeSet<String>,
    boundary_out_files: BTreeSet<String>,
    boundary_endpoint_counts: BTreeMap<String, usize>,
    top_level_directory_membership: BTreeMap<String, usize>,
}

#[derive(Default)]
struct DirectionPairAgg {
    e_ab: usize,
    e_ba: usize,
    ab_edge_witnesses: BTreeSet<LayoutEdgeWitness>,
    ba_edge_witnesses: BTreeSet<LayoutEdgeWitness>,
}

fn layout_partition(
    granularity: &str,
    analyzed: &BTreeSet<String>,
    undirected: &BTreeSet<(&str, &str)>,
    internal: &[(&str, &str)],
    m: usize,
    community: &BTreeMap<&str, String>,
    annotate_directory_purity: bool,
) -> LayoutPartition {
    let mut aggs: BTreeMap<String, CommunityAgg> = BTreeMap::new();
    for path in analyzed {
        let Some(path_community) = community.get(path.as_str()) else {
            unreachable!("analyzed file must belong to its constructed layout partition")
        };
        let agg = aggs.entry(path_community.clone()).or_default();
        agg.files += 1;
        if annotate_directory_purity {
            *agg.top_level_directory_membership
                .entry(top_level_community(path))
                .or_insert(0) += 1;
        }
    }
    let mut intra_total = 0usize;
    let mut cross_total = 0usize;
    for &(a, b) in undirected {
        let Some(ca) = community.get(a) else {
            unreachable!("internal edge endpoint must belong to the analyzed file partition")
        };
        let Some(cb) = community.get(b) else {
            unreachable!("internal edge endpoint must belong to the analyzed file partition")
        };
        if ca == cb {
            intra_total += 1;
            let Some(agg) = aggs.get_mut(ca) else {
                unreachable!("constructed layout community must have an aggregate")
            };
            agg.intra_edges += 1;
            agg.degree_sum += 2;
        } else {
            cross_total += 1;
            let Some(a_agg) = aggs.get_mut(ca) else {
                unreachable!("constructed layout community must have an aggregate")
            };
            a_agg.degree_sum += 1;
            let Some(b_agg) = aggs.get_mut(cb) else {
                unreachable!("constructed layout community must have an aggregate")
            };
            b_agg.degree_sum += 1;
        }
    }
    let mut direction_pair_aggs: BTreeMap<(String, String), DirectionPairAgg> = BTreeMap::new();
    for &(source, target) in internal {
        let Some(cs) = community.get(source) else {
            unreachable!("internal edge source must belong to the analyzed file partition")
        };
        let Some(ct) = community.get(target) else {
            unreachable!("internal edge target must belong to the analyzed file partition")
        };
        if cs != ct {
            let Some(source_agg) = aggs.get_mut(cs) else {
                unreachable!("internal edge source must belong to the analyzed file partition")
            };
            source_agg.out_edges += 1;
            source_agg.boundary_out_files.insert(source.to_owned());
            *source_agg
                .boundary_endpoint_counts
                .entry(source.to_owned())
                .or_insert(0) += 1;

            let Some(target_agg) = aggs.get_mut(ct) else {
                unreachable!("internal edge target must belong to the analyzed file partition")
            };
            target_agg.in_edges += 1;
            target_agg.boundary_in_files.insert(target.to_owned());
            *target_agg
                .boundary_endpoint_counts
                .entry(target.to_owned())
                .or_insert(0) += 1;

            let (path_a, path_b, a_to_b) = if cs < ct {
                (cs.clone(), ct.clone(), true)
            } else {
                (ct.clone(), cs.clone(), false)
            };
            let pair = direction_pair_aggs.entry((path_a, path_b)).or_default();
            let witness = LayoutEdgeWitness {
                source: source.to_owned(),
                target: target.to_owned(),
            };
            if a_to_b {
                pair.e_ab += 1;
                pair.ab_edge_witnesses.insert(witness);
            } else {
                pair.e_ba += 1;
                pair.ba_edge_witnesses.insert(witness);
            }
        }
    }
    let modularity_numerator = (m != 0).then(|| {
        aggs.values()
            .map(|agg| {
                4 * (m as i128) * (agg.intra_edges as i128) - (agg.degree_sum as i128).pow(2)
            })
            .sum::<i128>()
    });
    let modularity_denominator = (m != 0).then(|| 4 * (m as u128) * (m as u128));
    let modularity = modularity_numerator
        .zip(modularity_denominator)
        .map(|(numerator, denominator)| numerator as f64 / denominator as f64);
    let cross_community_edge_fraction =
        (m != 0).then(|| cross_total as f64 / (intra_total + cross_total) as f64);
    let mut direction_pairs: Vec<LayoutDirectionPair> = direction_pair_aggs
        .into_iter()
        .map(|((path_a, path_b), agg)| {
            let edge_witnesses = if agg.e_ab.min(agg.e_ba) >= 1 {
                let mut selected = BTreeSet::new();
                if let Some(witness) = agg.ab_edge_witnesses.first() {
                    selected.insert(witness.clone());
                }
                if let Some(witness) = agg.ba_edge_witnesses.first() {
                    selected.insert(witness.clone());
                }
                for witness in agg
                    .ab_edge_witnesses
                    .union(&agg.ba_edge_witnesses)
                    .take(5)
                {
                    if selected.len() == 5 {
                        break;
                    }
                    selected.insert(witness.clone());
                }
                selected.into_iter().collect()
            } else {
                Vec::new()
            };
            LayoutDirectionPair {
                path_a,
                path_b,
                e_ab: agg.e_ab,
                e_ba: agg.e_ba,
                edge_witnesses,
            }
        })
        .collect();
    direction_pairs.sort_by(|a, b| {
        b.e_ab
            .min(b.e_ba)
            .cmp(&a.e_ab.min(a.e_ba))
            .then_with(|| a.path_a.cmp(&b.path_a))
            .then_with(|| a.path_b.cmp(&b.path_b))
    });
    let direction_inconsistency_numerator = direction_pairs
        .iter()
        .map(|pair| pair.e_ab.min(pair.e_ba))
        .sum::<usize>();
    let direction_inconsistency_denominator = direction_pairs
        .iter()
        .map(|pair| pair.e_ab + pair.e_ba)
        .sum::<usize>();
    let direction_inconsistency = (direction_inconsistency_denominator != 0).then(|| {
        direction_inconsistency_numerator as f64
            / direction_inconsistency_denominator as f64
    });
    let mut rows: Vec<LayoutCommunity> = aggs
        .into_iter()
        .map(|(path, agg)| {
            let mut endpoints = agg
                .boundary_endpoint_counts
                .into_iter()
                .collect::<Vec<_>>();
            endpoints.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let endpoint_total = agg.out_edges + agg.in_edges;
            let cover_target = endpoint_total - endpoint_total / 10;
            let mut covered = 0usize;
            let boundary_cover_90_file_paths = endpoints
                .into_iter()
                .take_while(|(_, count)| {
                    let include = covered < cover_target;
                    if include {
                        covered += *count;
                    }
                    include
                })
                .map(|(path, _)| path)
                .collect::<Vec<_>>();
            LayoutCommunity {
                path,
                files: agg.files,
                intra_edges: agg.intra_edges,
                out_edges: agg.out_edges,
                in_edges: agg.in_edges,
                boundary_in_files: agg.boundary_in_files.len(),
                boundary_out_files: agg.boundary_out_files.len(),
                boundary_cover_90_files: boundary_cover_90_file_paths.len(),
                boundary_cover_90_file_paths,
                majority_top_level_directory: agg
                    .top_level_directory_membership
                    .iter()
                    .max_by(|left, right| left.1.cmp(right.1).then_with(|| right.0.cmp(left.0)))
                    .map(|(directory, _)| directory.clone()),
                directory_purity_numerator: agg
                    .top_level_directory_membership
                    .values()
                    .max()
                    .copied(),
                directory_purity_denominator: annotate_directory_purity.then_some(agg.files),
                directory_purity: agg
                    .top_level_directory_membership
                    .values()
                    .max()
                    .map(|majority| *majority as f64 / agg.files as f64),
                top_level_directory_membership: agg
                    .top_level_directory_membership
                    .into_iter()
                    .map(|(directory, files)| LayoutDirectoryMembership { directory, files })
                    .collect(),
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        let (a_cross, b_cross) = (a.out_edges + a.in_edges, b.out_edges + b.in_edges);
        b_cross.cmp(&a_cross).then_with(|| a.path.cmp(&b.path))
    });
    LayoutPartition {
        granularity: granularity.to_owned(),
        epistemic_class: if annotate_directory_purity {
            "heuristic_witness"
        } else {
            "fixed_directory_partition"
        }
        .to_owned(),
        communities: rows.len(),
        intra_community_edges: intra_total,
        cross_community_edges: cross_total,
        cross_community_edge_fraction,
        modularity,
        modularity_numerator,
        modularity_denominator,
        direction_inconsistency_numerator,
        direction_inconsistency_denominator,
        direction_inconsistency,
        direction_pairs,
        rows,
    }
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

fn walk(node: Node<'_>, file: &SourceFile, out: &mut Vec<Declaration>) {
    match file.language {
        SourceLanguage::Rust => extract_rust(node, file, out),
        SourceLanguage::Python => extract_python(node, file, out),
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            extract_js(node, file, out)
        }
        SourceLanguage::Go => extract_go(node, file, out),
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, file, out);
    }
}

fn text<'a>(node: Node<'_>, file: &'a SourceFile) -> &'a str {
    std::str::from_utf8(&file.bytes[node.byte_range()]).unwrap_or("")
}
fn line(node: Node<'_>) -> usize {
    node.start_position().row + 1
}
fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches(|c| matches!(c, '\'' | '"' | '`'))
        .to_owned()
}

fn extract_rust(node: Node<'_>, file: &SourceFile, out: &mut Vec<Declaration>) {
    match node.kind() {
        "mod_item" if node.child_by_field_name("body").is_none() => {
            if let Some(name) = node.child_by_field_name("name") {
                out.push(Declaration {
                    line: line(node),
                    specifier: text(name, file).to_owned(),
                    kind: "rust-mod",
                    hint: ResolutionHint::RustModule,
                });
            }
        }
        "use_declaration" => {
            let raw = text(node, file)
                .trim()
                .strip_prefix("use")
                .unwrap_or("")
                .trim()
                .trim_end_matches(';')
                .trim();
            let root = raw
                .split("::{")
                .next()
                .unwrap_or(raw)
                .split(" as ")
                .next()
                .unwrap_or(raw)
                .trim();
            if !root.is_empty() {
                out.push(Declaration {
                    line: line(node),
                    specifier: root.to_owned(),
                    kind: "rust-use",
                    hint: ResolutionHint::RustUse,
                });
            }
        }
        "extern_crate_declaration" => {
            let raw = text(node, file)
                .trim()
                .strip_prefix("extern crate")
                .unwrap_or("")
                .trim()
                .trim_end_matches(';')
                .split_whitespace()
                .next()
                .unwrap_or("");
            if !raw.is_empty() {
                out.push(Declaration {
                    line: line(node),
                    specifier: raw.to_owned(),
                    kind: "rust-extern-crate",
                    hint: ResolutionHint::Package,
                });
            }
        }
        _ => {}
    }
}

fn extract_python(node: Node<'_>, file: &SourceFile, out: &mut Vec<Declaration>) {
    match node.kind() {
        "import_statement" => {
            let raw = text(node, file)
                .trim()
                .strip_prefix("import ")
                .unwrap_or("");
            for item in raw.split(',') {
                let s = item.split_whitespace().next().unwrap_or("");
                if !s.is_empty() {
                    out.push(Declaration {
                        line: line(node),
                        specifier: s.to_owned(),
                        kind: "python-import",
                        hint: ResolutionHint::Package,
                    });
                }
            }
        }
        "import_from_statement" => {
            let raw = text(node, file).trim().strip_prefix("from ").unwrap_or("");
            if let Some((module, _)) = raw.split_once(" import ") {
                out.push(Declaration {
                    line: line(node),
                    specifier: module.trim().to_owned(),
                    kind: "python-from",
                    hint: ResolutionHint::Package,
                });
            }
        }
        _ => {}
    }
}

fn extract_js(node: Node<'_>, file: &SourceFile, out: &mut Vec<Declaration>) {
    match node.kind() {
        "import_statement" | "export_statement" => {
            if let Some(source) = node.child_by_field_name("source") {
                out.push(Declaration {
                    line: line(node),
                    specifier: unquote(text(source, file)),
                    kind: if node.kind() == "import_statement" {
                        "js-import"
                    } else {
                        "js-export-from"
                    },
                    hint: ResolutionHint::Path,
                });
            }
        }
        "call_expression" => {
            let Some(function) = node.child_by_field_name("function") else {
                return;
            };
            let name = text(function, file);
            if name != "require" && name != "import" {
                return;
            }
            let Some(args) = node.child_by_field_name("arguments") else {
                return;
            };
            let Some(arg) = args.named_child(0) else {
                return;
            };
            if matches!(arg.kind(), "string" | "template_string") {
                out.push(Declaration {
                    line: line(node),
                    specifier: unquote(text(arg, file)),
                    kind: if name == "require" {
                        "js-require"
                    } else {
                        "js-dynamic-import"
                    },
                    hint: ResolutionHint::Path,
                });
            }
        }
        _ => {}
    }
}

fn extract_go(node: Node<'_>, file: &SourceFile, out: &mut Vec<Declaration>) {
    if node.kind() == "import_spec" {
        if let Some(path) = node.child_by_field_name("path") {
            out.push(Declaration {
                line: line(node),
                specifier: unquote(text(path, file)),
                kind: "go-import",
                hint: ResolutionHint::GoPackage,
            });
        }
    } else if node.kind() == "import_declaration"
        && let Some(path) = node.child_by_field_name("path")
    {
        out.push(Declaration {
            line: line(node),
            specifier: unquote(text(path, file)),
            kind: "go-import",
            hint: ResolutionHint::GoPackage,
        });
    }
}

fn is_external(d: &Declaration) -> bool {
    match d.hint {
        ResolutionHint::GoPackage => true,
        ResolutionHint::Path => !d.specifier.starts_with('.'),
        ResolutionHint::Package => !d.specifier.starts_with('.'),
        ResolutionHint::RustUse => !matches!(
            d.specifier.split("::").next(),
            Some("crate" | "self" | "super")
        ),
        ResolutionHint::RustModule => false,
    }
}

fn resolve(file: &SourceFile, d: &Declaration, known: &BTreeSet<String>) -> Option<String> {
    match d.hint {
        ResolutionHint::RustModule => resolve_rust_mod(file, &d.specifier, known),
        ResolutionHint::RustUse => resolve_rust_use(file, &d.specifier, known),
        ResolutionHint::Path if d.specifier.starts_with('.') => {
            resolve_js(file, &d.specifier, known)
        }
        ResolutionHint::Package if file.language == SourceLanguage::Python => {
            resolve_python(file, &d.specifier, known)
        }
        _ => None,
    }
}

fn parent(path: &str) -> PathBuf {
    Path::new(path)
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf()
}
fn normalized(path: &Path) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for c in path.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_str()?),
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}
fn first_known(
    candidates: impl IntoIterator<Item = PathBuf>,
    known: &BTreeSet<String>,
) -> Option<String> {
    candidates
        .into_iter()
        .filter_map(|p| normalized(&p))
        .find(|p| known.contains(p))
}
fn resolve_rust_mod(file: &SourceFile, name: &str, known: &BTreeSet<String>) -> Option<String> {
    let dir = parent(&file.path);
    first_known(
        [
            dir.join(format!("{name}.rs")),
            dir.join(name).join("mod.rs"),
        ],
        known,
    )
}
fn resolve_rust_use(file: &SourceFile, value: &str, known: &BTreeSet<String>) -> Option<String> {
    let mut pieces = value
        .split("::")
        .filter(|p| !p.is_empty() && *p != "self")
        .peekable();
    let first = pieces.next()?;
    let mut base = if first == "crate" {
        PathBuf::from("src")
    } else if first == "super" {
        parent(&parent(&file.path).to_string_lossy())
    } else {
        parent(&file.path)
    };
    if first != "crate" && first != "super" {
        base.push(first);
    }
    for piece in pieces {
        base.push(piece);
    }
    let mut candidates = vec![base.with_extension("rs"), base.join("mod.rs")];
    while base.pop() {
        candidates.push(base.with_extension("rs"));
        candidates.push(base.join("mod.rs"));
    }
    first_known(candidates, known)
}
fn resolve_python(file: &SourceFile, value: &str, known: &BTreeSet<String>) -> Option<String> {
    let dots = value.chars().take_while(|c| *c == '.').count();
    let rest = &value[dots..];
    let mut base = if dots == 0 {
        PathBuf::new()
    } else {
        parent(&file.path)
    };
    for _ in 1..dots {
        base.pop();
    }
    if !rest.is_empty() {
        base.extend(rest.split('.'));
    }
    first_known([base.with_extension("py"), base.join("__init__.py")], known)
}
fn resolve_js(file: &SourceFile, value: &str, known: &BTreeSet<String>) -> Option<String> {
    let base = parent(&file.path).join(value);
    let suffixes = ["ts", "tsx", "js", "jsx", "mjs", "cjs"];
    let mut candidates = vec![base.clone()];
    if base.extension().is_none() {
        for suffix in suffixes {
            candidates.push(base.with_extension(suffix));
        }
    }
    for suffix in suffixes {
        candidates.push(base.join(format!("index.{suffix}")));
    }
    first_known(candidates, known)
}

fn adjacency(
    nodes: &BTreeSet<String>,
    edges: &[DependencyEdge],
    internal_only: bool,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut result: BTreeMap<_, _> = nodes
        .iter()
        .cloned()
        .map(|n| (n, BTreeSet::new()))
        .collect();
    for edge in edges {
        if (!internal_only || edge.classification == DependencyClassification::Internal)
            && nodes.contains(&edge.target)
        {
            result
                .entry(edge.source.clone())
                .or_default()
                .insert(edge.target.clone());
        }
    }
    result
}

fn tarjan(
    nodes: &BTreeSet<String>,
    graph: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<Vec<String>> {
    struct State<'a> {
        graph: &'a BTreeMap<String, BTreeSet<String>>,
        next: usize,
        indices: BTreeMap<String, usize>,
        low: BTreeMap<String, usize>,
        stack: Vec<String>,
        on_stack: BTreeSet<String>,
        result: Vec<Vec<String>>,
    }
    fn visit(v: &str, s: &mut State<'_>) {
        let index = s.next;
        s.next += 1;
        s.indices.insert(v.to_owned(), index);
        s.low.insert(v.to_owned(), index);
        s.stack.push(v.to_owned());
        s.on_stack.insert(v.to_owned());
        for w in s.graph.get(v).into_iter().flatten() {
            if !s.indices.contains_key(w) {
                visit(w, s);
                let low = s.low[v].min(s.low[w]);
                s.low.insert(v.to_owned(), low);
            } else if s.on_stack.contains(w) {
                let low = s.low[v].min(s.indices[w]);
                s.low.insert(v.to_owned(), low);
            }
        }
        if s.low[v] == s.indices[v] {
            let mut component = Vec::new();
            loop {
                let w = s.stack.pop().expect("Tarjan stack invariant");
                s.on_stack.remove(&w);
                component.push(w.clone());
                if w == v {
                    break;
                }
            }
            component.sort();
            s.result.push(component);
        }
    }
    let mut s = State {
        graph,
        next: 0,
        indices: BTreeMap::new(),
        low: BTreeMap::new(),
        stack: Vec::new(),
        on_stack: BTreeSet::new(),
        result: Vec::new(),
    };
    for node in nodes {
        if !s.indices.contains_key(node) {
            visit(node, &mut s);
        }
    }
    s.result.sort();
    s.result
}

fn weak_components(
    nodes: &BTreeSet<String>,
    graph: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<Vec<String>> {
    let mut undirected = adjacency(nodes, &[], false);
    for (a, bs) in graph {
        for b in bs {
            undirected.get_mut(a).unwrap().insert(b.clone());
            undirected.get_mut(b).unwrap().insert(a.clone());
        }
    }
    let mut remaining = nodes.clone();
    let mut result = Vec::new();
    while let Some(start) = remaining.iter().next().cloned() {
        let mut queue = VecDeque::from([start]);
        let mut component = Vec::new();
        while let Some(v) = queue.pop_front() {
            if !remaining.remove(&v) {
                continue;
            }
            component.push(v.clone());
            queue.extend(undirected[&v].iter().cloned());
        }
        component.sort();
        result.push(component);
    }
    result.sort();
    result
}

/// Longest-path depths on the SCC-condensation DAG: for each SCC node, the
/// maximum path length (in condensation edges) from any source (`depth_in`)
/// and to any sink (`depth_out`), plus one witness longest path. O(n + m)
/// dynamic programming over a topological order; everything integer.
fn condensation_depth_profile(
    sccs: &[Vec<String>],
    graph: &BTreeMap<String, BTreeSet<String>>,
) -> Option<CondensationDepthProfile> {
    if sccs.is_empty() {
        return None;
    }
    let mut owner = BTreeMap::new();
    for (i, component) in sccs.iter().enumerate() {
        for node in component {
            owner.insert(node.clone(), i);
        }
    }
    let mut dag = vec![BTreeSet::new(); sccs.len()];
    let mut reverse = vec![BTreeSet::new(); sccs.len()];
    let mut indegree = vec![0usize; sccs.len()];
    let mut edges = 0usize;
    for (a, bs) in graph {
        for b in bs {
            let (x, y) = (owner[a], owner[b]);
            if x != y && dag[x].insert(y) {
                reverse[y].insert(x);
                indegree[y] += 1;
                edges += 1;
            }
        }
    }
    // Kahn order; depth_in forward, depth_out via the reversed pass.
    let mut queue: VecDeque<_> = (0..sccs.len()).filter(|i| indegree[*i] == 0).collect();
    let mut order = Vec::with_capacity(sccs.len());
    let mut depth_in = vec![0usize; sccs.len()];
    while let Some(v) = queue.pop_front() {
        order.push(v);
        for &w in &dag[v] {
            depth_in[w] = depth_in[w].max(depth_in[v] + 1);
            indegree[w] -= 1;
            if indegree[w] == 0 {
                queue.push_back(w);
            }
        }
    }
    let mut depth_out = vec![0usize; sccs.len()];
    for &v in order.iter().rev() {
        for &w in &dag[v] {
            depth_out[v] = depth_out[v].max(depth_out[w] + 1);
        }
    }
    // Witness: start from the first node maximizing depth_in + depth_out
    // (deterministic: smallest index), walk back then forward along the DP.
    let start = (0..sccs.len())
        .max_by_key(|&v| (depth_in[v] + depth_out[v], std::cmp::Reverse(v)))
        .expect("nonempty");
    let mut path = VecDeque::from([start]);
    let mut cursor = start;
    while depth_in[cursor] > 0 {
        let previous = reverse[cursor]
            .iter()
            .copied()
            .find(|&p| depth_in[p] + 1 == depth_in[cursor])
            .expect("depth_in predecessor exists");
        path.push_front(previous);
        cursor = previous;
    }
    cursor = start;
    while depth_out[cursor] > 0 {
        let next = dag[cursor]
            .iter()
            .copied()
            .find(|&n| depth_out[n] + 1 == depth_out[cursor])
            .expect("depth_out successor exists");
        path.push_back(next);
        cursor = next;
    }
    // Per-file distributions: every file inherits its SCC's depths.
    let per_file =
        |depths: &[usize]| -> Vec<usize> {
            let mut values: Vec<usize> = sccs
                .iter()
                .enumerate()
                .flat_map(|(i, files)| std::iter::repeat_n(depths[i], files.len()))
                .collect();
            values.sort_unstable();
            values
        };
    let nearest_rank = |sorted: &[usize], hundredths: usize| -> usize {
        let rank = (sorted.len() * hundredths).div_ceil(100).max(1);
        sorted[rank - 1]
    };
    let files_in = per_file(&depth_in);
    let files_out = per_file(&depth_out);
    Some(CondensationDepthProfile {
        condensation_nodes: sccs.len(),
        condensation_edges: edges,
        source_files: files_in.len(),
        depth_in_p50: nearest_rank(&files_in, 50),
        depth_in_p90: nearest_rank(&files_in, 90),
        depth_in_max: *files_in.last().expect("nonempty"),
        depth_out_p50: nearest_rank(&files_out, 50),
        depth_out_p90: nearest_rank(&files_out, 90),
        depth_out_max: *files_out.last().expect("nonempty"),
        longest_path: path
            .into_iter()
            .map(|index| CondensationPathStep {
                file: sccs[index][0].clone(),
                scc_files: sccs[index].len(),
            })
            .collect(),
    })
}

fn inventory_manifests(
    input: &Path,
) -> Result<(Vec<ManifestDependency>, usize, Vec<UnreadableManifest>), DependencyError> {
    let root = if input.is_dir() {
        input
    } else {
        input.parent().unwrap_or_else(|| Path::new("."))
    };
    let mut paths = Vec::new();
    if input.is_file() {
        if is_manifest(input) {
            paths.push(input.to_owned());
        }
    } else {
        let walker = ignore::WalkBuilder::new(input)
            .standard_filters(true)
            .require_git(false)
            .follow_links(false)
            .build();
        for entry in walker {
            let entry = entry.map_err(|error| DependencyError::ManifestParse {
                path: input.to_owned(),
                message: error.to_string(),
            })?;
            if entry.file_type().is_some_and(|t| t.is_file()) && is_manifest(entry.path()) {
                paths.push(entry.into_path());
            }
        }
    }
    paths.sort_by_key(|p| normalized(p.strip_prefix(root).unwrap_or(p)).unwrap_or_default());
    let mut rows = Vec::new();
    let mut unreadable = Vec::new();
    let mut analyzed = 0usize;
    for path in paths {
        let relative = normalized(path.strip_prefix(root).unwrap_or(&path))
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(source) => {
                unreadable.push(UnreadableManifest {
                    path: relative,
                    reason: format!("read error: {source}"),
                });
                continue;
            }
        };
        let parsed = match path.file_name().and_then(|n| n.to_str()).unwrap_or("") {
            "Cargo.toml" => parse_cargo(&path, &relative, &content, &mut rows),
            "package.json" => parse_package_json(&path, &relative, &content, &mut rows),
            "pyproject.toml" => parse_pyproject(&path, &relative, &content, &mut rows),
            "go.mod" => {
                parse_go_mod(&relative, &content, &mut rows);
                Ok(())
            }
            _ => {
                parse_requirements(&relative, &content, &mut rows);
                Ok(())
            }
        };
        match parsed {
            Ok(()) => analyzed += 1,
            Err(error) => unreadable.push(UnreadableManifest {
                path: relative,
                reason: error.to_string(),
            }),
        }
    }
    rows.sort();
    rows.dedup();
    Ok((rows, analyzed, unreadable))
}

fn is_manifest(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    matches!(
        name,
        "Cargo.toml" | "package.json" | "pyproject.toml" | "go.mod"
    ) || (name.starts_with("requirements") && name.ends_with(".txt"))
}

fn parse_toml(path: &Path, content: &str) -> Result<toml::Value, DependencyError> {
    toml::from_str(content).map_err(|e: toml::de::Error| DependencyError::ManifestParse {
        path: path.to_owned(),
        message: e.to_string(),
    })
}

fn parse_cargo(
    path: &Path,
    manifest: &str,
    content: &str,
    out: &mut Vec<ManifestDependency>,
) -> Result<(), DependencyError> {
    let value = parse_toml(path, content)?;
    for (table, scope) in [
        ("dependencies", "runtime"),
        ("dev-dependencies", "development"),
        ("build-dependencies", "build"),
    ] {
        cargo_table(value.get(table), manifest, scope, out);
    }
    if let Some(workspace) = value.get("workspace") {
        cargo_table(workspace.get("dependencies"), manifest, "workspace", out);
    }
    if let Some(targets) = value.get("target").and_then(toml::Value::as_table) {
        for (target, body) in targets {
            for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
                cargo_table(
                    body.get(table),
                    manifest,
                    &format!("target:{target}:{table}"),
                    out,
                );
            }
        }
    }
    Ok(())
}

fn cargo_table(
    value: Option<&toml::Value>,
    manifest: &str,
    scope: &str,
    out: &mut Vec<ManifestDependency>,
) {
    let Some(table) = value.and_then(toml::Value::as_table) else {
        return;
    };
    for (name, value) in table {
        let (requirement, source_kind) = if let Some(version) = value.as_str() {
            (
                version.to_owned(),
                if version == "*" {
                    ManifestSourceKind::Wildcard
                } else {
                    ManifestSourceKind::Registry
                },
            )
        } else if let Some(detail) = value.as_table() {
            let kind = if detail.contains_key("path") {
                ManifestSourceKind::Path
            } else if detail.contains_key("git") {
                ManifestSourceKind::Git
            } else if detail.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
                ManifestSourceKind::Workspace
            } else if detail.contains_key("version") {
                ManifestSourceKind::Registry
            } else {
                ManifestSourceKind::Unknown
            };
            let req = detail
                .get("version")
                .and_then(toml::Value::as_str)
                .or_else(|| detail.get("path").and_then(toml::Value::as_str))
                .or_else(|| detail.get("git").and_then(toml::Value::as_str))
                .unwrap_or("")
                .to_owned();
            (req, kind)
        } else {
            (value.to_string(), ManifestSourceKind::Unknown)
        };
        out.push(manifest_row(
            manifest,
            "cargo",
            scope,
            name,
            &requirement,
            source_kind,
        ));
    }
}

fn parse_package_json(
    path: &Path,
    manifest: &str,
    content: &str,
    out: &mut Vec<ManifestDependency>,
) -> Result<(), DependencyError> {
    let value: serde_json::Value =
        serde_json::from_str(content).map_err(|e| DependencyError::ManifestParse {
            path: path.to_owned(),
            message: e.to_string(),
        })?;
    for (table, scope) in [
        ("dependencies", "runtime"),
        ("devDependencies", "development"),
        ("peerDependencies", "peer"),
        ("optionalDependencies", "optional"),
    ] {
        let Some(deps) = value.get(table).and_then(serde_json::Value::as_object) else {
            continue;
        };
        for (name, req) in deps {
            let req = req.as_str().unwrap_or("");
            out.push(manifest_row(
                manifest,
                "npm",
                scope,
                name,
                req,
                npm_source(req),
            ));
        }
    }
    Ok(())
}

fn npm_source(req: &str) -> ManifestSourceKind {
    if req == "*" {
        ManifestSourceKind::Wildcard
    } else if req.starts_with("workspace:") {
        ManifestSourceKind::Workspace
    } else if req.starts_with("file:") || req.starts_with("link:") {
        ManifestSourceKind::Path
    } else if req.starts_with("git") || req.contains("github.com/") {
        ManifestSourceKind::Git
    } else if req.is_empty() {
        ManifestSourceKind::Unknown
    } else {
        ManifestSourceKind::Registry
    }
}

fn parse_pyproject(
    path: &Path,
    manifest: &str,
    content: &str,
    out: &mut Vec<ManifestDependency>,
) -> Result<(), DependencyError> {
    let value = parse_toml(path, content)?;
    if let Some(deps) = value
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(toml::Value::as_array)
    {
        for dep in deps.iter().filter_map(toml::Value::as_str) {
            let (name, req) = split_python_req(dep);
            out.push(manifest_row(
                manifest,
                "python",
                "runtime",
                name,
                req,
                python_source(req),
            ));
        }
    }
    if let Some(groups) = value
        .get("project")
        .and_then(|p| p.get("optional-dependencies"))
        .and_then(toml::Value::as_table)
    {
        for (group, deps) in groups {
            if let Some(deps) = deps.as_array() {
                for dep in deps.iter().filter_map(toml::Value::as_str) {
                    let (name, req) = split_python_req(dep);
                    out.push(manifest_row(
                        manifest,
                        "python",
                        &format!("optional:{group}"),
                        name,
                        req,
                        python_source(req),
                    ));
                }
            }
        }
    }
    if let Some(deps) = value
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dependencies"))
        .and_then(toml::Value::as_table)
    {
        for (name, req) in deps {
            if name == "python" {
                continue;
            }
            let rendered = req
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| req.to_string());
            let kind = req.as_table().map_or_else(
                || python_source(&rendered),
                |t| {
                    if t.contains_key("path") {
                        ManifestSourceKind::Path
                    } else if t.contains_key("git") {
                        ManifestSourceKind::Git
                    } else {
                        ManifestSourceKind::Unknown
                    }
                },
            );
            out.push(manifest_row(
                manifest, "python", "runtime", name, &rendered, kind,
            ));
        }
    }
    Ok(())
}

fn split_python_req(value: &str) -> (&str, &str) {
    let end = value
        .char_indices()
        .find(|(_, c)| matches!(c, '<' | '>' | '=' | '!' | '~' | ';' | ' ' | '[' | '@'))
        .map_or(value.len(), |(i, _)| i);
    (&value[..end], value[end..].trim())
}
fn python_source(req: &str) -> ManifestSourceKind {
    if req.contains("@ file:") {
        ManifestSourceKind::Path
    } else if req.contains("@ git+") {
        ManifestSourceKind::Git
    } else if req == "*" {
        ManifestSourceKind::Wildcard
    } else {
        ManifestSourceKind::Registry
    }
}

fn parse_requirements(manifest: &str, content: &str, out: &mut Vec<ManifestDependency>) {
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        let value = line.split(" #").next().unwrap_or(line);
        let (name, req) = split_python_req(value);
        if !name.is_empty() {
            out.push(manifest_row(
                manifest,
                "python",
                "runtime",
                name,
                req,
                python_source(value),
            ));
        }
    }
}

fn parse_go_mod(manifest: &str, content: &str, out: &mut Vec<ManifestDependency>) {
    let mut block = false;
    for raw in content.lines() {
        let line = raw.trim();
        if line == "require (" {
            block = true;
            continue;
        }
        if block && line == ")" {
            block = false;
            continue;
        }
        let body = if block {
            line
        } else {
            line.strip_prefix("require ").unwrap_or("")
        };
        if body.is_empty() || body.starts_with("//") {
            continue;
        }
        let mut words = body.split_whitespace();
        if let (Some(name), Some(req)) = (words.next(), words.next()) {
            out.push(manifest_row(
                manifest,
                "go",
                "runtime",
                name,
                req,
                ManifestSourceKind::Registry,
            ));
        }
    }
}

fn manifest_row(
    manifest: &str,
    ecosystem: &str,
    scope: &str,
    name: &str,
    requirement: &str,
    source_kind: ManifestSourceKind,
) -> ManifestDependency {
    ManifestDependency {
        manifest: manifest.to_owned(),
        ecosystem: ecosystem.to_owned(),
        scope: scope.to_owned(),
        name: name.to_owned(),
        requirement: requirement.to_owned(),
        source_kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_skipped_reachability(
        adjacency: &[Vec<usize>],
        expected_status: ReachabilityStatus,
        expected_work_upper_bound: usize,
        expected_serialized_status: &str,
    ) {
        let reachability = transitive_internal_degrees(adjacency);
        assert_eq!(reachability.status, expected_status);
        assert_eq!(
            reachability.work_upper_bound,
            Some(expected_work_upper_bound)
        );
        assert_eq!(reachability.incoming, None);
        assert_eq!(reachability.outgoing, None);

        let serialized =
            serde_json::to_value(dependency_propagation(
                adjacency.len(),
                &[],
                &[],
                &reachability,
            ))
                .expect("serialize skipped reachability profile");
        assert_eq!(
            serialized["reachability_status"],
            expected_serialized_status
        );
        assert_eq!(serialized["reachability_node_limit"], 10_000);
        assert_eq!(serialized["reachability_work_limit"], 100_000_000);
        assert_eq!(
            serialized["reachability_work_upper_bound"],
            expected_work_upper_bound
        );
        assert_eq!(
            serialized["reachable_nonself_pairs"],
            serde_json::Value::Null
        );
        assert_eq!(
            serialized["possible_nonself_pairs"],
            serde_json::Value::Null
        );
        assert_eq!(
            serialized["nonself_propagation_fraction"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn reachability_node_bound_precedes_work_and_preserves_boundary_values() {
        let empty = transitive_internal_degrees(&[]);
        assert_eq!(empty.status, ReachabilityStatus::NotApplicable);
        assert_eq!(empty.work_upper_bound, Some(0));
        assert_eq!(empty.incoming, None);
        assert_eq!(empty.outgoing, None);

        let at_node_limit = vec![Vec::new(); REACHABILITY_NODE_LIMIT];
        let computed = transitive_internal_degrees(&at_node_limit);
        assert_eq!(computed.status, ReachabilityStatus::Computed);
        assert_eq!(computed.work_upper_bound, Some(10_000));
        assert_eq!(
            computed.incoming.as_deref(),
            Some(vec![0; 10_000].as_slice())
        );
        assert_eq!(
            computed.outgoing.as_deref(),
            Some(vec![0; 10_000].as_slice())
        );

        let above_node_limit = vec![Vec::new(); REACHABILITY_NODE_LIMIT + 1];
        assert_skipped_reachability(
            &above_node_limit,
            ReachabilityStatus::SizeLimit,
            10_001,
            "size_limit",
        );
    }

    #[test]
    fn reachability_work_bound_is_inclusive_and_skips_vectors_only_above_limit() {
        let mut at_work_limit = vec![Vec::new(); 1_000];
        at_work_limit[0] = vec![0; 99_999];
        let computed = transitive_internal_degrees(&at_work_limit);
        assert_eq!(computed.status, ReachabilityStatus::Computed);
        assert_eq!(computed.work_upper_bound, Some(100_000_000));
        assert!(computed.incoming.is_some());
        assert!(computed.outgoing.is_some());

        let mut above_work_limit = vec![Vec::new(); 1_000];
        above_work_limit[0] = vec![0; 100_000];
        assert_skipped_reachability(
            &above_work_limit,
            ReachabilityStatus::WorkLimit,
            100_001_000,
            "work_limit",
        );
    }
}
