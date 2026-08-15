//! Exact spectral-gap certificates for connected dependency-graph components.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;
use serde::Serialize;

pub const CONDUCTANCE_DENOMINATOR_POWER: u32 = 10;
pub const CONDUCTANCE_NODE_LIMIT: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConductanceCertificateStatus {
    /// Exact inertia bisection completed through the requested dyadic power.
    Certified,
    /// The component exceeds the exact-computation node bound.
    SizeLimit,
    /// Fewer than three files; spectral and conductance bounds are reported n/a.
    TrivialSmall,
}

/// A Cheeger lower-bound certificate for one connected component of the
/// undirected internal file graph. The bound is negative evidence: no cut of
/// the component has conductance below it. It does not establish good design.
/// Exact integers are authoritative; the f64 fields exist only for display.
#[derive(Debug, Clone, Serialize)]
pub struct ConductanceCertificate {
    pub status: ConductanceCertificateStatus,
    /// Deterministically ordered file denominator of this component.
    pub component_files: Vec<String>,
    /// Simple undirected edges with both endpoints in the component.
    pub internal_edges: usize,
    /// Degree-sum volume, exactly twice `internal_edges`.
    pub volume: usize,
    /// Exact `|C| / n` numerator and denominator; the f64 is display only.
    pub component_file_numerator: usize,
    pub analyzed_file_denominator: usize,
    pub component_file_fraction: f64,
    /// Exact certified raw bound `lambda_2 >= a / 2^b` as `(a, b)`.
    pub lambda2_lower_bound_numerator: Option<u64>,
    pub lambda2_lower_bound_denominator_power: Option<u32>,
    pub lambda2_lower_bound: Option<f64>,
    /// Exact Cheeger consequence `phi(C) >= a / 2^(b+1)` as `(a, b+1)`.
    pub conductance_lower_bound_numerator: Option<u64>,
    pub conductance_lower_bound_denominator_power: Option<u32>,
    pub conductance_lower_bound: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Inertia {
    positive: usize,
    zero: usize,
    negative: usize,
}

pub(crate) fn conductance_certificates(
    analyzed: &BTreeSet<String>,
    undirected_edges: &BTreeSet<(&str, &str)>,
    denominator_power: u32,
    node_limit: usize,
) -> Result<Vec<ConductanceCertificate>, String> {
    let denominator = 1u64
        .checked_shl(denominator_power)
        .ok_or_else(|| "conductance denominator power exceeds u64 capacity".to_owned())?;
    let components = connected_components(analyzed, undirected_edges)?;
    let mut certificates = Vec::with_capacity(components.len());

    for component_files in components {
        let member_set = component_files
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let component_edges = undirected_edges
            .iter()
            .copied()
            .filter(|(a, b)| member_set.contains(a) && member_set.contains(b))
            .collect::<BTreeSet<_>>();
        let internal_edges = component_edges.len();
        let volume = internal_edges
            .checked_mul(2)
            .ok_or_else(|| "conductance component volume overflowed usize".to_owned())?;
        let component_size = component_files.len();
        if component_size > node_limit {
            certificates.push(certificate_row(
                ConductanceCertificateStatus::SizeLimit,
                component_files,
                internal_edges,
                volume,
                analyzed.len(),
                None,
                denominator_power,
                denominator,
            ));
            continue;
        }

        let zero_inertia =
            pencil_inertia(&component_files, &component_edges, 0, denominator_power)?;
        let expected = Inertia {
            positive: component_size.saturating_sub(1),
            zero: 1,
            negative: 0,
        };
        if zero_inertia != expected {
            return Err(format!(
                "M(0) inertia for connected component beginning {:?} was (+{}, 0:{}, -{}), expected (+{}, 0:{}, -{})",
                component_files.first(),
                zero_inertia.positive,
                zero_inertia.zero,
                zero_inertia.negative,
                expected.positive,
                expected.zero,
                expected.negative,
            ));
        }

        if component_size < 3 {
            certificates.push(certificate_row(
                ConductanceCertificateStatus::TrivialSmall,
                component_files,
                internal_edges,
                volume,
                analyzed.len(),
                None,
                denominator_power,
                denominator,
            ));
            continue;
        }

        let numerator = bisect_lambda2(
            &component_files,
            &component_edges,
            denominator_power,
            denominator,
            zero_inertia.negative,
        )?;
        certificates.push(certificate_row(
            ConductanceCertificateStatus::Certified,
            component_files,
            internal_edges,
            volume,
            analyzed.len(),
            Some(numerator),
            denominator_power,
            denominator,
        ));
    }
    Ok(certificates)
}

#[allow(clippy::too_many_arguments)]
fn certificate_row(
    status: ConductanceCertificateStatus,
    component_files: Vec<String>,
    internal_edges: usize,
    volume: usize,
    analyzed_files: usize,
    numerator: Option<u64>,
    denominator_power: u32,
    denominator: u64,
) -> ConductanceCertificate {
    let component_size = component_files.len();
    ConductanceCertificate {
        status,
        component_files,
        internal_edges,
        volume,
        component_file_numerator: component_size,
        analyzed_file_denominator: analyzed_files,
        component_file_fraction: component_size as f64 / analyzed_files as f64,
        lambda2_lower_bound_numerator: numerator,
        lambda2_lower_bound_denominator_power: numerator.map(|_| denominator_power),
        lambda2_lower_bound: numerator.map(|value| value as f64 / denominator as f64),
        conductance_lower_bound_numerator: numerator,
        conductance_lower_bound_denominator_power: numerator.map(|_| denominator_power + 1),
        conductance_lower_bound: numerator.map(|value| value as f64 / (2 * denominator) as f64),
    }
}

fn connected_components(
    analyzed: &BTreeSet<String>,
    undirected_edges: &BTreeSet<(&str, &str)>,
) -> Result<Vec<Vec<String>>, String> {
    let mut adjacency = analyzed
        .iter()
        .map(|file| (file.clone(), BTreeSet::<String>::new()))
        .collect::<BTreeMap<_, _>>();
    for &(a, b) in undirected_edges {
        let Some(a_neighbors) = adjacency.get_mut(a) else {
            return Err(format!(
                "undirected internal edge source {a:?} is not analyzed"
            ));
        };
        a_neighbors.insert(b.to_owned());
        let Some(b_neighbors) = adjacency.get_mut(b) else {
            return Err(format!(
                "undirected internal edge target {b:?} is not analyzed"
            ));
        };
        b_neighbors.insert(a.to_owned());
    }

    let mut unseen = analyzed.clone();
    let mut components = Vec::new();
    while let Some(start) = unseen.pop_first() {
        let mut queue = VecDeque::from([start]);
        let mut component = Vec::new();
        while let Some(file) = queue.pop_front() {
            let Some(neighbors) = adjacency.get(&file) else {
                return Err(format!("analyzed file {file:?} has no adjacency row"));
            };
            component.push(file);
            for neighbor in neighbors {
                if unseen.remove(neighbor) {
                    queue.push_back(neighbor.clone());
                }
            }
        }
        component.sort();
        components.push(component);
    }
    components.sort();
    Ok(components)
}

fn bisect_lambda2(
    component_files: &[String],
    component_edges: &BTreeSet<(&str, &str)>,
    denominator_power: u32,
    denominator: u64,
    zero_negative_count: usize,
) -> Result<u64, String> {
    let maximum = denominator
        .checked_mul(2)
        .ok_or_else(|| "conductance bisection endpoint overflowed u64".to_owned())?;
    let mut probes = BTreeMap::from([(0, zero_negative_count)]);
    let maximum_negative = probe_negative_count(
        component_files,
        component_edges,
        maximum,
        denominator_power,
        &mut probes,
    )?;
    if maximum_negative <= 1 {
        return Err(format!(
            "M(2) for a connected component of {} files had only {maximum_negative} eigenvalue(s) below 2",
            component_files.len(),
        ));
    }

    let mut certified = 0u64;
    let mut rejected = maximum;
    while rejected - certified > 1 {
        let probe = certified + (rejected - certified) / 2;
        let negative = probe_negative_count(
            component_files,
            component_edges,
            probe,
            denominator_power,
            &mut probes,
        )?;
        if negative == 0 {
            return Err(format!(
                "positive conductance probe {probe}/2^{denominator_power} had no negative eigenvalue"
            ));
        }
        if negative == 1 {
            certified = probe;
        } else {
            rejected = probe;
        }
    }
    Ok(certified)
}

fn probe_negative_count(
    component_files: &[String],
    component_edges: &BTreeSet<(&str, &str)>,
    numerator: u64,
    denominator_power: u32,
    probes: &mut BTreeMap<u64, usize>,
) -> Result<usize, String> {
    if let Some(count) = probes.get(&numerator) {
        return Ok(*count);
    }
    let count = pencil_inertia(
        component_files,
        component_edges,
        numerator,
        denominator_power,
    )?
    .negative;
    probes.insert(numerator, count);

    let mut previous = None;
    for (&probe, &negative) in probes.iter() {
        if let Some((previous_probe, previous_negative)) = previous
            && negative < previous_negative
        {
            return Err(format!(
                "negative inertia count decreased from {previous_negative} at {previous_probe}/2^{denominator_power} to {negative} at {probe}/2^{denominator_power}"
            ));
        }
        previous = Some((probe, negative));
    }
    Ok(count)
}

fn pencil_inertia(
    component_files: &[String],
    component_edges: &BTreeSet<(&str, &str)>,
    numerator: u64,
    denominator_power: u32,
) -> Result<Inertia, String> {
    let denominator = 1u64
        .checked_shl(denominator_power)
        .ok_or_else(|| "conductance denominator power exceeds u64 capacity".to_owned())?;
    let scale = BigInt::from(denominator);
    let shift = &scale - BigInt::from(numerator);
    let n = component_files.len();
    let mut matrix = vec![vec![BigRational::from_integer(BigInt::from(0)); n]; n];
    let index = component_files
        .iter()
        .enumerate()
        .map(|(position, file)| (file.as_str(), position))
        .collect::<BTreeMap<_, _>>();
    let mut degrees = vec![0usize; n];

    for &(a, b) in component_edges {
        let Some(&i) = index.get(a) else {
            return Err(format!(
                "component edge endpoint {a:?} is absent from its component"
            ));
        };
        let Some(&j) = index.get(b) else {
            return Err(format!(
                "component edge endpoint {b:?} is absent from its component"
            ));
        };
        degrees[i] = degrees[i]
            .checked_add(1)
            .ok_or_else(|| "component degree overflowed usize".to_owned())?;
        degrees[j] = degrees[j]
            .checked_add(1)
            .ok_or_else(|| "component degree overflowed usize".to_owned())?;
        let off_diagonal = BigRational::from_integer(-scale.clone());
        matrix[i][j] = off_diagonal.clone();
        matrix[j][i] = off_diagonal;
    }
    for (i, degree) in degrees.into_iter().enumerate() {
        matrix[i][i] = BigRational::from_integer(&shift * BigInt::from(degree));
    }
    Ok(exact_symmetric_inertia(matrix))
}

/// Exact symmetric congruence elimination. The deterministic first nonzero
/// diagonal is a 1x1 pivot. If every remaining diagonal is zero, the first
/// nonzero off-diagonal supplies a 2x2 block with negative determinant and
/// therefore one positive and one negative eigenvalue.
fn exact_symmetric_inertia(mut matrix: Vec<Vec<BigRational>>) -> Inertia {
    let n = matrix.len();
    let mut inertia = Inertia {
        positive: 0,
        zero: 0,
        negative: 0,
    };
    let mut pivot_start = 0usize;

    while pivot_start < n {
        if let Some(diagonal) =
            (pivot_start..n).find(|&i| rational_sign(&matrix[i][i]) != Sign::NoSign)
        {
            swap_symmetric(&mut matrix, pivot_start, diagonal);
            let pivot = matrix[pivot_start][pivot_start].clone();
            match rational_sign(&pivot) {
                Sign::Plus => inertia.positive += 1,
                Sign::Minus => inertia.negative += 1,
                Sign::NoSign => unreachable!("selected diagonal pivot must be nonzero"),
            }
            for i in (pivot_start + 1)..n {
                for j in i..n {
                    let correction = matrix[i][pivot_start].clone()
                        * matrix[j][pivot_start].clone()
                        / pivot.clone();
                    let value = matrix[i][j].clone() - correction;
                    matrix[i][j] = value.clone();
                    matrix[j][i] = value;
                }
            }
            pivot_start += 1;
            continue;
        }

        let mut off_diagonal = None;
        'search: for (i, row) in matrix.iter().enumerate().skip(pivot_start) {
            for (j, value) in row.iter().enumerate().skip(i + 1) {
                if rational_sign(value) != Sign::NoSign {
                    off_diagonal = Some((i, j));
                    break 'search;
                }
            }
        }
        let Some((first, second)) = off_diagonal else {
            inertia.zero += n - pivot_start;
            break;
        };
        swap_symmetric(&mut matrix, pivot_start, first);
        swap_symmetric(&mut matrix, pivot_start + 1, second);
        let cross = matrix[pivot_start][pivot_start + 1].clone();
        inertia.positive += 1;
        inertia.negative += 1;
        for i in (pivot_start + 2)..n {
            for j in i..n {
                let correction = (matrix[i][pivot_start].clone()
                    * matrix[j][pivot_start + 1].clone()
                    + matrix[i][pivot_start + 1].clone() * matrix[j][pivot_start].clone())
                    / cross.clone();
                let value = matrix[i][j].clone() - correction;
                matrix[i][j] = value.clone();
                matrix[j][i] = value;
            }
        }
        pivot_start += 2;
    }
    inertia
}

fn rational_sign(value: &BigRational) -> Sign {
    value.numer().sign()
}

fn swap_symmetric(matrix: &mut [Vec<BigRational>], left: usize, right: usize) {
    if left == right {
        return;
    }
    matrix.swap(left, right);
    for row in matrix {
        row.swap(left, right);
    }
}
