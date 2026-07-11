//! Deterministic, evidence-first static dependency graph analysis.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use thiserror::Error;
use tree_sitter::Node;

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
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyReport {
    pub root: String,
    pub analyzer: String,
    pub coverage: DependencyCoverage,
    pub limitations: Vec<String>,
    pub syntax_error_files: usize,
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
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyCoverage {
    pub filesystem_entries_enumerated: usize,
    pub source_files_analyzed: usize,
    pub unsupported_entries_skipped: usize,
    pub declarations_extracted: usize,
    pub unique_edges: usize,
    pub manifests_analyzed: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyNode {
    pub id: String,
    pub kind: DependencyNodeKind,
    pub fan_in: usize,
    pub fan_out: usize,
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
    let nodes = node_kinds
        .iter()
        .map(|(id, kind)| DependencyNode {
            id: id.clone(),
            kind: *kind,
            fan_in: incoming.get(id).map_or(0, BTreeSet::len),
            fan_out: outgoing.get(id).map_or(0, BTreeSet::len),
        })
        .collect::<Vec<_>>();

    let internal_adjacency = adjacency(&known, &edges, true);
    let sccs = tarjan(&known, &internal_adjacency);
    let cycles = sccs
        .iter()
        .filter(|c| {
            c.len() > 1
                || internal_adjacency
                    .get(&c[0])
                    .is_some_and(|n| n.contains(&c[0]))
        })
        .cloned()
        .collect();
    let all_ids: BTreeSet<_> = node_kinds.keys().cloned().collect();
    let all_adjacency = adjacency(&all_ids, &edges, false);
    let weak_components = weak_components(&all_ids, &all_adjacency);
    let depth = condensation_depth(&sccs, &internal_adjacency);
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
    let (manifest_dependencies, manifest_count) = inventory_manifests(input)?;
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
        },
        limitations: vec![
            "Syntax-error trees are analyzed error-tolerantly; declarations from those files may be partial.".to_owned(),
            "Resolution is filesystem-only: no Cargo metadata, Python environment, package.json/tsconfig aliases, JavaScript package exports, Go modules, build tags, generated code, or conditional compilation are interpreted.".to_owned(),
            "Manifest inventory reads only direct declarations; it does not resolve lockfiles, target markers, feature activation, transitive dependencies, or registry defaults beyond the literal manifest syntax.".to_owned(),
            "Rust resolves only mod declarations and direct crate/self/super filesystem module paths; use aliases, re-exports, and extern-prelude names can remain unresolved.".to_owned(),
            "Python resolves only an exact matching .py file or package __init__.py; imported attributes and environment packages are not inferred.".to_owned(),
            "JavaScript and TypeScript resolve only relative paths using an explicit deterministic suffix/index search; bare specifiers are external.".to_owned(),
            "Go imports are external/unresolved without go.mod module-path knowledge; standard-library and third-party imports are not distinguished.".to_owned(),
            "Fan-in, fan-out, components, cycles, and depth are structural proxies and carry no quality verdict or weighting.".to_owned(),
        ],
        syntax_error_files,
        manifest_dependency_count: manifest_dependencies.len(),
        non_registry_manifest_dependency_count,
        risky_manifest_dependency_count,
        manifest_source_kind_counts,
        manifest_dependencies,
        node_count: nodes.len(), edge_count: edges.len(), internal_edges, external_edges, unresolved_edges,
        nodes, edges, strongly_connected_components: sccs, cycles, weak_components,
        condensation_maximum_depth: depth,
    })
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

fn condensation_depth(
    sccs: &[Vec<String>],
    graph: &BTreeMap<String, BTreeSet<String>>,
) -> Option<usize> {
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
    let mut indegree = vec![0usize; sccs.len()];
    for (a, bs) in graph {
        for b in bs {
            let (x, y) = (owner[a], owner[b]);
            if x != y && dag[x].insert(y) {
                indegree[y] += 1;
            }
        }
    }
    let mut queue: VecDeque<_> = (0..sccs.len()).filter(|i| indegree[*i] == 0).collect();
    let mut depth = vec![0usize; sccs.len()];
    while let Some(v) = queue.pop_front() {
        for &w in &dag[v] {
            depth[w] = depth[w].max(depth[v] + 1);
            indegree[w] -= 1;
            if indegree[w] == 0 {
                queue.push_back(w);
            }
        }
    }
    depth.into_iter().max()
}

fn inventory_manifests(input: &Path) -> Result<(Vec<ManifestDependency>, usize), DependencyError> {
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
    let manifest_count = paths.len();
    let mut rows = Vec::new();
    for path in paths {
        let relative = normalized(path.strip_prefix(root).unwrap_or(&path))
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let content =
            fs::read_to_string(&path).map_err(|source| DependencyError::ManifestRead {
                path: path.clone(),
                source,
            })?;
        match path.file_name().and_then(|n| n.to_str()).unwrap_or("") {
            "Cargo.toml" => parse_cargo(&path, &relative, &content, &mut rows)?,
            "package.json" => parse_package_json(&path, &relative, &content, &mut rows)?,
            "pyproject.toml" => parse_pyproject(&path, &relative, &content, &mut rows)?,
            "go.mod" => parse_go_mod(&relative, &content, &mut rows),
            _ => parse_requirements(&relative, &content, &mut rows),
        }
    }
    rows.sort();
    rows.dedup();
    Ok((rows, manifest_count))
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
