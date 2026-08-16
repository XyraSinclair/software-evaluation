//! Exact spectral-radius certificates for directed dependency SCCs.

use std::collections::{BTreeMap, BTreeSet};

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{ToPrimitive, Zero};
use serde::Serialize;

pub const SPECTRAL_NODE_LIMIT: usize = 128;
pub const SPECTRAL_MAX_ITERATIONS: usize = 24;
pub const SPECTRAL_WIDTH_DENOMINATOR_POWER: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpectralCertificateStatus {
    /// Exact Collatz-Wielandt bounds were computed and re-verified.
    Certified,
    /// The SCC exceeds the exact-computation node bound.
    SizeLimit,
    /// Fewer than two files; no nontrivial directed cycle exists upstream.
    TrivialSmall,
}

/// Exact Perron-root bounds for one directed internal-file SCC. Decimal-string
/// integers are authoritative beyond fixed-width integer limits; f64 fields
/// are display-only.
#[derive(Debug, Clone, Serialize)]
pub struct SpectralCertificate {
    pub status: SpectralCertificateStatus,
    /// Deterministically ordered files in this SCC.
    pub component_files: Vec<String>,
    /// Unique directed internal edges with both endpoints in this SCC.
    pub internal_edges: usize,
    /// Exact `|S| / n` numerator and denominator; the f64 is display-only.
    pub component_file_numerator: usize,
    pub analyzed_file_denominator: usize,
    pub component_file_fraction: f64,
    /// Reduced exact lower bound on rho(A), serialized as decimal strings.
    pub lower_bound_numerator: Option<String>,
    pub lower_bound_denominator: Option<String>,
    /// Display-only conversion of the exact lower bound.
    pub lower_bound: Option<f64>,
    /// Reduced exact upper bound on rho(A), serialized as decimal strings.
    pub upper_bound_numerator: Option<String>,
    pub upper_bound_denominator: Option<String>,
    /// Display-only conversion of the exact upper bound.
    pub upper_bound: Option<f64>,
    /// Number of exact Collatz-Wielandt power steps evaluated.
    pub iterations_used: usize,
    /// Display-only `upper_bound - lower_bound`.
    pub bound_width: Option<f64>,
    /// True only after one final exact multiplication reproduced the bounds.
    pub bounds_verified: bool,
}

struct ComponentBounds {
    lower: BigRational,
    upper: BigRational,
    iterations_used: usize,
}

pub(crate) fn spectral_certificates(
    analyzed: &BTreeSet<String>,
    directed_edges: &BTreeSet<(&str, &str)>,
    strongly_connected_components: &[Vec<String>],
    node_limit: usize,
    max_iterations: usize,
    width_denominator_power: u32,
) -> Result<Vec<SpectralCertificate>, String> {
    let width_denominator = BigInt::from(1u8) << width_denominator_power;
    let target_width = BigRational::new(BigInt::from(1u8), width_denominator);
    let mut certificates = Vec::with_capacity(strongly_connected_components.len());

    for component_files in strongly_connected_components {
        let members = component_files
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let component_edges = directed_edges
            .iter()
            .copied()
            .filter(|(source, target)| members.contains(source) && members.contains(target))
            .collect::<BTreeSet<_>>();
        let internal_edges = component_edges.len();

        if component_files.len() < 2 {
            certificates.push(certificate_row(
                SpectralCertificateStatus::TrivialSmall,
                component_files.clone(),
                internal_edges,
                analyzed.len(),
                None,
                false,
            ));
            continue;
        }
        if component_files.len() > node_limit {
            certificates.push(certificate_row(
                SpectralCertificateStatus::SizeLimit,
                component_files.clone(),
                internal_edges,
                analyzed.len(),
                None,
                false,
            ));
            continue;
        }

        let computation = compute_component(
            component_files,
            &component_edges,
            max_iterations,
            &target_width,
        )?;
        certificates.push(certificate_row(
            SpectralCertificateStatus::Certified,
            component_files.clone(),
            internal_edges,
            analyzed.len(),
            Some(&computation),
            true,
        ));
    }

    Ok(certificates)
}

fn compute_component(
    component_files: &[String],
    component_edges: &BTreeSet<(&str, &str)>,
    max_iterations: usize,
    target_width: &BigRational,
) -> Result<ComponentBounds, String> {
    if max_iterations == 0 {
        return Err("spectral maximum iterations must be positive".to_owned());
    }
    let dimension = component_files.len();
    let index = component_files
        .iter()
        .enumerate()
        .map(|(position, file)| (file.as_str(), position))
        .collect::<BTreeMap<_, _>>();
    let mut adjacency = vec![Vec::new(); dimension];
    for &(source, target) in component_edges {
        let Some(&source_index) = index.get(source) else {
            return Err(format!(
                "spectral edge source {source:?} is absent from its SCC"
            ));
        };
        let Some(&target_index) = index.get(target) else {
            return Err(format!(
                "spectral edge target {target:?} is absent from its SCC"
            ));
        };
        adjacency[source_index].push(target_index);
    }
    if adjacency.iter().any(Vec::is_empty) {
        return Err(format!(
            "nontrivial SCC beginning {:?} contains a vertex with no outgoing edge",
            component_files.first()
        ));
    }

    let one = BigRational::from_integer(BigInt::from(1u8));
    let mut vector = vec![one.clone(); dimension];
    let mut best: Option<(BigRational, BigRational, Vec<BigRational>, usize)> = None;
    let mut iterations_used = 0usize;

    for iteration in 1..=max_iterations {
        iterations_used = iteration;
        let product = multiply_shifted_adjacency(&adjacency, &vector)?;
        let (lower, upper) = collatz_bounds(&vector, &product, &one)?;
        validate_bounds(component_files, &lower, &upper, &one)?;
        let width = upper.clone() - lower.clone();
        let replace = best.as_ref().is_none_or(|(best_lower, best_upper, _, _)| {
            width < best_upper.clone() - best_lower.clone()
        });
        if replace {
            best = Some((lower, upper, vector.clone(), iteration));
        }
        if width < *target_width {
            break;
        }
        vector = normalize_positive(product)?;
    }

    let Some((lower, upper, witness, _best_iteration)) = best else {
        return Err(format!(
            "spectral iteration for SCC beginning {:?} produced no bounds",
            component_files.first()
        ));
    };
    let final_product = multiply_shifted_adjacency(&adjacency, &witness)?;
    let (verified_lower, verified_upper) = collatz_bounds(&witness, &final_product, &one)?;
    if verified_lower != lower || verified_upper != upper {
        return Err(format!(
            "spectral final multiplication for SCC beginning {:?} did not reproduce its bounds",
            component_files.first()
        ));
    }
    validate_bounds(component_files, &verified_lower, &verified_upper, &one)?;

    if dimension == 2 && component_edges.len() == 2 {
        let exactly_one = lower == one && upper == one;
        if !exactly_one {
            return Err(format!(
                "pure directed two-cycle beginning {:?} had rho bounds {}/{}..{}/{} rather than exactly 1",
                component_files.first(),
                lower.numer(),
                lower.denom(),
                upper.numer(),
                upper.denom(),
            ));
        }
    }

    Ok(ComponentBounds {
        lower,
        upper,
        iterations_used,
    })
}

fn multiply_shifted_adjacency(
    adjacency: &[Vec<usize>],
    vector: &[BigRational],
) -> Result<Vec<BigRational>, String> {
    if adjacency.len() != vector.len() {
        return Err("spectral adjacency and vector dimensions differ".to_owned());
    }
    let mut product = Vec::with_capacity(vector.len());
    for (row, neighbors) in adjacency.iter().enumerate() {
        let mut value = vector[row].clone();
        for &column in neighbors {
            let Some(component) = vector.get(column) else {
                return Err(format!(
                    "spectral adjacency column {column} exceeds dimension {}",
                    vector.len()
                ));
            };
            value += component.clone();
        }
        product.push(value);
    }
    Ok(product)
}

fn collatz_bounds(
    vector: &[BigRational],
    product: &[BigRational],
    one: &BigRational,
) -> Result<(BigRational, BigRational), String> {
    if vector.len() != product.len() || vector.is_empty() {
        return Err(
            "spectral Collatz-Wielandt vectors must be nonempty and equal-sized".to_owned(),
        );
    }
    let mut ratios = vector.iter().zip(product).map(|(value, image)| {
        if value <= &BigRational::zero() {
            return Err("spectral Collatz-Wielandt vector lost strict positivity".to_owned());
        }
        Ok(image.clone() / value.clone() - one.clone())
    });
    let first = ratios
        .next()
        .ok_or_else(|| "spectral Collatz-Wielandt ratio set was empty".to_owned())??;
    let mut lower = first.clone();
    let mut upper = first;
    for ratio in ratios {
        let ratio = ratio?;
        if ratio < lower {
            lower = ratio.clone();
        }
        if ratio > upper {
            upper = ratio;
        }
    }
    Ok((lower, upper))
}

fn normalize_positive(vector: Vec<BigRational>) -> Result<Vec<BigRational>, String> {
    let Some(maximum) = vector.iter().max().cloned() else {
        return Err("spectral normalization vector was empty".to_owned());
    };
    if maximum <= BigRational::zero() {
        return Err("spectral normalization maximum was not positive".to_owned());
    }
    Ok(vector
        .into_iter()
        .map(|value| value / maximum.clone())
        .collect())
}

fn validate_bounds(
    component_files: &[String],
    lower: &BigRational,
    upper: &BigRational,
    one: &BigRational,
) -> Result<(), String> {
    if lower > upper {
        return Err(format!(
            "spectral lower bound exceeded upper bound for SCC beginning {:?}: {}/{} > {}/{}",
            component_files.first(),
            lower.numer(),
            lower.denom(),
            upper.numer(),
            upper.denom(),
        ));
    }
    if lower < one {
        return Err(format!(
            "spectral lower bound for nontrivial SCC beginning {:?} was {}/{} below 1",
            component_files.first(),
            lower.numer(),
            lower.denom(),
        ));
    }
    Ok(())
}

fn certificate_row(
    status: SpectralCertificateStatus,
    component_files: Vec<String>,
    internal_edges: usize,
    analyzed_files: usize,
    computation: Option<&ComponentBounds>,
    bounds_verified: bool,
) -> SpectralCertificate {
    let component_size = component_files.len();
    let (lower_bound_numerator, lower_bound_denominator, lower_bound) =
        exact_fields(computation.map(|bounds| &bounds.lower));
    let (upper_bound_numerator, upper_bound_denominator, upper_bound) =
        exact_fields(computation.map(|bounds| &bounds.upper));
    let bound_width =
        computation.and_then(|bounds| (bounds.upper.clone() - bounds.lower.clone()).to_f64());
    SpectralCertificate {
        status,
        component_files,
        internal_edges,
        component_file_numerator: component_size,
        analyzed_file_denominator: analyzed_files,
        component_file_fraction: if analyzed_files == 0 {
            0.0
        } else {
            component_size as f64 / analyzed_files as f64
        },
        lower_bound_numerator,
        lower_bound_denominator,
        lower_bound,
        upper_bound_numerator,
        upper_bound_denominator,
        upper_bound,
        iterations_used: computation.map_or(0, |bounds| bounds.iterations_used),
        bound_width,
        bounds_verified,
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
