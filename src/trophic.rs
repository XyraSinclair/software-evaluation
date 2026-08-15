//! Exact improved trophic incoherence for directed dependency components.

use std::collections::{BTreeMap, BTreeSet};

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{ToPrimitive, Zero};
use serde::Serialize;

use crate::conductance::{connected_components, solve_exact_linear_system};

pub const TROPHIC_NODE_LIMIT: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrophicIncoherenceStatus {
    Computed,
    SizeLimit,
    TrivialNoEdges,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrophicPathSelfCheckStatus {
    Verified,
    SkippedNoTwoNodePath,
}

/// Exact improved trophic incoherence for one weakly connected component of
/// the directed internal file graph. Decimal-string integers are authoritative
/// and remain serializable beyond u128/i128; the f64 field is display-only.
#[derive(Debug, Clone, Serialize)]
pub struct TrophicIncoherenceComponent {
    /// Exact-computation outcome for this weak component.
    pub status: TrophicIncoherenceStatus,
    /// Deterministically ordered file denominator of this component.
    pub component_files: Vec<String>,
    /// Unique directed internal file edges in this component; the `m` in F0.
    pub directed_edges: usize,
    /// Exact `|C| / n` numerator and denominator; the f64 is display-only.
    pub component_file_numerator: usize,
    pub analyzed_file_denominator: usize,
    pub component_file_fraction: f64,
    /// Reduced exact F0 numerator as a decimal string, including beyond u128.
    pub f0_numerator: Option<String>,
    /// Reduced exact F0 denominator as a decimal string, including beyond u128.
    pub f0_denominator: Option<String>,
    /// Display-only conversion of the exact numerator/denominator.
    pub f0: Option<f64>,
    /// Files belonging to a nontrivial SCC or carrying a self-loop.
    pub files_in_cycle: usize,
    /// Largest directed SCC in this weak component, including singleton SCCs.
    pub largest_scc_files: usize,
}

/// Repository-level edge-weighted mean over all directed internal edges. If
/// any edge-bearing component exceeds the node bound, the exact mean is not
/// guessed. Decimal-string integers are authoritative; f64 is display-only.
#[derive(Debug, Clone, Serialize)]
pub struct TrophicIncoherenceReport {
    /// `SizeLimit` makes the repository F0 unavailable rather than partial.
    pub status: TrophicIncoherenceStatus,
    /// Maximum files permitted in an exactly solved edge-bearing component.
    pub node_limit: usize,
    /// All internal weak components, including isolated analyzed files.
    pub weak_components: usize,
    /// Weak components whose directed-edge denominator is nonzero.
    pub edge_bearing_components: usize,
    /// Edge-bearing components solved exactly within the node bound.
    pub computed_components: usize,
    /// Repository denominator: all unique directed internal file edges.
    pub total_directed_edges: usize,
    /// Directed edges represented by exact component solves.
    pub computed_directed_edges: usize,
    /// Reduced exact edge-weighted F0 numerator as a decimal string.
    pub f0_numerator: Option<String>,
    /// Reduced exact edge-weighted F0 denominator as a decimal string.
    pub f0_denominator: Option<String>,
    /// Display-only conversion of the exact numerator/denominator.
    pub f0: Option<f64>,
    /// Computed components whose exact `Lambda h - v` residual was zero.
    pub residuals_verified: usize,
    /// Exact F0=0 identity check when a two-file, one-edge component exists.
    pub two_node_path_self_check: TrophicPathSelfCheckStatus,
    /// Deterministic weak-component rows.
    pub components: Vec<TrophicIncoherenceComponent>,
}

pub(crate) fn trophic_incoherence(
    analyzed: &BTreeSet<String>,
    directed_edges: &BTreeSet<(&str, &str)>,
    strongly_connected_components: &[Vec<String>],
    node_limit: usize,
) -> Result<TrophicIncoherenceReport, String> {
    let undirected_edges = directed_edges
        .iter()
        .filter(|(source, target)| source != target)
        .map(|&(source, target)| {
            if source < target {
                (source, target)
            } else {
                (target, source)
            }
        })
        .collect::<BTreeSet<_>>();
    let weak_components = connected_components(analyzed, &undirected_edges)?;
    let self_loops = directed_edges
        .iter()
        .filter(|(source, target)| source == target)
        .map(|(source, _)| *source)
        .collect::<BTreeSet<_>>();
    let mut components = Vec::with_capacity(weak_components.len());
    let mut total_directed_edges = 0usize;
    let mut computed_directed_edges = 0usize;
    let mut edge_bearing_components = 0usize;
    let mut computed_components = 0usize;
    let mut residuals_verified = 0usize;
    let mut total_squared_error = BigRational::zero();
    let mut size_limited = false;
    let mut two_node_path_found = false;

    for component_files in weak_components {
        let members = component_files
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let component_edges = directed_edges
            .iter()
            .copied()
            .filter(|(source, target)| members.contains(source) && members.contains(target))
            .collect::<BTreeSet<_>>();
        let directed_edge_count = component_edges.len();
        total_directed_edges = total_directed_edges
            .checked_add(directed_edge_count)
            .ok_or_else(|| "trophic directed-edge total overflowed usize".to_owned())?;
        let (files_in_cycle, largest_scc_files) =
            component_loop_facts(&members, strongly_connected_components, &self_loops);

        if directed_edge_count == 0 {
            components.push(component_row(
                TrophicIncoherenceStatus::TrivialNoEdges,
                component_files,
                directed_edge_count,
                analyzed.len(),
                None,
                files_in_cycle,
                largest_scc_files,
            ));
            continue;
        }
        edge_bearing_components = edge_bearing_components
            .checked_add(1)
            .ok_or_else(|| "trophic component count overflowed usize".to_owned())?;

        if component_files.len() > node_limit {
            size_limited = true;
            components.push(component_row(
                TrophicIncoherenceStatus::SizeLimit,
                component_files,
                directed_edge_count,
                analyzed.len(),
                None,
                files_in_cycle,
                largest_scc_files,
            ));
            continue;
        }

        let computation = compute_component(&component_files, &component_edges)?;
        if component_files.len() == 2 && directed_edge_count == 1 {
            two_node_path_found = true;
            if !computation.f0.is_zero() {
                return Err(format!(
                    "two-node one-edge trophic path beginning {:?} had F0={}/{} rather than 0",
                    component_files.first(),
                    computation.f0.numer(),
                    computation.f0.denom(),
                ));
            }
        }
        total_squared_error += computation.squared_error;
        computed_directed_edges = computed_directed_edges
            .checked_add(directed_edge_count)
            .ok_or_else(|| "trophic computed-edge total overflowed usize".to_owned())?;
        computed_components = computed_components
            .checked_add(1)
            .ok_or_else(|| "trophic computed-component count overflowed usize".to_owned())?;
        residuals_verified = residuals_verified
            .checked_add(1)
            .ok_or_else(|| "trophic residual-check count overflowed usize".to_owned())?;
        components.push(component_row(
            TrophicIncoherenceStatus::Computed,
            component_files,
            directed_edge_count,
            analyzed.len(),
            Some(&computation.f0),
            files_in_cycle,
            largest_scc_files,
        ));
    }

    if total_directed_edges != directed_edges.len() {
        return Err(format!(
            "trophic components accounted for {total_directed_edges} directed edges, expected {}",
            directed_edges.len()
        ));
    }
    let status = if total_directed_edges == 0 {
        TrophicIncoherenceStatus::TrivialNoEdges
    } else if size_limited {
        TrophicIncoherenceStatus::SizeLimit
    } else {
        TrophicIncoherenceStatus::Computed
    };
    let repository_f0 = if status == TrophicIncoherenceStatus::Computed {
        Some(total_squared_error / BigRational::from_integer(BigInt::from(total_directed_edges)))
    } else {
        None
    };
    let (f0_numerator, f0_denominator, f0) = exact_fields(repository_f0.as_ref());

    Ok(TrophicIncoherenceReport {
        status,
        node_limit,
        weak_components: components.len(),
        edge_bearing_components,
        computed_components,
        total_directed_edges,
        computed_directed_edges,
        f0_numerator,
        f0_denominator,
        f0,
        residuals_verified,
        two_node_path_self_check: if two_node_path_found {
            TrophicPathSelfCheckStatus::Verified
        } else {
            TrophicPathSelfCheckStatus::SkippedNoTwoNodePath
        },
        components,
    })
}

struct ComponentComputation {
    f0: BigRational,
    squared_error: BigRational,
}

fn compute_component(
    component_files: &[String],
    directed_edges: &BTreeSet<(&str, &str)>,
) -> Result<ComponentComputation, String> {
    let dimension = component_files.len();
    let index = component_files
        .iter()
        .enumerate()
        .map(|(position, file)| (file.as_str(), position))
        .collect::<BTreeMap<_, _>>();
    let mut laplacian = vec![vec![BigRational::zero(); dimension]; dimension];
    let mut imbalance = vec![BigRational::zero(); dimension];
    let one = BigRational::from_integer(BigInt::from(1));

    for &(source, target) in directed_edges {
        let Some(&source_index) = index.get(source) else {
            return Err(format!(
                "trophic edge source {source:?} is absent from its component"
            ));
        };
        let Some(&target_index) = index.get(target) else {
            return Err(format!(
                "trophic edge target {target:?} is absent from its component"
            ));
        };
        laplacian[source_index][source_index] += one.clone();
        laplacian[target_index][target_index] += one.clone();
        laplacian[source_index][target_index] -= one.clone();
        laplacian[target_index][source_index] -= one.clone();
        imbalance[source_index] -= one.clone();
        imbalance[target_index] += one.clone();
    }

    let mut levels = vec![BigRational::zero(); dimension];
    if dimension > 1 {
        let reduced = laplacian
            .iter()
            .skip(1)
            .map(|row| row.iter().skip(1).cloned().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let reduced_imbalance = imbalance.iter().skip(1).cloned().collect::<Vec<_>>();
        let solution = solve_exact_linear_system(reduced, reduced_imbalance)?;
        for (level, value) in levels.iter_mut().skip(1).zip(solution) {
            *level = value;
        }
    }

    for row in 0..dimension {
        let residual = laplacian[row]
            .iter()
            .zip(&levels)
            .fold(BigRational::zero(), |sum, (coefficient, level)| {
                sum + coefficient.clone() * level.clone()
            })
            - imbalance[row].clone();
        if !residual.is_zero() {
            return Err(format!(
                "trophic residual for {:?} was {}/{} rather than 0",
                component_files.get(row),
                residual.numer(),
                residual.denom(),
            ));
        }
    }

    let mut squared_error = BigRational::zero();
    for &(source, target) in directed_edges {
        let source_index = index[source];
        let target_index = index[target];
        let error = levels[target_index].clone() - levels[source_index].clone() - one.clone();
        squared_error += error.clone() * error;
    }
    let f0 = squared_error.clone() / BigRational::from_integer(BigInt::from(directed_edges.len()));
    if f0 < BigRational::zero() || f0 > one {
        return Err(format!(
            "trophic F0 for component beginning {:?} was {}/{} outside [0,1]",
            component_files.first(),
            f0.numer(),
            f0.denom(),
        ));
    }
    Ok(ComponentComputation { f0, squared_error })
}

fn component_loop_facts(
    members: &BTreeSet<&str>,
    strongly_connected_components: &[Vec<String>],
    self_loops: &BTreeSet<&str>,
) -> (usize, usize) {
    let mut files_in_cycle = 0usize;
    let mut largest_scc_files = 0usize;
    for scc in strongly_connected_components {
        if scc
            .first()
            .is_some_and(|file| members.contains(file.as_str()))
        {
            largest_scc_files = largest_scc_files.max(scc.len());
            if scc.len() > 1
                || scc
                    .first()
                    .is_some_and(|file| self_loops.contains(file.as_str()))
            {
                files_in_cycle += scc.len();
            }
        }
    }
    (files_in_cycle, largest_scc_files)
}

#[allow(clippy::too_many_arguments)]
fn component_row(
    status: TrophicIncoherenceStatus,
    component_files: Vec<String>,
    directed_edges: usize,
    analyzed_files: usize,
    f0: Option<&BigRational>,
    files_in_cycle: usize,
    largest_scc_files: usize,
) -> TrophicIncoherenceComponent {
    let component_size = component_files.len();
    let (f0_numerator, f0_denominator, f0_display) = exact_fields(f0);
    TrophicIncoherenceComponent {
        status,
        component_files,
        directed_edges,
        component_file_numerator: component_size,
        analyzed_file_denominator: analyzed_files,
        component_file_fraction: if analyzed_files == 0 {
            0.0
        } else {
            component_size as f64 / analyzed_files as f64
        },
        f0_numerator,
        f0_denominator,
        f0: f0_display,
        files_in_cycle,
        largest_scc_files,
    }
}

fn exact_fields(value: Option<&BigRational>) -> (Option<String>, Option<String>, Option<f64>) {
    match value {
        Some(value) => (
            Some(value.numer().to_string()),
            Some(value.denom().to_string()),
            value.to_f64(),
        ),
        None => (None, None, None),
    }
}
