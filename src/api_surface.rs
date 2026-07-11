//! Deterministic, tree-sitter-based public API surface inventory.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;
use thiserror::Error;
use tree_sitter::Node;

use crate::source::{SourceError, SourceFile, SourceLanguage, load_source_tree, parse_source};

const ANALYZER: &str = "tree-sitter-public-api-surface-v1";

#[derive(Debug, Error)]
pub enum ApiError {
    #[error(transparent)]
    Source(#[from] SourceError),
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiReport {
    pub root: String,
    pub analyzer: String,
    pub coverage: ApiCoverage,
    pub limitations: Vec<String>,
    pub symbols: Vec<ApiSymbol>,
    pub counts: ApiCounts,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiCoverage {
    pub enumerated_paths: usize,
    pub skipped_non_source_paths: usize,
    pub source_files: usize,
    pub parsed_files: usize,
    pub syntax_error_files: usize,
    pub source_lines: usize,
    pub source_line_definition: String,
    pub density_denominator_source_lines: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiCounts {
    pub public_symbols: usize,
    pub functions: usize,
    pub methods: usize,
    pub types: usize,
    pub constants: usize,
    pub fields: usize,
    pub other: usize,
    pub documented_symbols: usize,
    pub total_parameters: usize,
    pub total_generic_or_type_parameters: usize,
    pub public_symbols_per_ksloc: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiSymbol {
    pub path: String,
    pub line: usize,
    pub language: SourceLanguage,
    pub symbol: String,
    pub kind: ApiSymbolKind,
    pub visibility_or_proxy_basis: String,
    pub parameter_count: usize,
    pub generic_or_type_parameter_count: usize,
    pub documentation_immediately_precedes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApiSymbolKind {
    Function,
    Method,
    Type,
    Constant,
    Field,
    ReExport,
    Other,
}

pub fn analyze_api_surface(input: &Path) -> Result<ApiReport, ApiError> {
    let source = load_source_tree(input)?;
    let mut symbols = Vec::new();
    let mut syntax_error_files = 0;
    let mut source_lines = 0;

    for file in &source.files {
        source_lines += count_source_lines(&file.bytes);
        let parsed = parse_source(file)?;
        if parsed.has_syntax_errors {
            syntax_error_files += 1;
        }
        collect_file(file, parsed.tree.root_node(), &mut symbols);
    }

    symbols.sort_by(|a, b| {
        (&a.path, a.line, a.kind, &a.symbol).cmp(&(&b.path, b.line, b.kind, &b.symbol))
    });
    symbols.dedup_by(|a, b| {
        a.path == b.path
            && a.line == b.line
            && a.kind == b.kind
            && a.symbol == b.symbol
            && a.visibility_or_proxy_basis == b.visibility_or_proxy_basis
    });

    let counts = count_symbols(&symbols, source_lines);
    let mut limitations = vec![
        "This is a structural API-surface proxy, not an API quality score or verdict.".to_owned(),
        "JavaScript CommonJS assignments (for example module.exports), computed exports, and dynamic export patterns are not covered.".to_owned(),
        "Python publicness is a convention proxy: module-level names not beginning with an underscore are treated as public; __all__ is not interpreted.".to_owned(),
        "Go visibility is a lexical proxy based on an identifier beginning with an uppercase Unicode character; package reachability is not resolved.".to_owned(),
        "Documentation attachment is adjacency-based and language-aware; generated, macro-expanded, inherited, detached, and non-comment documentation is not resolved.".to_owned(),
        "Export lists and re-exports are inventoried as re-export rows; their target declaration signatures are not resolved.".to_owned(),
    ];
    if syntax_error_files > 0 {
        limitations.push("Tree-sitter produced error-tolerant trees for syntax-error files; symbols from those files may be partial.".to_owned());
    }

    Ok(ApiReport {
        root: source.root,
        analyzer: ANALYZER.to_owned(),
        coverage: ApiCoverage {
            enumerated_paths: source.enumerated,
            skipped_non_source_paths: source.skipped,
            source_files: source.files.len(),
            parsed_files: source.files.len(),
            syntax_error_files,
            source_lines,
            source_line_definition: "non-blank physical lines in supported source files".to_owned(),
            density_denominator_source_lines: source_lines,
        },
        limitations,
        symbols,
        counts,
    })
}

fn count_symbols(symbols: &[ApiSymbol], source_lines: usize) -> ApiCounts {
    let mut counts = ApiCounts {
        public_symbols: symbols.len(),
        functions: 0,
        methods: 0,
        types: 0,
        constants: 0,
        fields: 0,
        other: 0,
        documented_symbols: 0,
        total_parameters: 0,
        total_generic_or_type_parameters: 0,
        public_symbols_per_ksloc: 0.0,
    };
    for symbol in symbols {
        match symbol.kind {
            ApiSymbolKind::Function => counts.functions += 1,
            ApiSymbolKind::Method => counts.methods += 1,
            ApiSymbolKind::Type => counts.types += 1,
            ApiSymbolKind::Constant => counts.constants += 1,
            ApiSymbolKind::Field => counts.fields += 1,
            ApiSymbolKind::ReExport | ApiSymbolKind::Other => counts.other += 1,
        }
        counts.documented_symbols += usize::from(symbol.documentation_immediately_precedes);
        counts.total_parameters += symbol.parameter_count;
        counts.total_generic_or_type_parameters += symbol.generic_or_type_parameter_count;
    }
    if source_lines != 0 {
        counts.public_symbols_per_ksloc = symbols.len() as f64 * 1000.0 / source_lines as f64;
    }
    counts
}

fn collect_file(file: &SourceFile, root: Node<'_>, out: &mut Vec<ApiSymbol>) {
    match file.language {
        SourceLanguage::Rust => walk_rust(file, root, out),
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            walk_ecmascript(file, root, out)
        }
        SourceLanguage::Go => walk_go(file, root, out),
        SourceLanguage::Python => walk_python_module(file, root, out),
    }
}

fn walk_rust(file: &SourceFile, node: Node<'_>, out: &mut Vec<ApiSymbol>) {
    walk_rust_reachable(file, node, true, false, out);
}

fn walk_rust_reachable(
    file: &SourceFile,
    node: Node<'_>,
    publicly_reachable: bool,
    in_public_trait: bool,
    out: &mut Vec<ApiSymbol>,
) {
    let kind = node.kind();
    let visibility = rust_visibility(file, node);
    let is_public = visibility.as_deref() == Some("pub");
    let mut child_reachable = publicly_reachable;
    let mut child_in_public_trait = in_public_trait;

    if kind == "mod_item" {
        if publicly_reachable && is_public {
            push_decl(
                file,
                node,
                ApiSymbolKind::Other,
                "Rust explicit visibility: pub".to_owned(),
                out,
            );
        }
        child_reachable = publicly_reachable && is_public;
        child_in_public_trait = false;
    } else if matches!(
        kind,
        "function_item"
            | "struct_item"
            | "enum_item"
            | "union_item"
            | "trait_item"
            | "type_item"
            | "const_item"
            | "static_item"
    ) {
        if in_public_trait && kind == "function_item" && is_direct_trait_member(node) {
            push_decl(
                file,
                node,
                ApiSymbolKind::Method,
                "Rust public trait member; containing trait is publicly reachable".to_owned(),
                out,
            );
        } else if publicly_reachable && is_public {
            push_decl(
                file,
                node,
                rust_kind(node),
                "Rust explicit visibility: pub".to_owned(),
                out,
            );
            if kind == "struct_item" || kind == "union_item" {
                collect_rust_fields(file, node, out);
            }
        }
        if kind == "trait_item" {
            child_in_public_trait = publicly_reachable && is_public;
        }
    } else if kind == "use_declaration" {
        if publicly_reachable && is_public {
            collect_rust_reexports(file, node, "pub", out);
        }
    } else if kind == "function_signature_item" {
        if in_public_trait && is_direct_trait_member(node) {
            push_decl(
                file,
                node,
                ApiSymbolKind::Method,
                "Rust public trait member; containing trait is publicly reachable".to_owned(),
                out,
            );
        }
    } else if kind == "field_declaration" {
        // Fields are collected with their public containing type to avoid duplicates.
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_rust_reachable(file, child, child_reachable, child_in_public_trait, out);
    }
}

fn rust_visibility(file: &SourceFile, node: Node<'_>) -> Option<String> {
    node.child_by_field_name("visibility")
        .or_else(|| named_child_of_kind(node, "visibility_modifier"))
        .map(|visibility| text(file, visibility).trim().to_owned())
}

fn is_direct_trait_member(node: Node<'_>) -> bool {
    node.parent()
        .filter(|parent| parent.kind() == "declaration_list")
        .and_then(|parent| parent.parent())
        .is_some_and(|parent| parent.kind() == "trait_item")
}

fn collect_rust_reexports(
    file: &SourceFile,
    declaration: Node<'_>,
    visibility: &str,
    out: &mut Vec<ApiSymbol>,
) {
    let basis = format!("Rust explicit visibility: {visibility}; re-export syntax");
    let mut aliases = BTreeSet::new();
    collect_rust_use_aliases(file, declaration, &mut aliases);
    if aliases.is_empty() && text(file, declaration).contains('*') {
        aliases.insert("*".to_owned());
    }
    for alias in aliases {
        push_named(
            file,
            declaration,
            alias,
            ApiSymbolKind::ReExport,
            basis.clone(),
            0,
            0,
            out,
        );
    }
}

fn collect_rust_use_aliases(file: &SourceFile, node: Node<'_>, aliases: &mut BTreeSet<String>) {
    if node.kind() == "use_as_clause" {
        if let Some(alias) = node
            .child_by_field_name("alias")
            .or_else(|| last_named_child(node))
        {
            aliases.insert(text(file, alias));
        }
        return;
    }
    if node.kind() == "scoped_use_list" {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "use_list" {
                collect_rust_use_aliases(file, child, aliases);
            }
        }
        return;
    }
    if node.kind() == "use_list" {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            collect_rust_use_aliases(file, child, aliases);
        }
        return;
    }
    if matches!(node.kind(), "identifier" | "type_identifier") {
        aliases.insert(text(file, node));
        return;
    }
    if matches!(node.kind(), "scoped_identifier" | "scoped_type_identifier") {
        if let Some(name) = node
            .child_by_field_name("name")
            .or_else(|| last_named_child(node))
        {
            aliases.insert(text(file, name));
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_rust_use_aliases(file, child, aliases);
    }
}

fn collect_rust_fields(file: &SourceFile, declaration: Node<'_>, out: &mut Vec<ApiSymbol>) {
    let mut stack = vec![declaration];
    while let Some(node) = stack.pop() {
        if node != declaration && node.kind() == "field_declaration" {
            if rust_visibility(file, node).as_deref() == Some("pub") {
                push_decl(
                    file,
                    node,
                    ApiSymbolKind::Field,
                    "Rust explicit visibility: pub".to_owned(),
                    out,
                );
            }
            continue;
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
}

fn rust_kind(node: Node<'_>) -> ApiSymbolKind {
    match node.kind() {
        "function_item" => {
            if ancestor_kind(node, "impl_item") {
                ApiSymbolKind::Method
            } else {
                ApiSymbolKind::Function
            }
        }
        "struct_item" | "enum_item" | "union_item" | "trait_item" | "type_item" => {
            ApiSymbolKind::Type
        }
        "const_item" | "static_item" => ApiSymbolKind::Constant,
        _ => ApiSymbolKind::Other,
    }
}

fn walk_ecmascript(file: &SourceFile, node: Node<'_>, out: &mut Vec<ApiSymbol>) {
    if matches!(node.kind(), "export_statement" | "export_declaration") {
        collect_export(file, node, out);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_ecmascript(file, child, out);
    }
}

fn collect_export(file: &SourceFile, export: Node<'_>, out: &mut Vec<ApiSymbol>) {
    let raw = text(file, export);
    let basis = if raw.trim_start().starts_with("export default") {
        "ECMAScript export default syntax"
    } else if raw.contains("export type") {
        "ECMAScript type-only export syntax"
    } else {
        "ECMAScript named export syntax"
    };
    let mut declarations = Vec::new();
    let mut cursor = export.walk();
    for child in export.named_children(&mut cursor) {
        if is_ecma_declaration(child.kind()) {
            declarations.push(child);
        }
    }
    if !declarations.is_empty() {
        for declaration in declarations {
            let declaration_kind = ecma_kind(declaration);
            if matches!(
                declaration.kind(),
                "lexical_declaration" | "variable_declaration"
            ) {
                collect_exported_bindings(file, declaration, basis, out);
            } else if declaration_name(file, declaration).is_some() {
                push_decl(file, declaration, declaration_kind, basis.to_owned(), out);
            } else if raw.trim_start().starts_with("export default") {
                push_named(
                    file,
                    declaration,
                    "default".to_owned(),
                    declaration_kind,
                    basis.to_owned(),
                    parameter_count(declaration),
                    generic_count(declaration),
                    out,
                );
            }
            if matches!(
                declaration.kind(),
                "class"
                    | "class_declaration"
                    | "class_expression"
                    | "abstract_class_declaration"
                    | "interface_declaration"
            ) {
                collect_ecma_members(file, declaration, basis, out);
            }
        }
        return;
    }

    // Export clauses and star exports are intentionally rows of their own: resolving
    // their local or remote declaration would make this analyzer non-local.
    let mut names = BTreeSet::new();
    collect_export_names(file, export, &mut names);
    if names.is_empty() && raw.trim_start().starts_with("export default") {
        names.insert("default".to_owned());
    }
    if names.is_empty() && raw.contains('*') {
        names.insert("*".to_owned());
    }
    for name in names {
        push_named(
            file,
            export,
            name,
            ApiSymbolKind::ReExport,
            basis.to_owned(),
            0,
            0,
            out,
        );
    }
}

fn collect_exported_bindings(
    file: &SourceFile,
    declaration: Node<'_>,
    basis: &str,
    out: &mut Vec<ApiSymbol>,
) {
    let mut cursor = declaration.walk();
    for child in declaration.named_children(&mut cursor) {
        if child.kind() == "variable_declarator"
            && let Some(name) = declaration_name(file, child)
        {
            let value = child.child_by_field_name("value");
            let kind = value.map_or(ApiSymbolKind::Constant, |n| match n.kind() {
                "arrow_function" | "function_expression" => ApiSymbolKind::Function,
                "class" | "class_expression" => ApiSymbolKind::Type,
                _ => ApiSymbolKind::Constant,
            });
            let params = value.map_or(0, parameter_count);
            let generics = value.map_or(0, generic_count);
            push_named(
                file,
                child,
                name,
                kind,
                basis.to_owned(),
                params,
                generics,
                out,
            );
        }
    }
}

fn collect_ecma_members(
    file: &SourceFile,
    declaration: Node<'_>,
    export_basis: &str,
    out: &mut Vec<ApiSymbol>,
) {
    let mut stack = vec![declaration];
    while let Some(node) = stack.pop() {
        if node != declaration
            && matches!(
                node.kind(),
                "method_definition"
                    | "method_signature"
                    | "abstract_method_signature"
                    | "public_field_definition"
                    | "field_definition"
                    | "property_signature"
            )
        {
            let raw = text(file, node);
            let name = declaration_name(file, node);
            let explicitly_non_public = raw.trim_start().starts_with("private ")
                || raw.trim_start().starts_with("protected ")
                || name.as_deref().is_some_and(|value| value.starts_with('#'));
            if !explicitly_non_public && let Some(name) = name {
                let kind = if node.kind().contains("method") {
                    ApiSymbolKind::Method
                } else {
                    ApiSymbolKind::Field
                };
                push_named(
                    file,
                    node,
                    name,
                    kind,
                    format!("{export_basis}; public/default member visibility"),
                    parameter_count(node),
                    generic_count(node),
                    out,
                );
            }
            continue;
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
}

fn collect_export_names(file: &SourceFile, node: Node<'_>, names: &mut BTreeSet<String>) {
    if matches!(node.kind(), "export_specifier" | "namespace_export") {
        let name = node
            .child_by_field_name("alias")
            .or_else(|| node.child_by_field_name("name"));
        if let Some(name) = name {
            names.insert(text(file, name));
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_export_names(file, child, names);
    }
}

fn is_ecma_declaration(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "generator_function_declaration"
            | "function_expression"
            | "generator_function"
            | "arrow_function"
            | "class"
            | "class_expression"
            | "class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration"
            | "lexical_declaration"
            | "variable_declaration"
            | "abstract_class_declaration"
            | "module"
    )
}

fn ecma_kind(node: Node<'_>) -> ApiSymbolKind {
    match node.kind() {
        "function_declaration"
        | "generator_function_declaration"
        | "function_expression"
        | "generator_function"
        | "arrow_function" => ApiSymbolKind::Function,
        "class"
        | "class_expression"
        | "class_declaration"
        | "abstract_class_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration" => ApiSymbolKind::Type,
        "lexical_declaration" | "variable_declaration" => ApiSymbolKind::Constant,
        _ => ApiSymbolKind::Other,
    }
}

fn walk_go(file: &SourceFile, node: Node<'_>, out: &mut Vec<ApiSymbol>) {
    match node.kind() {
        "function_declaration" => push_go_if_exported(file, node, ApiSymbolKind::Function, out),
        "method_declaration" => push_go_if_exported(file, node, ApiSymbolKind::Method, out),
        "type_spec" => push_go_if_exported(file, node, ApiSymbolKind::Type, out),
        "const_spec" | "var_spec" => collect_go_names(file, node, ApiSymbolKind::Constant, out),
        "field_declaration" => collect_go_names(file, node, ApiSymbolKind::Field, out),
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_go(file, child, out);
    }
}

fn push_go_if_exported(
    file: &SourceFile,
    node: Node<'_>,
    kind: ApiSymbolKind,
    out: &mut Vec<ApiSymbol>,
) {
    if let Some(name) = declaration_name(file, node)
        && starts_uppercase(&name)
    {
        push_named(
            file,
            node,
            name,
            kind,
            "Go exported-name capitalization lexical proxy".to_owned(),
            parameter_count(node),
            generic_count(node),
            out,
        );
    }
}

fn collect_go_names(
    file: &SourceFile,
    node: Node<'_>,
    kind: ApiSymbolKind,
    out: &mut Vec<ApiSymbol>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "field_identifier"
            || (kind == ApiSymbolKind::Constant && child.kind() == "identifier")
        {
            let name = text(file, child);
            if starts_uppercase(&name) {
                push_named(
                    file,
                    child,
                    name,
                    kind,
                    "Go exported-name capitalization lexical proxy".to_owned(),
                    0,
                    0,
                    out,
                );
            }
        }
    }
}

fn walk_python_module(file: &SourceFile, root: Node<'_>, out: &mut Vec<ApiSymbol>) {
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        collect_python_top_level(file, child, out);
    }
}

fn collect_python_top_level(file: &SourceFile, node: Node<'_>, out: &mut Vec<ApiSymbol>) {
    match node.kind() {
        "decorated_definition" => {
            if let Some(definition) = node
                .child_by_field_name("definition")
                .or_else(|| last_named_child(node))
            {
                push_python_decl(file, definition, out);
            }
        }
        "function_definition" | "class_definition" => push_python_decl(file, node, out),
        "expression_statement" => {
            if let Some(assignment) = node.named_child(0)
                && matches!(assignment.kind(), "assignment" | "annotated_assignment")
                && let Some(name) = declaration_name(file, assignment)
                && !name.starts_with('_')
            {
                let kind = if name.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()) {
                    ApiSymbolKind::Constant
                } else {
                    ApiSymbolKind::Other
                };
                push_named(
                    file,
                    node,
                    name,
                    kind,
                    "Python module-level non-underscore convention proxy".to_owned(),
                    0,
                    0,
                    out,
                );
            }
        }
        _ => {}
    }
}

fn push_python_decl(file: &SourceFile, node: Node<'_>, out: &mut Vec<ApiSymbol>) {
    if let Some(name) = declaration_name(file, node)
        && !name.starts_with('_')
    {
        let kind = if node.kind() == "class_definition" {
            ApiSymbolKind::Type
        } else {
            ApiSymbolKind::Function
        };
        push_named(
            file,
            node,
            name,
            kind,
            "Python module-level non-underscore convention proxy".to_owned(),
            parameter_count(node),
            generic_count(node),
            out,
        );
    }
}

fn push_decl(
    file: &SourceFile,
    node: Node<'_>,
    kind: ApiSymbolKind,
    basis: String,
    out: &mut Vec<ApiSymbol>,
) {
    if let Some(name) = declaration_name(file, node) {
        push_named(
            file,
            node,
            name,
            kind,
            basis,
            parameter_count(node),
            generic_count(node),
            out,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_named(
    file: &SourceFile,
    node: Node<'_>,
    symbol: String,
    kind: ApiSymbolKind,
    basis: String,
    parameter_count: usize,
    generic_count: usize,
    out: &mut Vec<ApiSymbol>,
) {
    out.push(ApiSymbol {
        path: file.path.clone(),
        line: node.start_position().row + 1,
        language: file.language,
        symbol,
        kind,
        visibility_or_proxy_basis: basis,
        parameter_count,
        generic_or_type_parameter_count: generic_count,
        documentation_immediately_precedes: has_adjacent_documentation(file, node),
    });
}

fn declaration_name(file: &SourceFile, node: Node<'_>) -> Option<String> {
    node.child_by_field_name("name")
        .or_else(|| node.child_by_field_name("property"))
        .or_else(|| node.child_by_field_name("declarator"))
        .or_else(|| node.child_by_field_name("left"))
        .map(|name| text(file, name))
        .filter(|name| is_simple_name(name))
}

fn parameter_count(node: Node<'_>) -> usize {
    node.child_by_field_name("parameters")
        .map_or(0, |parameters| {
            let mut cursor = parameters.walk();

            parameters
                .named_children(&mut cursor)
                .filter(|child| !matches!(child.kind(), "comment" | "type_parameters"))
                .count()
        })
}

fn generic_count(node: Node<'_>) -> usize {
    node.child_by_field_name("type_parameters")
        .map_or(0, |parameters| {
            let mut cursor = parameters.walk();

            parameters
                .named_children(&mut cursor)
                .filter(|child| child.kind() != "comment")
                .count()
        })
}

fn has_adjacent_documentation(file: &SourceFile, node: Node<'_>) -> bool {
    let declaration_line = node.start_position().row;
    if declaration_line == 0 {
        return false;
    }
    let lines: Vec<&[u8]> = file.bytes.split(|byte| *byte == b'\n').collect();
    let mut index = declaration_line;
    let mut saw_doc = false;
    while index > 0 {
        index -= 1;
        let line = String::from_utf8_lossy(lines.get(index).copied().unwrap_or_default());
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return saw_doc;
        }
        let is_doc = match file.language {
            SourceLanguage::Rust => {
                trimmed.starts_with("///")
                    || trimmed.starts_with("//!")
                    || trimmed.starts_with("#[doc")
            }
            SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
                trimmed.starts_with("/**") || trimmed.starts_with('*') || trimmed.ends_with("*/")
            }
            SourceLanguage::Go => {
                trimmed.starts_with("//")
                    || trimmed.starts_with("/*")
                    || trimmed.starts_with('*')
                    || trimmed.ends_with("*/")
            }
            SourceLanguage::Python => trimmed.starts_with('#'),
        };
        if is_doc {
            saw_doc = true;
            if matches!(
                file.language,
                SourceLanguage::JavaScript
                    | SourceLanguage::TypeScript
                    | SourceLanguage::Tsx
                    | SourceLanguage::Go
            ) && (trimmed.starts_with("/**") || trimmed.starts_with("/*"))
            {
                return true;
            }
            continue;
        }
        // Rust attributes and Python decorators may occur between docs and declarations.
        if (file.language == SourceLanguage::Rust && trimmed.starts_with("#["))
            || (file.language == SourceLanguage::Python && trimmed.starts_with('@'))
        {
            continue;
        }
        return saw_doc;
    }
    saw_doc
}

fn count_source_lines(bytes: &[u8]) -> usize {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| line.iter().any(|byte| !byte.is_ascii_whitespace()))
        .count()
}

fn named_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();

    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn last_named_child(node: Node<'_>) -> Option<Node<'_>> {
    (0..node.named_child_count())
        .rev()
        .filter_map(|index| u32::try_from(index).ok())
        .find_map(|index| node.named_child(index))
}

fn ancestor_kind(mut node: Node<'_>, kind: &str) -> bool {
    while let Some(parent) = node.parent() {
        if parent.kind() == kind {
            return true;
        }
        node = parent;
    }
    false
}

fn starts_uppercase(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

fn is_simple_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c == '_' || c == '$' || c.is_alphanumeric())
}

fn text(file: &SourceFile, node: Node<'_>) -> String {
    String::from_utf8_lossy(&file.bytes[node.byte_range()]).into_owned()
}
