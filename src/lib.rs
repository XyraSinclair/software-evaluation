//! Evidence-first primitives for software evaluation.
//!
//! The crate keeps artifact observations, evaluator quality, resource cost,
//! and elapsed time separate. It does not define a composite quality score.

pub mod audit;
pub mod compare;
pub mod info;
pub mod kernel;
pub mod metrics;
pub mod repo;
