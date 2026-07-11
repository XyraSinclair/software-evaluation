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
        "verdict",
        "grade",
        "winner",
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
