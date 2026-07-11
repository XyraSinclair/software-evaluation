use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use software_evaluation::deps::{
    DependencyClassification, DependencyNodeKind, ManifestDependency, ManifestSourceKind,
    analyze_dependencies,
};
use tempfile::TempDir;

struct DependencyFixture {
    directory: TempDir,
}

impl DependencyFixture {
    fn new() -> Self {
        let directory = TempDir::new().expect("temporary dependency repository");
        let root = directory.path();
        let files = [
            (
                "src/main.rs",
                "mod alpha;\nuse crate::alpha;\nextern crate serde;\n",
            ),
            ("src/alpha.rs", "use crate::main;\nuse crate::leaf;\n"),
            ("src/leaf.rs", "use crate::tail;\n"),
            ("src/tail.rs", "mod ghost;\n"),
            (
                "web/app.ts",
                "import { helper } from \"./helper.js\";\nexport { helper } from \"./helper.js\";\nimport React from \"react\";\nimport \"./missing\";\n",
            ),
            ("web/helper.js", "export const helper = 1;\n"),
            (
                "py/a.py",
                "import py.b\nfrom py.b import value\nimport requests\nfrom .missing import value\n",
            ),
            ("py/b.py", "value = 1\n"),
            (
                "go/main.go",
                "package main\nimport \"fmt\"\nimport \"example.com/remote\"\nfunc main() { fmt.Println() }\n",
            ),
            ("broken.rs", "fn broken( {\n"),
            (
                "Cargo.toml",
                "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\nlocal = { path = \"../local\" }\ngitdep = { git = \"https://example.com/gitdep.git\" }\nshared = { workspace = true }\nany = \"*\"\n",
            ),
            (
                "package.json",
                "{\"dependencies\":{\"lodash\":\"^4\"},\"devDependencies\":{\"local-ui\":\"file:../ui\"}}\n",
            ),
            (
                "pyproject.toml",
                "[project]\nname = \"fixture\"\nversion = \"0.1.0\"\ndependencies = [\"requests>=2\", \"mypkg @ git+https://example.com/mypkg.git\"]\n\n[project.optional-dependencies]\ndev = [\"pytest>=8\"]\n",
            ),
            (
                "requirements.txt",
                "flask==3.0\nlocalreq @ file:../localreq\n",
            ),
            (
                "go.mod",
                "module example.com/fixture\n\ngo 1.23\n\nrequire example.com/library v1.2.3\n",
            ),
        ];
        for (relative, contents) in files.into_iter().rev() {
            write_file(root, relative, contents);
        }
        Self { directory }
    }

    fn path(&self) -> &Path {
        self.directory.path()
    }
}

fn write_file(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create dependency fixture parent");
    }
    fs::write(&path, contents)
        .unwrap_or_else(|error| panic!("write fixture {}: {error}", path.display()));
}

fn analyze_source_fixture(files: &[(&str, &str)]) -> software_evaluation::deps::DependencyReport {
    let directory = TempDir::new().expect("temporary dependency graph repository");
    for (path, contents) in files {
        write_file(directory.path(), path, contents);
    }
    analyze_dependencies(directory.path()).expect("analyze dependency graph fixture")
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

fn run_cli(root: &Path, trailing: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seval"))
        .arg("deps")
        .arg(root.as_os_str())
        .args(trailing)
        .output()
        .expect("run seval deps")
}

#[test]
fn polyglot_graph_preserves_evidence_topology_and_direct_manifest_observations() {
    let fixture = DependencyFixture::new();

    // Oracle commitments are deliberately defined before production is invoked. The graph has
    // six internal edges: a two-node Rust cycle, a two-edge tail from that cycle, and one edge
    // each for Python and TypeScript. Repeated declarations add evidence, never graph degree.
    let expected_internal_edges = vec![
        ("py/a.py", "py/b.py"),
        ("src/alpha.rs", "src/leaf.rs"),
        ("src/alpha.rs", "src/main.rs"),
        ("src/leaf.rs", "src/tail.rs"),
        ("src/main.rs", "src/alpha.rs"),
        ("web/app.ts", "web/helper.js"),
    ];
    let expected_evidence = vec![
        (
            "py/a.py",
            "py/b.py",
            vec![(1, "py.b", "python-import"), (2, "py.b", "python-from")],
        ),
        (
            "src/alpha.rs",
            "src/leaf.rs",
            vec![(2, "crate::leaf", "rust-use")],
        ),
        (
            "src/alpha.rs",
            "src/main.rs",
            vec![(1, "crate::main", "rust-use")],
        ),
        (
            "src/leaf.rs",
            "src/tail.rs",
            vec![(1, "crate::tail", "rust-use")],
        ),
        (
            "src/main.rs",
            "src/alpha.rs",
            vec![(1, "alpha", "rust-mod"), (2, "crate::alpha", "rust-use")],
        ),
        (
            "web/app.ts",
            "web/helper.js",
            vec![
                (1, "./helper.js", "js-import"),
                (2, "./helper.js", "js-export-from"),
            ],
        ),
    ];
    let expected_external_edges = vec![
        ("go/main.go", "external:example.com/remote"),
        ("go/main.go", "external:fmt"),
        ("py/a.py", "external:requests"),
        ("src/main.rs", "external:serde"),
        ("web/app.ts", "external:react"),
    ];
    let expected_unresolved_edges = vec![
        ("py/a.py", "unresolved:.missing"),
        ("src/tail.rs", "unresolved:ghost"),
        ("web/app.ts", "unresolved:./missing"),
    ];
    let expected_degrees = BTreeMap::from([
        ("broken.rs", (0, 0, DependencyNodeKind::AnalyzedFile)),
        (
            "external:example.com/remote",
            (1, 0, DependencyNodeKind::ExternalSpecifier),
        ),
        (
            "external:fmt",
            (1, 0, DependencyNodeKind::ExternalSpecifier),
        ),
        (
            "external:react",
            (1, 0, DependencyNodeKind::ExternalSpecifier),
        ),
        (
            "external:requests",
            (1, 0, DependencyNodeKind::ExternalSpecifier),
        ),
        (
            "external:serde",
            (1, 0, DependencyNodeKind::ExternalSpecifier),
        ),
        ("go/main.go", (0, 2, DependencyNodeKind::AnalyzedFile)),
        ("py/a.py", (0, 3, DependencyNodeKind::AnalyzedFile)),
        ("py/b.py", (1, 0, DependencyNodeKind::AnalyzedFile)),
        ("src/alpha.rs", (1, 2, DependencyNodeKind::AnalyzedFile)),
        ("src/leaf.rs", (1, 1, DependencyNodeKind::AnalyzedFile)),
        ("src/main.rs", (1, 2, DependencyNodeKind::AnalyzedFile)),
        ("src/tail.rs", (1, 1, DependencyNodeKind::AnalyzedFile)),
        (
            "unresolved:./missing",
            (1, 0, DependencyNodeKind::UnresolvedSpecifier),
        ),
        (
            "unresolved:.missing",
            (1, 0, DependencyNodeKind::UnresolvedSpecifier),
        ),
        (
            "unresolved:ghost",
            (1, 0, DependencyNodeKind::UnresolvedSpecifier),
        ),
        ("web/app.ts", (0, 3, DependencyNodeKind::AnalyzedFile)),
        ("web/helper.js", (1, 0, DependencyNodeKind::AnalyzedFile)),
    ]);
    let expected_sccs = vec![
        vec!["broken.rs"],
        vec!["go/main.go"],
        vec!["py/a.py"],
        vec!["py/b.py"],
        vec!["src/alpha.rs", "src/main.rs"],
        vec!["src/leaf.rs"],
        vec!["src/tail.rs"],
        vec!["web/app.ts"],
        vec!["web/helper.js"],
    ];
    let expected_weak_components = vec![
        vec!["broken.rs"],
        vec!["external:example.com/remote", "external:fmt", "go/main.go"],
        vec![
            "external:react",
            "unresolved:./missing",
            "web/app.ts",
            "web/helper.js",
        ],
        vec![
            "external:requests",
            "py/a.py",
            "py/b.py",
            "unresolved:.missing",
        ],
        vec![
            "external:serde",
            "src/alpha.rs",
            "src/leaf.rs",
            "src/main.rs",
            "src/tail.rs",
            "unresolved:ghost",
        ],
    ];
    let expected_manifests = vec![
        manifest_row(
            "Cargo.toml",
            "cargo",
            "runtime",
            "any",
            "*",
            ManifestSourceKind::Wildcard,
        ),
        manifest_row(
            "Cargo.toml",
            "cargo",
            "runtime",
            "gitdep",
            "https://example.com/gitdep.git",
            ManifestSourceKind::Git,
        ),
        manifest_row(
            "Cargo.toml",
            "cargo",
            "runtime",
            "local",
            "../local",
            ManifestSourceKind::Path,
        ),
        manifest_row(
            "Cargo.toml",
            "cargo",
            "runtime",
            "serde",
            "1",
            ManifestSourceKind::Registry,
        ),
        manifest_row(
            "Cargo.toml",
            "cargo",
            "runtime",
            "shared",
            "",
            ManifestSourceKind::Workspace,
        ),
        manifest_row(
            "go.mod",
            "go",
            "runtime",
            "example.com/library",
            "v1.2.3",
            ManifestSourceKind::Registry,
        ),
        manifest_row(
            "package.json",
            "npm",
            "development",
            "local-ui",
            "file:../ui",
            ManifestSourceKind::Path,
        ),
        manifest_row(
            "package.json",
            "npm",
            "runtime",
            "lodash",
            "^4",
            ManifestSourceKind::Registry,
        ),
        manifest_row(
            "pyproject.toml",
            "python",
            "optional:dev",
            "pytest",
            ">=8",
            ManifestSourceKind::Registry,
        ),
        manifest_row(
            "pyproject.toml",
            "python",
            "runtime",
            "mypkg",
            "@ git+https://example.com/mypkg.git",
            ManifestSourceKind::Git,
        ),
        manifest_row(
            "pyproject.toml",
            "python",
            "runtime",
            "requests",
            ">=2",
            ManifestSourceKind::Registry,
        ),
        manifest_row(
            "requirements.txt",
            "python",
            "runtime",
            "flask",
            "==3.0",
            ManifestSourceKind::Registry,
        ),
        manifest_row(
            "requirements.txt",
            "python",
            "runtime",
            "localreq",
            "@ file:../localreq",
            ManifestSourceKind::Path,
        ),
    ];
    let expected_source_counts = BTreeMap::from([
        ("git".to_owned(), 2),
        ("path".to_owned(), 3),
        ("registry".to_owned(), 6),
        ("wildcard".to_owned(), 1),
        ("workspace".to_owned(), 1),
    ]);

    let report =
        analyze_dependencies(fixture.path()).expect("analyze seeded dependency repository");

    assert_eq!(report.coverage.filesystem_entries_enumerated, 15);
    assert_eq!(report.coverage.source_files_analyzed, 10);
    assert_eq!(report.coverage.unsupported_entries_skipped, 5);
    assert_eq!(report.coverage.declarations_extracted, 17);
    assert_eq!(report.coverage.unique_edges, 14);
    assert_eq!(report.coverage.manifests_analyzed, 5);
    assert_eq!(
        report.syntax_error_files, 1,
        "malformed Rust must be counted while nine valid files are not"
    );

    let edges_of = |classification| {
        report
            .edges
            .iter()
            .filter(|edge| edge.classification == classification)
            .map(|edge| (edge.source.as_str(), edge.target.as_str()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        edges_of(DependencyClassification::Internal),
        expected_internal_edges
    );
    assert_eq!(
        edges_of(DependencyClassification::External),
        expected_external_edges
    );
    assert_eq!(
        edges_of(DependencyClassification::Unresolved),
        expected_unresolved_edges
    );
    assert_eq!(
        (
            report.internal_edges,
            report.external_edges,
            report.unresolved_edges
        ),
        (6, 5, 3)
    );

    let actual_evidence = report
        .edges
        .iter()
        .filter(|edge| edge.classification == DependencyClassification::Internal)
        .map(|edge| {
            (
                edge.source.as_str(),
                edge.target.as_str(),
                edge.evidence
                    .iter()
                    .map(|item| {
                        assert_eq!(item.source_path, edge.source);
                        assert_eq!(item.resolved_target.as_deref(), Some(edge.target.as_str()));
                        (item.line, item.raw_specifier.as_str(), item.kind.as_str())
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual_evidence, expected_evidence,
        "evidence must retain lexical edge order and source-line order"
    );

    let actual_degrees = report
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), (node.fan_in, node.fan_out, node.kind)))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(actual_degrees, expected_degrees);
    assert_eq!(report.node_count, 18);
    assert_eq!(report.edge_count, 14);

    let actual_sccs = report
        .strongly_connected_components
        .iter()
        .map(|component| component.iter().map(String::as_str).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(actual_sccs, expected_sccs);
    assert_eq!(
        report.cycles,
        vec![vec!["src/alpha.rs".to_owned(), "src/main.rs".to_owned()]]
    );
    let actual_weak_components = report
        .weak_components
        .iter()
        .map(|component| component.iter().map(String::as_str).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(actual_weak_components, expected_weak_components);
    assert_eq!(report.condensation_maximum_depth, Some(2));

    assert_eq!(report.manifest_dependencies, expected_manifests);
    assert_eq!(report.manifest_dependency_count, 13);
    assert_eq!(report.non_registry_manifest_dependency_count, 7);
    assert_eq!(report.risky_manifest_dependency_count, 6);
    assert_eq!(report.manifest_source_kind_counts, expected_source_counts);
}

#[test]
fn internal_degree_and_propagation_oracle_is_exact_for_polyglot_graph() {
    let fixture = DependencyFixture::new();

    // Hand-computed over the ten analyzed files and the six unique internal edges. External and
    // unresolved targets do not participate. The Rust cycle reaches its two members plus its
    // two-node tail; Python and JavaScript each contribute one reachable ordered pair.
    let expected_internal_counts = BTreeMap::from([
        ("broken.rs", (Some(0), Some(0), Some(0), Some(0))),
        ("external:react", (None, None, None, None)),
        ("go/main.go", (Some(0), Some(0), Some(0), Some(0))),
        ("py/a.py", (Some(0), Some(1), Some(0), Some(1))),
        ("py/b.py", (Some(1), Some(0), Some(1), Some(0))),
        ("src/alpha.rs", (Some(1), Some(2), Some(1), Some(3))),
        ("src/leaf.rs", (Some(1), Some(1), Some(2), Some(1))),
        ("src/main.rs", (Some(1), Some(1), Some(1), Some(3))),
        ("src/tail.rs", (Some(1), Some(0), Some(3), Some(0))),
        ("unresolved:ghost", (None, None, None, None)),
        ("web/app.ts", (Some(0), Some(1), Some(0), Some(1))),
        ("web/helper.js", (Some(1), Some(0), Some(1), Some(0))),
    ]);

    let report = analyze_dependencies(fixture.path()).expect("analyze polyglot graph oracle");
    let actual_internal_counts = report
        .nodes
        .iter()
        .filter(|node| expected_internal_counts.contains_key(node.id.as_str()))
        .map(|node| {
            (
                node.id.as_str(),
                (
                    node.direct_internal_in_degree,
                    node.direct_internal_out_degree,
                    node.transitive_internal_in_count,
                    node.transitive_internal_out_count,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(actual_internal_counts, expected_internal_counts);

    let propagation = serde_json::to_value(&report.propagation).expect("serialize propagation");
    assert_eq!(propagation["source_files"], 10);
    assert_eq!(propagation["reachability_status"], "computed");
    assert_eq!(propagation["reachability_node_limit"], 10_000);
    assert_eq!(propagation["reachability_work_limit"], 100_000_000);
    assert_eq!(propagation["reachability_work_upper_bound"], 70);
    assert_eq!(propagation["reachable_nonself_pairs"], 9);
    assert_eq!(propagation["possible_nonself_pairs"], 90);
    assert_eq!(propagation["nonself_propagation_fraction"], 0.1);
    assert_eq!(propagation["cyclic_components"], 1);
    assert_eq!(propagation["cyclic_source_files"], 2);
    assert_eq!(propagation["cyclic_source_file_fraction"], 0.2);
    assert_eq!(propagation["largest_cyclic_component_files"], 2);
    assert_eq!(propagation["largest_cyclic_component_fraction"], 0.2);
}

#[test]
fn smallest_dependency_graphs_prove_null_zero_cycle_and_dedup_semantics() {
    let cases = [
        ("zero", analyze_source_fixture(&[])),
        (
            "one-edge",
            analyze_source_fixture(&[
                ("a.js", "import './b.js';\nimport './b.js';\n"),
                ("b.js", "export const b = 1;\n"),
            ]),
        ),
        (
            "self-loop",
            analyze_source_fixture(&[("a.js", "import './a.js';\nimport './a.js';\n")]),
        ),
        (
            "two-cycle",
            analyze_source_fixture(&[
                ("a.js", "import './b.js';\n"),
                ("b.js", "import './a.js';\n"),
            ]),
        ),
    ];

    let expected_profiles = [
        (
            "zero",
            0,
            "not_applicable",
            None,
            None,
            None,
            0,
            0,
            None,
            0,
            None,
        ),
        (
            "one-edge",
            2,
            "computed",
            Some(1),
            Some(2),
            Some(0.5),
            0,
            0,
            Some(0.0),
            0,
            Some(0.0),
        ),
        (
            "self-loop",
            1,
            "computed",
            Some(0),
            Some(0),
            None,
            1,
            1,
            Some(1.0),
            1,
            Some(1.0),
        ),
        (
            "two-cycle",
            2,
            "computed",
            Some(2),
            Some(2),
            Some(1.0),
            1,
            2,
            Some(1.0),
            2,
            Some(1.0),
        ),
    ];

    for ((name, report), expected) in cases.iter().zip(expected_profiles) {
        assert_eq!(*name, expected.0);
        let profile = &report.propagation;
        let value = serde_json::to_value(profile).expect("serialize smallest-case propagation");
        assert_eq!(value["source_files"].as_u64(), Some(expected.1));
        assert_eq!(value["reachability_status"], expected.2);
        assert_eq!(value["reachability_node_limit"], 10_000);
        assert_eq!(value["reachable_nonself_pairs"].as_u64(), expected.3);
        assert_eq!(value["possible_nonself_pairs"].as_u64(), expected.4);
        assert_eq!(value["nonself_propagation_fraction"].as_f64(), expected.5);
        assert_eq!(value["cyclic_components"].as_u64(), Some(expected.6));
        assert_eq!(value["cyclic_source_files"].as_u64(), Some(expected.7));
        assert_eq!(value["cyclic_source_file_fraction"].as_f64(), expected.8);
        assert_eq!(
            value["largest_cyclic_component_files"].as_u64(),
            Some(expected.9)
        );
        assert_eq!(
            value["largest_cyclic_component_fraction"].as_f64(),
            expected.10
        );
    }

    let one_edge = &cases[1].1;
    let a = one_edge
        .nodes
        .iter()
        .find(|node| node.id == "a.js")
        .expect("a.js node");
    let b = one_edge
        .nodes
        .iter()
        .find(|node| node.id == "b.js")
        .expect("b.js node");
    assert_eq!(
        (a.direct_internal_in_degree, a.direct_internal_out_degree),
        (Some(0), Some(1))
    );
    assert_eq!(
        (b.direct_internal_in_degree, b.direct_internal_out_degree),
        (Some(1), Some(0))
    );
    assert_eq!(
        (
            a.transitive_internal_in_count,
            a.transitive_internal_out_count
        ),
        (Some(0), Some(1))
    );
    assert_eq!(
        (
            b.transitive_internal_in_count,
            b.transitive_internal_out_count
        ),
        (Some(1), Some(0))
    );
    assert_eq!(
        one_edge.internal_edges, 1,
        "repeated declarations must not duplicate degree"
    );

    let self_node = cases[2]
        .1
        .nodes
        .iter()
        .find(|node| node.id == "a.js")
        .expect("self node");
    assert_eq!(
        (
            self_node.direct_internal_in_degree,
            self_node.direct_internal_out_degree
        ),
        (Some(1), Some(1))
    );
    assert_eq!(
        (
            self_node.transitive_internal_in_count,
            self_node.transitive_internal_out_count
        ),
        (Some(0), Some(0)),
        "transitive counts exclude self even through a cycle"
    );
}

#[test]
fn deps_cli_json_is_observational_and_text_discloses_structural_proxy_limits() {
    let fixture = DependencyFixture::new();
    let expected_counts = (6_u64, 5_u64, 3_u64);

    let json_output = run_cli(fixture.path(), &["--format", "json"]);
    assert!(
        json_output.status.success(),
        "JSON deps command failed with {:?}: {}",
        json_output.status.code(),
        String::from_utf8_lossy(&json_output.stderr)
    );
    let value: Value = serde_json::from_slice(&json_output.stdout).unwrap_or_else(|error| {
        panic!(
            "deps JSON was invalid: {error}; stdout={:?}",
            String::from_utf8_lossy(&json_output.stdout)
        )
    });
    let root = value
        .as_object()
        .expect("dependency JSON root must be an object");
    for forbidden in ["score", "quality_score", "verdict"] {
        assert!(
            !root.contains_key(forbidden),
            "dependency observations must not emit judgment field {forbidden:?}"
        );
    }
    assert_eq!(
        root.get("internal_edges").and_then(Value::as_u64),
        Some(expected_counts.0)
    );
    assert_eq!(
        root.get("external_edges").and_then(Value::as_u64),
        Some(expected_counts.1)
    );
    assert_eq!(
        root.get("unresolved_edges").and_then(Value::as_u64),
        Some(expected_counts.2)
    );
    assert_eq!(
        root.get("syntax_error_files").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        root.get("manifest_dependency_count")
            .and_then(Value::as_u64),
        Some(13)
    );
    assert_eq!(
        root.get("cycles").and_then(Value::as_array).map(Vec::len),
        Some(1)
    );
    let propagation = root
        .get("propagation")
        .expect("dependency JSON must include propagation profile");
    assert_eq!(propagation["source_files"], 10);
    assert_eq!(propagation["reachability_status"], "computed");
    assert_eq!(propagation["reachability_node_limit"], 10_000);
    assert_eq!(propagation["reachability_work_limit"], 100_000_000);
    assert_eq!(propagation["reachability_work_upper_bound"], 70);
    assert_eq!(propagation["reachable_nonself_pairs"], 9);
    assert_eq!(propagation["possible_nonself_pairs"], 90);
    assert_eq!(propagation["nonself_propagation_fraction"], 0.1);
    assert_eq!(propagation["cyclic_components"], 1);
    assert_eq!(propagation["cyclic_source_files"], 2);
    assert_eq!(propagation["cyclic_source_file_fraction"], 0.2);
    let nodes = root["nodes"]
        .as_array()
        .expect("dependency nodes must be an array");
    let alpha = nodes
        .iter()
        .find(|node| node["id"] == "src/alpha.rs")
        .expect("serialized alpha node");
    assert_eq!(alpha["direct_internal_in_degree"], 1);
    assert_eq!(alpha["direct_internal_out_degree"], 2);
    assert_eq!(alpha["transitive_internal_in_count"], 1);
    assert_eq!(alpha["transitive_internal_out_count"], 3);
    let external = nodes
        .iter()
        .find(|node| node["id"] == "external:react")
        .expect("serialized external node");
    assert!(external["direct_internal_in_degree"].is_null());
    assert!(external["direct_internal_out_degree"].is_null());
    assert!(external["transitive_internal_in_count"].is_null());
    assert!(external["transitive_internal_out_count"].is_null());

    let text_output = run_cli(fixture.path(), &["--top", "2", "--format", "text"]);
    assert!(
        text_output.status.success(),
        "text deps command failed with {:?}: {}",
        text_output.status.code(),
        String::from_utf8_lossy(&text_output.stderr)
    );
    let text = String::from_utf8(text_output.stdout).expect("deps text must be UTF-8");
    assert!(text.contains("graph: 18 nodes, 14 edges (6 internal, 5 external, 3 unresolved), 5 weak components, 1 cycles, condensation-depth=2"));
    assert!(text.contains("internal transitive reachability: 9/90 non-self source-file pairs; status=computed; node-limit=10000"));
    assert!(text.contains(
        "internal cycles: 1 cyclic components, 2/10 cyclic source files, largest=2 source files"
    ));
    assert!(text.contains("INTERNAL-OUT"));
    assert!(text.contains("INTERNAL-IN"));
    assert!(text.contains("TRANSITIVE-OUT"));
    assert!(text.contains("TRANSITIVE-IN"));
    assert!(text.contains("graph statistics are structural proxies, not quality measures"));
    assert!(text.contains("Fan-in, fan-out, components, cycles, and depth are structural proxies and carry no quality verdict or weighting."));
    assert!(text.contains("Resolution is filesystem-only:"));

    let empty = TempDir::new().expect("empty dependency repository");
    let empty_text_output = run_cli(empty.path(), &["--format", "text"]);
    assert!(empty_text_output.status.success());
    let empty_text = String::from_utf8(empty_text_output.stdout).expect("empty deps text UTF-8");
    assert!(empty_text.contains("internal transitive reachability: n/a/n/a non-self source-file pairs; status=not_applicable; node-limit=10000"));
    assert!(empty_text.contains("internal cycles: 0 cyclic components, n/a/n/a cyclic source files, largest=n/a source files"));
}
