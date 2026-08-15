//! Deterministic, heuristic Rust symbol-graph analysis.
//!
//! The graph deliberately resolves less than rustc. Every syntactic reference
//! is classified, collisions remain ambiguous, and only resolved references
//! become edges. The result is a lower bound where it has edges and blind
//! elsewhere, not an approximation that silently guesses.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;
use thiserror::Error;
use tree_sitter::Node;

use crate::deps::{REACHABILITY_NODE_LIMIT, REACHABILITY_WORK_LIMIT, ReachabilityStatus};
use crate::source::{SourceError, SourceFile, SourceLanguage, load_source_tree, parse_source};

#[derive(Debug, Error)]
pub enum SymbolError {
    #[error(transparent)]
    Source(#[from] SourceError),
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolReport {
    pub root: String,
    pub analyzer: String,
    pub epistemic_class: String,
    pub coverage: SymbolCoverage,
    pub resolution: ResolutionLedger,
    pub graph: SymbolGraph,
    pub working_set_reachability: ReachabilityDistribution,
    pub transitive_fan_in_tail: Vec<RankedSymbol>,
    pub per_file_symbol_counts: Vec<FileSymbolCount>,
    pub nodes: Vec<SymbolNode>,
    pub edges: Vec<SymbolEdge>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolCoverage {
    pub filesystem_entries_enumerated: usize,
    pub rust_files_analyzed: usize,
    pub non_rust_or_unsupported_entries_skipped: usize,
    pub syntax_error_files: usize,
    pub declarations_extracted: usize,
    pub symbols_extracted: usize,
    pub call_references: usize,
    pub type_use_references: usize,
    /// Declarations whose (file, qualified path) collided with an earlier
    /// declaration of a different kind; kept under a kind-tagged identity
    /// instead of aborting (Rust's type and value namespaces legitimately
    /// share names, and pre-fix the walker did not scope function bodies).
    pub identity_disambiguations: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolutionLedger {
    pub references_total: usize,
    pub resolved_same_file: usize,
    pub resolved_unique_crate: usize,
    pub resolved_total: usize,
    pub ambiguous: usize,
    pub external_or_unresolved: usize,
    pub resolution_fraction: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolGraph {
    pub node_count: usize,
    pub edge_count: usize,
    pub strongly_connected_component_count: usize,
    pub strongly_connected_component_sizes: Vec<usize>,
    pub strongly_connected_components: Vec<Vec<String>>,
    pub mutually_reachable_pairs: usize,
    pub possible_nonself_pairs: Option<usize>,
    pub mutual_reachability_fraction: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReachabilityDistribution {
    pub status: ReachabilityStatus,
    pub node_limit: usize,
    pub work_limit: usize,
    pub work_upper_bound: Option<usize>,
    pub nodes_in_distribution: usize,
    pub min: Option<usize>,
    pub p50: Option<usize>,
    pub p90: Option<usize>,
    pub max: Option<usize>,
    pub top: Vec<RankedSymbol>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankedSymbol {
    pub id: String,
    pub path: String,
    pub qualified_path: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileSymbolCount {
    pub path: String,
    pub symbols: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolNode {
    pub id: String,
    pub path: String,
    pub qualified_path: String,
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: usize,
    pub direct_in_degree: usize,
    pub direct_out_degree: usize,
    pub transitive_in_count: Option<usize>,
    pub forward_reachable_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolEdge {
    pub source: String,
    pub target: String,
    pub kinds: Vec<SymbolEdgeKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    TypeAlias,
    Static,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolEdgeKind {
    Call,
    TypeUse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ReferenceNamespace {
    Callable,
    Function,
    Type,
}

#[derive(Debug)]
struct RawReference {
    source: String,
    source_path: String,
    name: Option<String>,
    namespace: ReferenceNamespace,
    edge_kind: SymbolEdgeKind,
    method_call: bool,
}

#[derive(Default)]
struct DeclarationContext {
    modules: Vec<String>,
    impl_target: Option<String>,
    in_trait: bool,
}

#[derive(Debug)]
struct Reachability {
    status: ReachabilityStatus,
    work_upper_bound: Option<usize>,
    incoming: Option<Vec<usize>>,
    outgoing: Option<Vec<usize>>,
}

pub fn analyze_symbols(input: &Path) -> Result<SymbolReport, SymbolError> {
    let tree = load_source_tree(input)?;
    let rust_files = tree
        .files
        .iter()
        .filter(|file| file.language == SourceLanguage::Rust)
        .collect::<Vec<_>>();
    let mut syntax_error_files = 0;
    let mut declarations_extracted = 0;
    let mut nodes_by_id = BTreeMap::new();
    let mut references = Vec::new();
    let mut identity_disambiguations = 0usize;

    for file in &rust_files {
        let parsed = parse_source(file)?;
        syntax_error_files += usize::from(parsed.has_syntax_errors);
        let mut declarations_by_start = BTreeMap::new();
        let mut impl_methods = BTreeMap::new();
        collect_declarations(
            file,
            parsed.tree.root_node(),
            &mut DeclarationContext::default(),
            &mut declarations_by_start,
            &mut impl_methods,
            &mut nodes_by_id,
            &mut identity_disambiguations,
        )?;
        declarations_extracted += declarations_by_start.len();
        collect_references(
            file,
            parsed.tree.root_node(),
            None,
            &declarations_by_start,
            &impl_methods,
            &BTreeSet::new(),
            &mut references,
        );
    }

    let mut by_name: BTreeMap<(ReferenceNamespace, String), BTreeSet<String>> = BTreeMap::new();
    let mut by_file_name: BTreeMap<(String, ReferenceNamespace, String), BTreeSet<String>> =
        BTreeMap::new();
    let mut methods_by_name: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for node in nodes_by_id.values() {
        let namespaces = match node.kind {
            SymbolKind::Function => {
                &[ReferenceNamespace::Callable, ReferenceNamespace::Function][..]
            }
            SymbolKind::Method => &[ReferenceNamespace::Callable][..],
            SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Trait | SymbolKind::TypeAlias => {
                &[ReferenceNamespace::Type][..]
            }
            SymbolKind::Static => &[],
        };
        for &namespace in namespaces {
            by_name
                .entry((namespace, node.name.clone()))
                .or_default()
                .insert(node.id.clone());
            by_file_name
                .entry((node.path.clone(), namespace, node.name.clone()))
                .or_default()
                .insert(node.id.clone());
        }
        if node.kind == SymbolKind::Method {
            methods_by_name
                .entry(node.name.clone())
                .or_default()
                .insert(node.id.clone());
        }
    }

    let mut resolution = ResolutionLedger {
        references_total: references.len(),
        resolved_same_file: 0,
        resolved_unique_crate: 0,
        resolved_total: 0,
        ambiguous: 0,
        external_or_unresolved: 0,
        resolution_fraction: None,
    };
    let mut grouped_edges: BTreeMap<(String, String), BTreeSet<SymbolEdgeKind>> = BTreeMap::new();
    for reference in &references {
        match resolve_reference(reference, &by_name, &by_file_name, &methods_by_name) {
            Resolution::SameFile(target) => {
                resolution.resolved_same_file += 1;
                grouped_edges
                    .entry((reference.source.clone(), target))
                    .or_default()
                    .insert(reference.edge_kind);
            }
            Resolution::UniqueCrate(target) => {
                resolution.resolved_unique_crate += 1;
                grouped_edges
                    .entry((reference.source.clone(), target))
                    .or_default()
                    .insert(reference.edge_kind);
            }
            Resolution::Ambiguous => resolution.ambiguous += 1,
            Resolution::ExternalOrUnresolved => resolution.external_or_unresolved += 1,
        }
    }
    resolution.resolved_total = resolution.resolved_same_file + resolution.resolved_unique_crate;
    resolution.resolution_fraction = (resolution.references_total != 0)
        .then(|| resolution.resolved_total as f64 / resolution.references_total as f64);

    let edges = grouped_edges
        .into_iter()
        .map(|((source, target), kinds)| SymbolEdge {
            source,
            target,
            kinds: kinds.into_iter().collect(),
        })
        .collect::<Vec<_>>();
    let ids = nodes_by_id.keys().cloned().collect::<Vec<_>>();
    let index = ids
        .iter()
        .enumerate()
        .map(|(position, id)| (id.as_str(), position))
        .collect::<BTreeMap<_, _>>();
    let mut adjacency = vec![BTreeSet::new(); ids.len()];
    let mut reverse = vec![BTreeSet::new(); ids.len()];
    for edge in &edges {
        let (Some(&source), Some(&target)) = (
            index.get(edge.source.as_str()),
            index.get(edge.target.as_str()),
        ) else {
            continue;
        };
        adjacency[source].insert(target);
        reverse[target].insert(source);
    }
    let adjacency = adjacency
        .into_iter()
        .map(|targets| targets.into_iter().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let reverse = reverse
        .into_iter()
        .map(|sources| sources.into_iter().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let reachability = transitive_degrees(&adjacency);
    let components = tarjan(&ids, &adjacency);

    let mut nodes = nodes_by_id.into_values().collect::<Vec<_>>();
    for node in &mut nodes {
        if let Some(&position) = index.get(node.id.as_str()) {
            node.direct_out_degree = adjacency[position].len();
            node.direct_in_degree = reverse[position].len();
            node.forward_reachable_count = reachability
                .outgoing
                .as_ref()
                .map(|counts| counts[position]);
            node.transitive_in_count = reachability
                .incoming
                .as_ref()
                .map(|counts| counts[position]);
        }
    }

    let graph = graph_summary(nodes.len(), edges.len(), components);
    let working_set_reachability = reachability_distribution(&nodes, &reachability);
    let transitive_fan_in_tail = ranked_symbols(&nodes, |node| node.transitive_in_count);
    let per_file_symbol_counts = file_symbol_counts(&rust_files, &nodes);
    let call_references = references
        .iter()
        .filter(|reference| reference.edge_kind == SymbolEdgeKind::Call)
        .count();
    let type_use_references = references.len() - call_references;

    Ok(SymbolReport {
        root: tree.root,
        analyzer: "tree-sitter-rust-symbol-graph-v1".to_owned(),
        epistemic_class: "proxy".to_owned(),
        coverage: SymbolCoverage {
            filesystem_entries_enumerated: tree.enumerated,
            rust_files_analyzed: rust_files.len(),
            non_rust_or_unsupported_entries_skipped: tree.enumerated - rust_files.len(),
            syntax_error_files,
            declarations_extracted,
            symbols_extracted: nodes.len(),
            call_references,
            type_use_references,
            identity_disambiguations,
        },
        resolution,
        graph,
        working_set_reachability,
        transitive_fan_in_tail,
        per_file_symbol_counts,
        nodes,
        edges,
        limitations: vec![
            "Macro-generated code is invisible because tree-sitter sees source syntax, not expanded Rust.".to_owned(),
            "Trait-object/dyn dispatch and generic instantiation are unresolved.".to_owned(),
            "Name-collision references are dropped as ambiguous; the exact count is reported in resolution.ambiguous.".to_owned(),
            "Resolution is a lexical heuristic, not rustc name resolution: the graph is a lower bound on true coupling for resolved edges and blind elsewhere.".to_owned(),
            "Rust-only: non-Rust files are outside this slice; the report shape does not assume a Rust grammar.".to_owned(),
            "Consts are excluded entirely; statics whose declared type contains a function type are also excluded.".to_owned(),
            "Multiple same-kind syntactic declarations with the same required (file, qualified-path) identity, such as cfg alternatives, collapse to one graph node; declarations_extracted and symbols_extracted expose that denominator difference.".to_owned(),
            "Impl-header type references are attributed to every named function in that impl because impl blocks are not graph nodes.".to_owned(),
            "Files with tree-sitter syntax errors remain in the file denominator; error recovery can make their extracted symbols and references partial.".to_owned(),
        ],
    })
}

fn collect_declarations(
    file: &SourceFile,
    node: Node<'_>,
    context: &mut DeclarationContext,
    declarations_by_start: &mut BTreeMap<usize, String>,
    impl_methods: &mut BTreeMap<usize, Vec<String>>,
    nodes: &mut BTreeMap<String, SymbolNode>,
    identity_disambiguations: &mut usize,
) -> Result<(), SymbolError> {
    if node.kind() == "mod_item" {
        let name = field_text(file, node, "name");
        if let (Some(name), Some(body)) = (name, node.child_by_field_name("body")) {
            context.modules.push(name);
            try_visit_named_children(body, |child| {
                collect_declarations(
                    file,
                    child,
                    context,
                    declarations_by_start,
                    impl_methods,
                    nodes,
                    identity_disambiguations,
                )
            })?;
            context.modules.pop();
        }
        return Ok(());
    }
    if node.kind() == "impl_item" {
        let target = node
            .child_by_field_name("type")
            .and_then(|target| compact_text(file, target))
            .unwrap_or_else(|| "<unknown-impl>".to_owned());
        let impl_target = node
            .child_by_field_name("trait")
            .and_then(|trait_node| compact_text(file, trait_node))
            .map_or(target.clone(), |trait_name| {
                format!("<{target} as {trait_name}>")
            });
        let previous = context.impl_target.replace(impl_target);
        let before = declarations_by_start.len();
        if let Some(body) = node.child_by_field_name("body") {
            try_visit_named_children(body, |child| {
                collect_declarations(
                    file,
                    child,
                    context,
                    declarations_by_start,
                    impl_methods,
                    nodes,
                    identity_disambiguations,
                )
            })?;
        }
        let methods = declarations_by_start
            .values()
            .skip(before)
            .filter(|id| {
                nodes
                    .get(*id)
                    .is_some_and(|symbol| symbol.kind == SymbolKind::Method)
            })
            .cloned()
            .collect::<Vec<_>>();
        impl_methods.insert(node.start_byte(), methods);
        context.impl_target = previous;
        return Ok(());
    }

    if let Some(kind) = declaration_kind(node, context)
        && let Some(name) = field_text(file, node, "name")
    {
        let qualified_path =
            qualified_path(&context.modules, context.impl_target.as_deref(), &name);
        let id = format!("{}::{qualified_path}", file.path);
        let symbol = SymbolNode {
            id: id.clone(),
            path: file.path.clone(),
            qualified_path: qualified_path.clone(),
            name,
            kind,
            start_line: node.start_position().row + 1,
            direct_in_degree: 0,
            direct_out_degree: 0,
            transitive_in_count: None,
            forward_reachable_count: None,
        };
        if let Some(existing) = nodes.get(&id)
            && existing.kind != kind
        {
            // Same (file, qualified path), different kind: Rust's type and
            // value namespaces legitimately allow this. Keep both under a
            // kind-tagged identity and count the disambiguation.
            let tagged = format!("{id}#{}", kind_tag(kind));
            declarations_by_start.insert(node.start_byte(), tagged.clone());
            if !nodes.contains_key(&tagged) {
                *identity_disambiguations += 1;
                nodes.insert(
                    tagged.clone(),
                    SymbolNode {
                        id: tagged,
                        ..symbol
                    },
                );
            }
        } else {
            declarations_by_start.insert(node.start_byte(), id.clone());
            nodes.entry(id).or_insert(symbol);
        }
    }

    let previous_trait = context.in_trait;
    if node.kind() == "trait_item" {
        context.in_trait = true;
    }
    // A function body is a scope: function-local declarations are qualified
    // by the enclosing function so distinct locals never share an identity.
    let mut function_scope_pushed = false;
    if node.kind() == "function_item"
        && let Some(function_name) = field_text(file, node, "name")
    {
        context.modules.push(function_name);
        function_scope_pushed = true;
    }
    try_visit_named_children(node, |child| {
        collect_declarations(
            file,
            child,
            context,
            declarations_by_start,
            impl_methods,
            nodes,
            identity_disambiguations,
        )
    })?;
    if function_scope_pushed {
        context.modules.pop();
    }
    context.in_trait = previous_trait;
    Ok(())
}

fn declaration_kind(node: Node<'_>, context: &DeclarationContext) -> Option<SymbolKind> {
    match node.kind() {
        "function_item" if context.impl_target.is_some() => Some(SymbolKind::Method),
        "function_item" if !context.in_trait => Some(SymbolKind::Function),
        "struct_item" => Some(SymbolKind::Struct),
        "enum_item" => Some(SymbolKind::Enum),
        "trait_item" => Some(SymbolKind::Trait),
        "type_item" => Some(SymbolKind::TypeAlias),
        "static_item"
            if node
                .child_by_field_name("type")
                .is_some_and(|declared_type| !contains_kind(declared_type, "function_type")) =>
        {
            Some(SymbolKind::Static)
        }
        _ => None,
    }
}

fn kind_tag(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "fn",
        SymbolKind::Method => "method",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Trait => "trait",
        SymbolKind::TypeAlias => "type",
        SymbolKind::Static => "static",
    }
}

fn qualified_path(modules: &[String], impl_target: Option<&str>, name: &str) -> String {
    modules
        .iter()
        .map(String::as_str)
        .chain(impl_target)
        .chain(std::iter::once(name))
        .collect::<Vec<_>>()
        .join("::")
}

fn collect_references(
    file: &SourceFile,
    node: Node<'_>,
    inherited_source: Option<&str>,
    declarations: &BTreeMap<usize, String>,
    impl_methods: &BTreeMap<usize, Vec<String>>,
    inherited_generics: &BTreeSet<String>,
    out: &mut Vec<RawReference>,
) {
    let declared_source = declarations.get(&node.start_byte()).map(String::as_str);
    let source = declared_source.or(inherited_source);
    let mut generics = inherited_generics.clone();
    if let Some(type_parameters) = node.child_by_field_name("type_parameters") {
        collect_generic_parameter_names(file, type_parameters, &mut generics);
    }

    if node.kind() == "impl_item" {
        let methods = impl_methods
            .get(&node.start_byte())
            .map(Vec::as_slice)
            .unwrap_or_default();
        for field in ["trait", "type", "type_parameters"] {
            if let Some(region) = node.child_by_field_name(field) {
                for method in methods {
                    collect_type_references(file, region, method, &generics, out);
                }
            }
        }
        visit_named_children(node, |child| {
            if child.kind() == "where_clause" {
                for method in methods {
                    collect_type_references(file, child, method, &generics, out);
                }
            }
        });
        visit_named_children(node, |child| {
            if child.kind() == "declaration_list" {
                collect_references(
                    file,
                    child,
                    None,
                    declarations,
                    impl_methods,
                    &generics,
                    out,
                );
            }
        });
        return;
    }

    if let Some(source) = declared_source {
        match node.kind() {
            "function_item" => {
                collect_fields_as_types(
                    file,
                    node,
                    &["parameters", "return_type", "type_parameters"],
                    source,
                    &generics,
                    out,
                );
                collect_where_clause_types(file, node, source, &generics, out);
            }
            "struct_item" | "enum_item" | "trait_item" => {
                collect_fields_as_types(
                    file,
                    node,
                    &["bounds", "type_parameters"],
                    source,
                    &generics,
                    out,
                );
                if let Some(body) = node.child_by_field_name("body") {
                    collect_member_type_regions(file, body, source, &generics, out);
                }
                collect_where_clause_types(file, node, source, &generics, out);
            }
            "type_item" => {
                collect_fields_as_types(
                    file,
                    node,
                    &["type", "type_parameters"],
                    source,
                    &generics,
                    out,
                );
                collect_where_clause_types(file, node, source, &generics, out);
            }
            "static_item" => {
                collect_fields_as_types(file, node, &["type"], source, &generics, out);
            }
            _ => {}
        }
    }

    if node.kind() == "call_expression"
        && let Some(source) = source
    {
        let (name, namespace, method_call) = node
            .child_by_field_name("function")
            .map(|callee| call_target(file, callee))
            .unwrap_or((None, ReferenceNamespace::Callable, false));
        out.push(RawReference {
            source: source.to_owned(),
            source_path: file.path.clone(),
            name,
            namespace,
            edge_kind: SymbolEdgeKind::Call,
            method_call,
        });
    }

    visit_named_children(node, |child| {
        collect_references(
            file,
            child,
            source,
            declarations,
            impl_methods,
            &generics,
            out,
        );
    });
}

fn collect_member_type_regions(
    file: &SourceFile,
    node: Node<'_>,
    source: &str,
    inherited_generics: &BTreeSet<String>,
    out: &mut Vec<RawReference>,
) {
    match node.kind() {
        "field_declaration_list" | "ordered_field_declaration_list" => {
            collect_type_references(file, node, source, inherited_generics, out);
            return;
        }
        "function_signature_item" | "function_item" => {
            let mut generics = inherited_generics.clone();
            if let Some(type_parameters) = node.child_by_field_name("type_parameters") {
                collect_generic_parameter_names(file, type_parameters, &mut generics);
            }
            collect_fields_as_types(
                file,
                node,
                &["parameters", "return_type", "type_parameters"],
                source,
                &generics,
                out,
            );
            collect_where_clause_types(file, node, source, &generics, out);
            return;
        }
        "type_item" => return,
        _ => {}
    }
    visit_named_children(node, |child| {
        collect_member_type_regions(file, child, source, inherited_generics, out);
    });
}

fn collect_fields_as_types(
    file: &SourceFile,
    node: Node<'_>,
    fields: &[&str],
    source: &str,
    generics: &BTreeSet<String>,
    out: &mut Vec<RawReference>,
) {
    for field in fields {
        if let Some(region) = node.child_by_field_name(field) {
            collect_type_references(file, region, source, generics, out);
        }
    }
}

fn collect_where_clause_types(
    file: &SourceFile,
    node: Node<'_>,
    source: &str,
    generics: &BTreeSet<String>,
    out: &mut Vec<RawReference>,
) {
    visit_named_children(node, |child| {
        if child.kind() == "where_clause" {
            collect_type_references(file, child, source, generics, out);
        }
    });
}

fn collect_type_references(
    file: &SourceFile,
    node: Node<'_>,
    source: &str,
    generics: &BTreeSet<String>,
    out: &mut Vec<RawReference>,
) {
    if node.kind() == "type_identifier" {
        if let Some(name) = node_text(file, node).filter(|name| !generics.contains(*name)) {
            out.push(RawReference {
                source: source.to_owned(),
                source_path: file.path.clone(),
                name: Some(name.to_owned()),
                namespace: ReferenceNamespace::Type,
                edge_kind: SymbolEdgeKind::TypeUse,
                method_call: false,
            });
        }
        return;
    }
    if node.kind() == "type_parameter" {
        visit_named_children(node, |child| {
            if node.child_by_field_name("name") != Some(child) {
                collect_type_references(file, child, source, generics, out);
            }
        });
        return;
    }
    visit_named_children(node, |child| {
        collect_type_references(file, child, source, generics, out);
    });
}

fn collect_generic_parameter_names(
    file: &SourceFile,
    node: Node<'_>,
    names: &mut BTreeSet<String>,
) {
    if node.kind() == "type_parameter" {
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|name| node_text(file, name))
        {
            names.insert(name.to_owned());
        }
        return;
    }
    visit_named_children(node, |child| {
        collect_generic_parameter_names(file, child, names);
    });
}

fn call_target(
    file: &SourceFile,
    mut callee: Node<'_>,
) -> (Option<String>, ReferenceNamespace, bool) {
    if callee.kind() == "generic_function" {
        let Some(function) = callee.child_by_field_name("function") else {
            return (None, ReferenceNamespace::Callable, false);
        };
        callee = function;
    }
    if callee.kind() == "field_expression" {
        let name = callee
            .child_by_field_name("field")
            .and_then(|field| node_text(file, field))
            .map(str::to_owned);
        return (name, ReferenceNamespace::Callable, true);
    }
    let (name_node, namespace) = match callee.kind() {
        "scoped_identifier" => (
            callee.child_by_field_name("name"),
            ReferenceNamespace::Callable,
        ),
        "identifier" => (Some(callee), ReferenceNamespace::Function),
        _ => (None, ReferenceNamespace::Callable),
    };
    (
        name_node
            .and_then(|name| node_text(file, name))
            .map(str::to_owned),
        namespace,
        false,
    )
}

#[derive(Debug)]
enum Resolution {
    SameFile(String),
    UniqueCrate(String),
    Ambiguous,
    ExternalOrUnresolved,
}

fn resolve_reference(
    reference: &RawReference,
    by_name: &BTreeMap<(ReferenceNamespace, String), BTreeSet<String>>,
    by_file_name: &BTreeMap<(String, ReferenceNamespace, String), BTreeSet<String>>,
    methods_by_name: &BTreeMap<String, BTreeSet<String>>,
) -> Resolution {
    let Some(name) = reference.name.as_ref() else {
        return Resolution::ExternalOrUnresolved;
    };
    if reference.method_call {
        return match methods_by_name.get(name).map(BTreeSet::len).unwrap_or(0) {
            0 => Resolution::ExternalOrUnresolved,
            1 => {
                let target = methods_by_name
                    .get(name)
                    .and_then(|matches| matches.first())
                    .cloned();
                match target {
                    Some(target) if target.starts_with(&format!("{}::", reference.source_path)) => {
                        Resolution::SameFile(target)
                    }
                    Some(target) => Resolution::UniqueCrate(target),
                    None => Resolution::ExternalOrUnresolved,
                }
            }
            _ => Resolution::Ambiguous,
        };
    }

    let file_key = (
        reference.source_path.clone(),
        reference.namespace,
        name.clone(),
    );
    match by_file_name.get(&file_key).map(BTreeSet::len).unwrap_or(0) {
        1 => {
            return by_file_name
                .get(&file_key)
                .and_then(|matches| matches.first())
                .cloned()
                .map_or(Resolution::ExternalOrUnresolved, Resolution::SameFile);
        }
        count if count > 1 => return Resolution::Ambiguous,
        _ => {}
    }
    let crate_key = (reference.namespace, name.clone());
    match by_name.get(&crate_key).map(BTreeSet::len).unwrap_or(0) {
        1 => by_name
            .get(&crate_key)
            .and_then(|matches| matches.first())
            .cloned()
            .map_or(Resolution::ExternalOrUnresolved, Resolution::UniqueCrate),
        count if count > 1 => Resolution::Ambiguous,
        _ => Resolution::ExternalOrUnresolved,
    }
}

fn transitive_degrees(adjacency: &[Vec<usize>]) -> Reachability {
    let work_upper_bound = adjacency
        .iter()
        .try_fold(0usize, |sum, edges| sum.checked_add(edges.len()))
        .and_then(|edges| edges.checked_add(1))
        .and_then(|per_source| adjacency.len().checked_mul(per_source));
    let status = if adjacency.is_empty() {
        ReachabilityStatus::NotApplicable
    } else if adjacency.len() > REACHABILITY_NODE_LIMIT {
        ReachabilityStatus::SizeLimit
    } else if work_upper_bound.is_none_or(|bound| bound > REACHABILITY_WORK_LIMIT) {
        ReachabilityStatus::WorkLimit
    } else {
        ReachabilityStatus::Computed
    };
    if status != ReachabilityStatus::Computed {
        return Reachability {
            status,
            work_upper_bound,
            incoming: None,
            outgoing: None,
        };
    }

    let mut incoming = vec![0; adjacency.len()];
    let mut outgoing = vec![0; adjacency.len()];
    let mut visited = vec![0; adjacency.len()];
    let mut generation = 0usize;
    let mut stack = Vec::new();
    for source in 0..adjacency.len() {
        generation += 1;
        visited[source] = generation;
        stack.extend(adjacency[source].iter().copied());
        while let Some(target) = stack.pop() {
            if visited[target] == generation {
                continue;
            }
            visited[target] = generation;
            outgoing[source] += 1;
            incoming[target] += 1;
            stack.extend(adjacency[target].iter().copied());
        }
    }
    Reachability {
        status,
        work_upper_bound,
        incoming: Some(incoming),
        outgoing: Some(outgoing),
    }
}

fn tarjan(ids: &[String], graph: &[Vec<usize>]) -> Vec<Vec<String>> {
    struct State<'a> {
        ids: &'a [String],
        graph: &'a [Vec<usize>],
        next: usize,
        indices: Vec<Option<usize>>,
        low: Vec<usize>,
        stack: Vec<usize>,
        on_stack: BTreeSet<usize>,
        result: Vec<Vec<String>>,
    }
    fn visit(vertex: usize, state: &mut State<'_>) {
        let index = state.next;
        state.next += 1;
        state.indices[vertex] = Some(index);
        state.low[vertex] = index;
        state.stack.push(vertex);
        state.on_stack.insert(vertex);
        for &target in &state.graph[vertex] {
            if state.indices[target].is_none() {
                visit(target, state);
                state.low[vertex] = state.low[vertex].min(state.low[target]);
            } else if state.on_stack.contains(&target)
                && let Some(target_index) = state.indices[target]
            {
                state.low[vertex] = state.low[vertex].min(target_index);
            }
        }
        if state.indices[vertex] == Some(state.low[vertex]) {
            let mut component = Vec::new();
            while let Some(target) = state.stack.pop() {
                state.on_stack.remove(&target);
                component.push(state.ids[target].clone());
                if target == vertex {
                    break;
                }
            }
            component.sort();
            state.result.push(component);
        }
    }

    let mut state = State {
        ids,
        graph,
        next: 0,
        indices: vec![None; ids.len()],
        low: vec![0; ids.len()],
        stack: Vec::new(),
        on_stack: BTreeSet::new(),
        result: Vec::new(),
    };
    for vertex in 0..ids.len() {
        if state.indices[vertex].is_none() {
            visit(vertex, &mut state);
        }
    }
    state.result.sort();
    state.result
}

fn graph_summary(
    node_count: usize,
    edge_count: usize,
    components: Vec<Vec<String>>,
) -> SymbolGraph {
    let mutually_reachable_pairs = components
        .iter()
        .map(|component| component.len() * component.len().saturating_sub(1))
        .sum();
    let possible_nonself_pairs = node_count.checked_mul(node_count.saturating_sub(1));
    let mutual_reachability_fraction = possible_nonself_pairs.and_then(|possible| {
        (possible != 0).then_some(mutually_reachable_pairs as f64 / possible as f64)
    });
    let mut sizes = components.iter().map(Vec::len).collect::<Vec<_>>();
    sizes.sort_by(|left, right| right.cmp(left));
    SymbolGraph {
        node_count,
        edge_count,
        strongly_connected_component_count: components.len(),
        strongly_connected_component_sizes: sizes,
        strongly_connected_components: components,
        mutually_reachable_pairs,
        possible_nonself_pairs,
        mutual_reachability_fraction,
    }
}

fn reachability_distribution(
    nodes: &[SymbolNode],
    reachability: &Reachability,
) -> ReachabilityDistribution {
    let mut values = reachability.outgoing.clone().unwrap_or_default();
    values.sort_unstable();
    ReachabilityDistribution {
        status: reachability.status,
        node_limit: REACHABILITY_NODE_LIMIT,
        work_limit: REACHABILITY_WORK_LIMIT,
        work_upper_bound: reachability.work_upper_bound,
        nodes_in_distribution: values.len(),
        min: values.first().copied(),
        p50: nearest_rank(&values, 50),
        p90: nearest_rank(&values, 90),
        max: values.last().copied(),
        top: ranked_symbols(nodes, |node| node.forward_reachable_count),
    }
}

fn nearest_rank(values: &[usize], percentile: usize) -> Option<usize> {
    if values.is_empty() {
        return None;
    }
    let rank = values.len().saturating_mul(percentile).saturating_add(99) / 100;
    values.get(rank.saturating_sub(1)).copied()
}

fn ranked_symbols(
    nodes: &[SymbolNode],
    count: impl Fn(&SymbolNode) -> Option<usize>,
) -> Vec<RankedSymbol> {
    let mut rows = nodes
        .iter()
        .filter_map(|node| {
            count(node).map(|count| RankedSymbol {
                id: node.id.clone(),
                path: node.path.clone(),
                qualified_path: node.qualified_path.clone(),
                count,
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.id.cmp(&right.id))
    });
    rows
}

fn file_symbol_counts(files: &[&SourceFile], nodes: &[SymbolNode]) -> Vec<FileSymbolCount> {
    let mut counts = files
        .iter()
        .map(|file| (file.path.clone(), 0usize))
        .collect::<BTreeMap<_, _>>();
    for node in nodes {
        *counts.entry(node.path.clone()).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(path, symbols)| FileSymbolCount { path, symbols })
        .collect()
}

fn field_text(file: &SourceFile, node: Node<'_>, field: &str) -> Option<String> {
    node.child_by_field_name(field)
        .and_then(|child| node_text(file, child))
        .map(str::to_owned)
}

fn compact_text(file: &SourceFile, node: Node<'_>) -> Option<String> {
    node_text(file, node).map(|text| {
        text.chars()
            .filter(|character| !character.is_whitespace())
            .collect()
    })
}

fn contains_kind(node: Node<'_>, kind: &str) -> bool {
    if node.kind() == kind {
        return true;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| contains_kind(child, kind))
}

fn node_text<'a>(file: &'a SourceFile, node: Node<'_>) -> Option<&'a str> {
    file.bytes
        .get(node.byte_range())
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
}

fn visit_named_children<'tree>(node: Node<'tree>, mut visit: impl FnMut(Node<'tree>)) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(child);
    }
}

fn try_visit_named_children<'tree, E>(
    node: Node<'tree>,
    mut visit: impl FnMut(Node<'tree>) -> Result<(), E>,
) -> Result<(), E> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(child)?;
    }
    Ok(())
}
