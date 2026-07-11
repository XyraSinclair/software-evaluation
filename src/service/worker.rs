use crate::{
    api_surface, deps,
    duplicates::{self, DuplicateConfig},
    metrics::{self, MetricSort},
    service::dto::{CompactResult, InstrumentResult, InstrumentState, RepositoryProvenance},
    tests_analysis,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, process::Stdio, time::Duration};
use thiserror::Error;
use tokio::{io::AsyncReadExt, process::Command, time::timeout};

const OUTPUT_LIMIT: usize = 256 * 1024;
fn failed(analyzer: &str) -> InstrumentResult {
    InstrumentResult {
        analyzer: analyzer.into(),
        state: InstrumentState::Failed,
        coverage: json!({}),
        observations: json!({}),
        limitations: vec![],
        error: Some("instrument analysis failed".into()),
    }
}
fn value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or_else(|_| json!({}))
}
fn dependency_hotspots(
    nodes: &[deps::DependencyNode],
    primary: fn(&deps::DependencyNode) -> Option<usize>,
    secondary: fn(&deps::DependencyNode) -> Option<usize>,
) -> Vec<Value> {
    let mut nodes = nodes
        .iter()
        .filter(|node| primary(node).is_some_and(|count| count != 0))
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        primary(right)
            .cmp(&primary(left))
            .then_with(|| secondary(right).cmp(&secondary(left)))
            .then_with(|| left.id.cmp(&right.id))
    });
    nodes
        .into_iter()
        .take(5)
        .map(|node| {
            json!({
                "path": node.id,
                "internal_fan_in": node.direct_internal_in_degree,
                "internal_fan_out": node.direct_internal_out_degree,
                "transitive_internal_fan_in": node.transitive_internal_in_count,
                "transitive_internal_fan_out": node.transitive_internal_out_count,
            })
        })
        .collect()
}

pub fn analyze(root: &Path, provenance: RepositoryProvenance) -> CompactResult {
    let mut instruments = BTreeMap::new();
    let m = match metrics::analyze_path(root) {
        Ok(r) => {
            let functions=metrics::rank_functions(&r,MetricSort::Cognitive,5).into_iter().map(|x|json!({"path":x.path,"name":x.name,"start_line":x.start_line,"cognitive":x.cognitive,"cyclomatic":x.cyclomatic,"sloc":x.sloc})).collect::<Vec<_>>();
            let files=metrics::rank_files(&r,MetricSort::Cognitive,5).into_iter().map(|x|json!({"path":x.path,"cognitive":x.cognitive,"cyclomatic":x.cyclomatic,"sloc":x.sloc})).collect::<Vec<_>>();
            InstrumentResult {
                analyzer: r.analyzer,
                state: InstrumentState::Complete,
                coverage: value(&r.coverage),
                observations: json!({"files":r.summary.files,"functions":r.summary.functions,"sloc":r.summary.sloc,"cognitive":r.summary.cognitive,"cyclomatic":r.summary.cyclomatic,"rates":r.rates,"function_tails":{"cognitive":r.distributions.cognitive,"cyclomatic":r.distributions.cyclomatic,"sloc":r.distributions.sloc},"hotspots":{"functions":functions,"files":files}}),
                limitations: r.limitations,
                error: None,
            }
        }
        Err(_) => failed("static-metrics"),
    };
    instruments.insert("metrics".into(), m);
    let d = match deps::analyze_dependencies(root) {
        Ok(r) => {
            let direct_internal_in_hotspots = dependency_hotspots(
                &r.nodes,
                |node| node.direct_internal_in_degree,
                |node| node.direct_internal_out_degree,
            );
            let direct_internal_out_hotspots = dependency_hotspots(
                &r.nodes,
                |node| node.direct_internal_out_degree,
                |node| node.direct_internal_in_degree,
            );
            let transitive_internal_in_hotspots = dependency_hotspots(
                &r.nodes,
                |node| node.transitive_internal_in_count,
                |node| node.transitive_internal_out_count,
            );
            let transitive_internal_out_hotspots = dependency_hotspots(
                &r.nodes,
                |node| node.transitive_internal_out_count,
                |node| node.transitive_internal_in_count,
            );
            InstrumentResult {
                analyzer: r.analyzer,
                state: InstrumentState::Complete,
                coverage: value(&r.coverage),
                observations: json!({"manifest_count":r.coverage.manifests_analyzed,"manifest_dependencies":r.manifest_dependency_count,"nodes":r.node_count,"edges":r.edge_count,"internal_edges":r.internal_edges,"external_edges":r.external_edges,"unresolved_edges":r.unresolved_edges,"cycles":r.cycles.len(),"components":r.weak_components.len(),"condensation_depth":r.condensation_maximum_depth,"dependency_structure":{"propagation":r.propagation,"direct_internal_in_hotspots":direct_internal_in_hotspots,"direct_internal_out_hotspots":direct_internal_out_hotspots,"transitive_internal_in_hotspots":transitive_internal_in_hotspots,"transitive_internal_out_hotspots":transitive_internal_out_hotspots}}),
                limitations: r.limitations,
                error: None,
            }
        }
        Err(_) => failed("dependency-structure"),
    };
    instruments.insert("dependencies".into(), d);
    let d = match duplicates::analyze_duplicates(root, &DuplicateConfig::default()) {
        Ok(r) => {
            let saturated = r.groups.len() >= r.config.max_groups;
            let retained = if r.coverage.considered_tokens == 0 {
                None
            } else {
                Some(r.totals.duplicated_tokens as f64 / r.coverage.considered_tokens as f64)
            };
            InstrumentResult {
                analyzer: r.analyzer,
                state: InstrumentState::Complete,
                coverage: value(&r.coverage),
                observations: json!({"config":r.config,"totals":r.totals,"retained_token_ratio":retained,"saturated":saturated}),
                limitations: r.limitations,
                error: None,
            }
        }
        Err(_) => failed("duplicate-structure"),
    };
    instruments.insert("duplicates".into(), d);
    let a = match api_surface::analyze_api_surface(root) {
        Ok(r) => {
            let ratio = if r.counts.public_symbols == 0 {
                None
            } else {
                Some(r.counts.documented_symbols as f64 / r.counts.public_symbols as f64)
            };
            InstrumentResult {
                analyzer: r.analyzer,
                state: InstrumentState::Complete,
                coverage: value(&r.coverage),
                observations: json!({"counts":r.counts,"documented_ratio":ratio}),
                limitations: r.limitations,
                error: None,
            }
        }
        Err(_) => failed("api-surface"),
    };
    instruments.insert("api".into(), a);
    let t = match tests_analysis::analyze_tests(root) {
        Ok(r) => InstrumentResult {
            analyzer: r.analyzer,
            state: InstrumentState::Complete,
            coverage: value(&r.coverage),
            observations: json!({"test_coverage":r.coverage}),
            limitations: r.limitations,
            error: None,
        },
        Err(_) => failed("test-structure"),
    };
    instruments.insert("tests".into(), t);
    let complete = instruments
        .values()
        .filter(|x| x.state == InstrumentState::Complete)
        .count();
    CompactResult {
        repository: provenance,
        instruments,
        completed_instruments: complete,
        failed_instruments: 5 - complete,
    }
}

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("worker timed out")]
    Timeout,
    #[error("worker failed")]
    Failed,
    #[error("worker output was invalid")]
    InvalidOutput,
}
pub async fn run_child(
    exe: &Path,
    root: &Path,
    p: RepositoryProvenance,
    deadline: Duration,
) -> Result<CompactResult, WorkerError> {
    let mut child = Command::new(exe)
        .arg("worker")
        .arg("--repository-root")
        .arg(root)
        .arg("--full-name")
        .arg(&p.full_name)
        .arg("--repository-id")
        .arg(p.repository_id.to_string())
        .arg("--commit")
        .arg(&p.commit)
        .env_clear()
        .env("RAYON_NUM_THREADS", "2")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| WorkerError::Failed)?;
    let stdout = child.stdout.take().ok_or(WorkerError::Failed)?;
    let task = tokio::spawn(async move {
        let mut out = Vec::new();
        stdout
            .take((OUTPUT_LIMIT + 1) as u64)
            .read_to_end(&mut out)
            .await
            .map(|_| out)
    });
    let status = match timeout(deadline, child.wait()).await {
        Ok(Ok(s)) => s,
        Ok(Err(_)) => return Err(WorkerError::Failed),
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(WorkerError::Timeout);
        }
    };
    if !status.success() {
        return Err(WorkerError::Failed);
    }
    let out = task
        .await
        .map_err(|_| WorkerError::Failed)?
        .map_err(|_| WorkerError::Failed)?;
    if out.len() > OUTPUT_LIMIT {
        return Err(WorkerError::InvalidOutput);
    }
    serde_json::from_slice(&out).map_err(|_| WorkerError::InvalidOutput)
}
