use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;
use software_evaluation::service::dto::{InstrumentState, RepositoryProvenance};
use software_evaluation::service::worker::analyze;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn provenance() -> RepositoryProvenance {
    RepositoryProvenance {
        full_name: "fixture-owner/fixture-repo".to_owned(),
        repository_id: 42,
        commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        cached: false,
    }
}

fn assert_forbidden_keys_absent(value: &Value) {
    const FORBIDDEN: &[&str] = &[
        "score",
        "quality_score",
        "quality",
        "verdict",
        "grade",
        "winner",
        "threshold",
        "rating",
        "rank",
        "direction",
        "html",
        "raw_source",
        "local_path",
        "upstream_body",
        "stderr",
        "command_input",
    ];

    match value {
        Value::Object(object) => {
            for key in object.keys() {
                assert!(
                    !FORBIDDEN.contains(&key.as_str()),
                    "compact result exposed forbidden key {key:?}"
                );
            }
            for child in object.values() {
                assert_forbidden_keys_absent(child);
            }
        }
        Value::Array(items) => items.iter().for_each(assert_forbidden_keys_absent),
        _ => {}
    }
}

#[test]
fn worker_reports_all_five_instruments_as_compact_independent_evidence() {
    let result = analyze(&fixture("service_worker_complete"), provenance());
    let names = result
        .instruments
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();

    assert_eq!(
        names,
        BTreeSet::from(["api", "dependencies", "duplicates", "metrics", "tests"])
    );
    assert_eq!(result.completed_instruments, 5);
    assert_eq!(result.failed_instruments, 0);
    for (name, instrument) in &result.instruments {
        assert_eq!(instrument.state, InstrumentState::Complete, "{name}");
        assert!(
            !instrument.analyzer.is_empty(),
            "{name} must identify its analyzer"
        );
        assert!(
            instrument.coverage.is_object(),
            "{name} coverage must be structured"
        );
        assert!(
            instrument.observations.is_object(),
            "{name} observations must be structured"
        );
        assert!(
            !instrument.limitations.is_empty(),
            "{name} limitations must remain visible"
        );
        assert_eq!(instrument.error, None, "{name}");
    }

    let json = serde_json::to_value(&result).expect("serialize compact worker result");
    assert_forbidden_keys_absent(&json);
    let encoded = serde_json::to_vec(&result).expect("encode compact worker result");
    assert!(
        encoded.len() <= 256 * 1024,
        "compact result was {} bytes",
        encoded.len()
    );

    let dependencies = &result.instruments["dependencies"].observations;
    let structure = dependencies
        .get("dependency_structure")
        .and_then(Value::as_object)
        .expect("dependencies must expose a nested dependency_structure object");
    assert_eq!(
        structure
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "direct_internal_in_hotspots",
            "direct_internal_out_hotspots",
            "propagation",
            "transitive_internal_in_hotspots",
            "transitive_internal_out_hotspots",
        ]),
        "dependency_structure schema is closed"
    );

    let propagation = &structure["propagation"];
    assert_eq!(propagation["source_files"], 3);
    assert_eq!(propagation["reachability_status"], "computed");
    assert_eq!(propagation["reachability_node_limit"], 10_000);
    assert_eq!(propagation["reachability_work_limit"], 100_000_000);
    assert_eq!(propagation["reachability_work_upper_bound"], 15);
    assert_eq!(propagation["reachable_nonself_pairs"], 4);
    assert_eq!(propagation["possible_nonself_pairs"], 6);
    assert_eq!(propagation["nonself_propagation_fraction"], 2.0 / 3.0);
    assert_eq!(propagation["cyclic_components"], 1);
    assert_eq!(propagation["cyclic_source_files"], 2);
    assert_eq!(propagation["cyclic_source_file_fraction"], 2.0 / 3.0);
    assert_eq!(propagation["largest_cyclic_component_files"], 2);
    assert_eq!(propagation["largest_cyclic_component_fraction"], 2.0 / 3.0);

    let alpha = serde_json::json!({
        "path": "src/alpha.rs",
        "internal_fan_in": 2,
        "internal_fan_out": 1,
        "transitive_internal_fan_in": 2,
        "transitive_internal_fan_out": 1
    });
    let beta = serde_json::json!({
        "path": "src/beta.rs",
        "internal_fan_in": 2,
        "internal_fan_out": 1,
        "transitive_internal_fan_in": 2,
        "transitive_internal_fan_out": 1
    });
    let lib = serde_json::json!({
        "path": "src/lib.rs",
        "internal_fan_in": 0,
        "internal_fan_out": 2,
        "transitive_internal_fan_in": 0,
        "transitive_internal_fan_out": 2
    });
    let expected_hotspots = [
        (
            "direct_internal_in_hotspots",
            serde_json::json!([alpha.clone(), beta.clone()]),
        ),
        (
            "direct_internal_out_hotspots",
            serde_json::json!([lib.clone(), alpha.clone(), beta.clone()]),
        ),
        (
            "transitive_internal_in_hotspots",
            serde_json::json!([alpha.clone(), beta.clone()]),
        ),
        (
            "transitive_internal_out_hotspots",
            serde_json::json!([lib, alpha, beta]),
        ),
    ];
    for (key, expected) in expected_hotspots {
        assert_eq!(
            structure[key], expected,
            "{key} must be count-descending then path-ascending"
        );
        let rows = structure[key].as_array().expect("hotspots must be arrays");
        assert!(rows.len() <= 5, "{key} must be bounded to five rows");
        let primary = match key {
            "direct_internal_in_hotspots" => "internal_fan_in",
            "direct_internal_out_hotspots" => "internal_fan_out",
            "transitive_internal_in_hotspots" => "transitive_internal_fan_in",
            "transitive_internal_out_hotspots" => "transitive_internal_fan_out",
            _ => unreachable!(),
        };
        assert!(
            rows.iter()
                .all(|row| row[primary].as_u64().is_some_and(|count| count > 0)),
            "{key} must not emit zero-primary rows"
        );
    }
}

#[test]
fn worker_preserves_four_successes_when_dependency_manifest_is_malformed() {
    let result = analyze(&fixture("service_worker_partial"), provenance());

    assert_eq!(result.completed_instruments, 4);
    assert_eq!(result.failed_instruments, 1);
    let dependencies = &result.instruments["dependencies"];
    assert_eq!(dependencies.state, InstrumentState::Failed);
    assert_eq!(
        dependencies.error.as_deref(),
        Some("instrument analysis failed")
    );
    for name in ["metrics", "duplicates", "api", "tests"] {
        assert_eq!(
            result.instruments[name].state,
            InstrumentState::Complete,
            "{name}"
        );
    }

    let json = serde_json::to_value(&result).expect("serialize partial compact result");
    assert_forbidden_keys_absent(&json);
}
