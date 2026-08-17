//! Evidence-first primitives for software evaluation.
//!
//! The crate keeps artifact observations, evaluator quality, resource cost,
//! and elapsed time separate. It does not define a composite quality score.

pub mod api_surface;
pub mod audit;
pub mod benchmark;
pub mod change_profile;
pub mod cochange;
pub mod cochange_support;
pub mod compare;
pub mod conductance;
pub mod deps;
pub mod discipline;
pub mod duplicates;
pub mod frontier;
pub mod info;
pub mod kernel;
pub mod metrics;
pub mod repo;
pub mod shape;

pub mod service;
pub mod source;
pub mod spectral;
pub mod symbols;
pub mod tests_analysis;
pub mod trophic;
pub mod typespace;
