//! Deterministic per-function shape profiles over tree-sitter ASTs.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use thiserror::Error;
use tree_sitter::Node;

use crate::source::{SourceError, SourceFile, SourceLanguage, load_source_tree, parse_source};

#[derive(Debug, Error)]
pub enum ShapeError {
    #[error(transparent)]
    Source(#[from] SourceError),
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionShape {
    pub path: String,
    pub language: SourceLanguage,
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub syntax_errors: bool,
    pub params: u32,
    pub type_parameters: u32,
    pub non_unit_return: bool,
    pub method_self: bool,
    pub interface_width: u32,
    pub interior_volume: u32,
    pub shallow: bool,
    pub cyclomatic: u32,
    pub cognitive: u32,
    pub cognitive_gap: i64,
    pub max_nesting_depth: u32,
    pub max_arm_size_ratio: Option<f64>,
    pub no_else_large_then_arms: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct IntegerDistribution {
    pub observations: u64,
    pub min: Option<i64>,
    pub p50: Option<i64>,
    pub p90: Option<i64>,
    pub max: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct OptionalFloatDistribution {
    pub observations: u64,
    pub min: Option<f64>,
    pub p50: Option<f64>,
    pub p90: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ShapeDistributions {
    pub interface_width: IntegerDistribution,
    pub interior_volume: IntegerDistribution,
    pub cyclomatic: IntegerDistribution,
    pub cognitive: IntegerDistribution,
    pub cognitive_gap: IntegerDistribution,
    pub max_nesting_depth: IntegerDistribution,
    pub max_arm_size_ratio: OptionalFloatDistribution,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileShape {
    pub path: String,
    pub language: SourceLanguage,
    pub syntax_errors: bool,
    pub functions_analyzed: u64,
    pub shallow_functions: u64,
    pub no_else_large_then_arms: u64,
    pub distributions: ShapeDistributions,
}

#[derive(Debug, Clone, Serialize)]
pub struct LanguageShapeCoverage {
    pub language: SourceLanguage,
    pub files_analyzed: u64,
    pub functions_analyzed: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShapeCoverage {
    pub enumerated_files: usize,
    pub supported_files: usize,
    pub skipped_unsupported_files: usize,
    pub syntax_error_files: u64,
    pub functions_analyzed: u64,
    pub functions_per_language: Vec<LanguageShapeCoverage>,
    pub shallow_functions: u64,
    pub shallow_denominator: u64,
    pub no_else_large_then_arms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShapeReport {
    pub root: String,
    pub analyzer: String,
    pub epistemic_class: String,
    pub coverage: ShapeCoverage,
    pub distributions: ShapeDistributions,
    pub files: Vec<FileShape>,
    pub functions: Vec<FunctionShape>,
    pub limitations: Vec<String>,
}

pub fn analyze_shape(input: &Path) -> Result<ShapeReport, ShapeError> {
    let tree = load_source_tree(input)?;
    let mut functions = Vec::new();
    let mut files = Vec::with_capacity(tree.files.len());
    let mut syntax_error_files = 0u64;
    let mut language_files: BTreeMap<SourceLanguage, u64> = BTreeMap::new();

    for file in &tree.files {
        let parsed = parse_source(file)?;
        syntax_error_files += u64::from(parsed.has_syntax_errors);
        *language_files.entry(file.language).or_default() += 1;
        let mut rows = Vec::new();
        collect_functions(file, parsed.tree.root_node(), &mut rows);
        rows.sort_by(function_order);
        files.push(file_shape(file, parsed.has_syntax_errors, &rows));
        functions.append(&mut rows);
    }
    functions.sort_by(function_order);
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let mut language_functions: BTreeMap<SourceLanguage, u64> = BTreeMap::new();
    for function in &functions {
        *language_functions.entry(function.language).or_default() += 1;
    }
    let functions_per_language = language_files
        .into_iter()
        .map(|(language, files_analyzed)| LanguageShapeCoverage {
            language,
            files_analyzed,
            functions_analyzed: language_functions.get(&language).copied().unwrap_or(0),
        })
        .collect();
    let functions_analyzed = functions.len() as u64;
    let shallow_functions = functions.iter().filter(|row| row.shallow).count() as u64;
    let no_else_large_then_arms = functions
        .iter()
        .map(|row| u64::from(row.no_else_large_then_arms))
        .sum();

    Ok(ShapeReport {
        root: tree.root,
        analyzer: "tree-sitter-function-shape-v1".to_owned(),
        epistemic_class: "exact-on-AST counts; proxies for reader experience".to_owned(),
        coverage: ShapeCoverage {
            enumerated_files: tree.enumerated,
            supported_files: tree.files.len(),
            skipped_unsupported_files: tree.skipped,
            syntax_error_files,
            functions_analyzed,
            functions_per_language,
            shallow_functions,
            shallow_denominator: functions_analyzed,
            no_else_large_then_arms,
        },
        distributions: distributions(&functions),
        files,
        functions,
        limitations: vec![
            "Macro expansion and generated code are not observed beyond syntax present in the analyzed files; code generation can hide both interface and interior shape.".to_owned(),
            "Cognitive complexity has several published conventions. This analyzer uses the explicit Campbell/SonarSource-style increment rules documented in INSTRUMENTS.md; comparisons require the same analyzer version and rules.".to_owned(),
            "Interface width counts syntactic surface, not information content: a HashMap<String, Any> parameter has width 1 while potentially leaking an unbounded protocol.".to_owned(),
            "All values are exact for the error-tolerant tree-sitter AST, but module depth, cognitive burden, nesting burden, and branch uniformity are proxies for reader experience rather than direct observations of it.".to_owned(),
            "Files with syntax errors stay in every file/function denominator and are flagged; recovery nodes can make their counts partial.".to_owned(),
            "Lexical same-name calls are the recursion proxy; unresolved aliases, dynamic dispatch, and mutual recursion are not detected, while an unrelated same-name receiver call can overcount.".to_owned(),
        ],
    })
}

pub fn rank_functions(report: &ShapeReport, top: usize) -> Vec<&FunctionShape> {
    let mut rows: Vec<_> = report.functions.iter().collect();
    rows.sort_by(|left, right| {
        right
            .cognitive_gap
            .cmp(&left.cognitive_gap)
            .then_with(|| right.max_nesting_depth.cmp(&left.max_nesting_depth))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.start_line.cmp(&right.start_line))
            .then_with(|| left.name.cmp(&right.name))
    });
    rows.truncate(top);
    rows
}

fn function_order(left: &FunctionShape, right: &FunctionShape) -> Ordering {
    left.path
        .cmp(&right.path)
        .then_with(|| left.start_line.cmp(&right.start_line))
        .then_with(|| left.name.cmp(&right.name))
}

fn file_shape(file: &SourceFile, syntax_errors: bool, rows: &[FunctionShape]) -> FileShape {
    FileShape {
        path: file.path.clone(),
        language: file.language,
        syntax_errors,
        functions_analyzed: rows.len() as u64,
        shallow_functions: rows.iter().filter(|row| row.shallow).count() as u64,
        no_else_large_then_arms: rows
            .iter()
            .map(|row| u64::from(row.no_else_large_then_arms))
            .sum(),
        distributions: distributions(rows),
    }
}

fn distributions(rows: &[FunctionShape]) -> ShapeDistributions {
    ShapeDistributions {
        interface_width: integer_distribution(
            rows.iter().map(|row| i64::from(row.interface_width)),
        ),
        interior_volume: integer_distribution(
            rows.iter().map(|row| i64::from(row.interior_volume)),
        ),
        cyclomatic: integer_distribution(rows.iter().map(|row| i64::from(row.cyclomatic))),
        cognitive: integer_distribution(rows.iter().map(|row| i64::from(row.cognitive))),
        cognitive_gap: integer_distribution(rows.iter().map(|row| row.cognitive_gap)),
        max_nesting_depth: integer_distribution(
            rows.iter().map(|row| i64::from(row.max_nesting_depth)),
        ),
        max_arm_size_ratio: float_distribution(
            rows.iter().filter_map(|row| row.max_arm_size_ratio),
        ),
    }
}

fn integer_distribution(values: impl Iterator<Item = i64>) -> IntegerDistribution {
    let mut values: Vec<_> = values.collect();
    values.sort_unstable();
    if values.is_empty() {
        return IntegerDistribution::default();
    }
    IntegerDistribution {
        observations: values.len() as u64,
        min: values.first().copied(),
        p50: Some(nearest_rank_i64(&values, 50)),
        p90: Some(nearest_rank_i64(&values, 90)),
        max: values.last().copied(),
    }
}

fn float_distribution(values: impl Iterator<Item = f64>) -> OptionalFloatDistribution {
    let mut values: Vec<_> = values.collect();
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    if values.is_empty() {
        return OptionalFloatDistribution::default();
    }
    OptionalFloatDistribution {
        observations: values.len() as u64,
        min: values.first().copied(),
        p50: nearest_rank_f64(&values, 50),
        p90: nearest_rank_f64(&values, 90),
        max: values.last().copied(),
    }
}

fn nearest_rank_i64(values: &[i64], percentile: usize) -> i64 {
    let index = (percentile * values.len()).div_ceil(100).saturating_sub(1);
    values[index]
}

fn nearest_rank_f64(values: &[f64], percentile: usize) -> Option<f64> {
    let index = (percentile * values.len()).div_ceil(100).saturating_sub(1);
    values.get(index).copied()
}

fn collect_functions(file: &SourceFile, node: Node<'_>, rows: &mut Vec<FunctionShape>) {
    if is_function_space(file.language, node.kind()) {
        rows.push(analyze_function(file, node));
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_functions(file, child, rows);
    }
}

fn analyze_function(file: &SourceFile, node: Node<'_>) -> FunctionShape {
    let name = function_name(file, node);
    let (params, method_self) = parameter_shape(file, node);
    let type_parameters = count_type_parameters(file.language, node);
    let non_unit_return = has_non_unit_return(file, node);
    let (
        interior_volume,
        cyclomatic,
        cognitive,
        max_nesting_depth,
        max_arm_size_ratio,
        no_else_large_then_arms,
    ) = {
        let mut walk = FunctionWalk::new(file, node, &name);
        let body = node.child_by_field_name("body").unwrap_or(node);
        walk.visit(body, 0, true);
        (
            walk.statements,
            walk.cyclomatic,
            walk.cognitive,
            walk.max_nesting,
            walk.max_arm_ratio,
            walk.no_else_large_then_arms,
        )
    };
    let interface_width =
        params + type_parameters + u32::from(non_unit_return) + u32::from(method_self);
    let shallow = interior_volume > 0 && interface_width >= interior_volume;

    FunctionShape {
        path: file.path.clone(),
        language: file.language,
        name,
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        syntax_errors: node.has_error(),
        params,
        type_parameters,
        non_unit_return,
        method_self,
        interface_width,
        interior_volume,
        shallow,
        cyclomatic,
        cognitive,
        cognitive_gap: i64::from(cognitive) - i64::from(cyclomatic),
        max_nesting_depth,
        max_arm_size_ratio,
        no_else_large_then_arms,
    }
}

struct FunctionWalk<'a> {
    file: &'a SourceFile,
    root: Node<'a>,
    function_name: &'a str,
    statements: u32,
    cyclomatic: u32,
    cognitive: u32,
    max_nesting: u32,
    max_arm_ratio: Option<f64>,
    no_else_large_then_arms: u32,
}

impl<'a> FunctionWalk<'a> {
    fn new(file: &'a SourceFile, root: Node<'a>, function_name: &'a str) -> Self {
        Self {
            file,
            root,
            function_name,
            statements: 0,
            cyclomatic: 1,
            cognitive: 0,
            max_nesting: 0,
            max_arm_ratio: None,
            no_else_large_then_arms: 0,
        }
    }

    fn visit(&mut self, node: Node<'a>, nesting: u32, is_root_body: bool) {
        if node.id() != self.root.id() && is_function_space(self.file.language, node.kind()) {
            return;
        }
        if is_statement_kind(self.file.language, node.kind()) {
            self.statements += 1;
        }
        if self.is_recursive_call(node) {
            self.cognitive += 1;
        }

        let kind = node.kind();
        if is_if(self.file.language, kind) {
            self.cyclomatic += 1;
            self.cognitive += 1 + nesting;
            self.max_nesting = self.max_nesting.max(nesting + 1);
            self.observe_if_symmetry(node);
            self.visit_if_children(node, nesting);
            return;
        }
        if is_loop(self.file.language, kind) {
            self.cyclomatic += 1;
            self.cognitive += 1 + nesting;
            self.max_nesting = self.max_nesting.max(nesting + 1);
            self.visit_control_children(node, nesting);
            return;
        }
        if is_match_or_switch(self.file.language, kind) {
            self.cognitive += 1 + nesting;
            self.max_nesting = self.max_nesting.max(nesting + 1);
            let arms = branch_arms(self.file.language, node);
            self.cyclomatic += arms.len() as u32;
            self.observe_arm_sizes(&arms);
            self.visit_control_children(node, nesting);
            return;
        }
        if is_catch(self.file.language, kind) {
            self.cyclomatic += 1;
            self.cognitive += 1 + nesting;
            self.max_nesting = self.max_nesting.max(nesting + 1);
            self.visit_control_children(node, nesting);
            return;
        }
        if is_logical_operator(self.file, node) {
            self.cyclomatic += 1;
            if starts_boolean_sequence(self.file, node) {
                self.cognitive += 1 + nesting;
            }
        }
        let next_nesting = if !is_root_body && is_standalone_block(node) {
            self.max_nesting = self.max_nesting.max(nesting + 1);
            nesting + 1
        } else {
            nesting
        };
        self.visit_children(node, next_nesting);
    }

    fn visit_children(&mut self, node: Node<'a>, nesting: u32) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit(child, nesting, false);
        }
    }

    fn visit_if_children(&mut self, node: Node<'a>, nesting: u32) {
        let consequence = node
            .child_by_field_name("consequence")
            .or_else(|| node.child_by_field_name("body"));
        let alternative = node.child_by_field_name("alternative");
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            let is_else_if = alternative.is_some_and(|alt| {
                (alt.id() == child.id() && is_if(self.file.language, child.kind()))
                    || (alt.id() == child.id() && child.kind() == "elif_clause")
            });
            let child_nesting = if is_else_if {
                nesting
            } else if consequence.is_some_and(|body| body.id() == child.id())
                || alternative.is_some_and(|body| body.id() == child.id())
            {
                nesting + 1
            } else {
                nesting
            };
            self.visit(child, child_nesting, false);
        }
    }

    fn visit_control_children(&mut self, node: Node<'a>, nesting: u32) {
        let body = node
            .child_by_field_name("body")
            .or_else(|| node.child_by_field_name("consequence"));
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            let child_nesting = if body.is_some_and(|value| value.id() == child.id()) {
                nesting + 1
            } else {
                nesting
            };
            self.visit(child, child_nesting, false);
        }
    }

    fn observe_if_symmetry(&mut self, node: Node<'a>) {
        if is_else_if_child(node) {
            return;
        }
        let arms = if_arms(self.file.language, node);
        if arms.len() >= 2 {
            self.observe_arm_sizes(&arms);
        } else if let Some(then_arm) = arms.first()
            && statement_count(self.file.language, *then_arm, self.root) >= 8
        {
            self.no_else_large_then_arms += 1;
        }
    }

    fn observe_arm_sizes(&mut self, arms: &[Node<'a>]) {
        let sizes: Vec<_> = arms
            .iter()
            .map(|arm| statement_count(self.file.language, *arm, self.root))
            .filter(|size| *size >= 1)
            .collect();
        if sizes.len() < 2 {
            return;
        }
        let smallest = sizes.iter().min().copied().unwrap_or(1);
        let largest = sizes.iter().max().copied().unwrap_or(smallest);
        let ratio = f64::from(largest) / f64::from(smallest);
        self.max_arm_ratio = Some(self.max_arm_ratio.map_or(ratio, |old| old.max(ratio)));
    }

    fn is_recursive_call(&self, node: Node<'_>) -> bool {
        if !is_call(self.file.language, node.kind()) {
            return false;
        }
        node.child_by_field_name("function")
            .or_else(|| node.child_by_field_name("method"))
            .and_then(last_identifier)
            .is_some_and(|callee| text(self.file, callee) == self.function_name)
    }
}

fn is_function_space(language: SourceLanguage, kind: &str) -> bool {
    match language {
        SourceLanguage::Rust => matches!(kind, "function_item" | "closure_expression"),
        SourceLanguage::Python => matches!(kind, "function_definition" | "lambda"),
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => matches!(
            kind,
            "function_declaration"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
                | "generator_function"
                | "generator_function_declaration"
        ),
        SourceLanguage::Go => matches!(
            kind,
            "function_declaration" | "method_declaration" | "func_literal"
        ),
    }
}

fn function_name(file: &SourceFile, node: Node<'_>) -> String {
    node.child_by_field_name("name")
        .map(|name| text(file, name).to_owned())
        .unwrap_or_else(|| {
            if node.kind() == "lambda" {
                "<lambda>"
            } else {
                "<closure>"
            }
            .to_owned()
        })
}

fn parameter_shape(file: &SourceFile, node: Node<'_>) -> (u32, bool) {
    let method_self = match file.language {
        SourceLanguage::Rust => node
            .child_by_field_name("parameters")
            .is_some_and(|parameters| has_named_descendant(parameters, "self_parameter")),
        SourceLanguage::Python => python_method(node),
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            node.kind() == "method_definition"
        }
        SourceLanguage::Go => node.kind() == "method_declaration",
    };
    let parameters = node
        .child_by_field_name("parameters")
        .or_else(|| node.child_by_field_name("parameter"));
    let mut count = parameters.map_or(0, |params| count_parameters(file.language, params));
    if file.language == SourceLanguage::Python && method_self && count > 0 {
        count -= 1;
    }
    (count, method_self)
}

fn count_parameters(language: SourceLanguage, node: Node<'_>) -> u32 {
    if matches!(node.kind(), "identifier" | "parameter") {
        return 1;
    }
    let mut count = 0;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        count += match language {
            SourceLanguage::Rust => u32::from(child.kind() == "parameter"),
            SourceLanguage::Python => u32::from(matches!(
                child.kind(),
                "identifier"
                    | "default_parameter"
                    | "typed_parameter"
                    | "typed_default_parameter"
                    | "list_splat_pattern"
                    | "dictionary_splat_pattern"
            )),
            SourceLanguage::JavaScript => u32::from(matches!(
                child.kind(),
                "identifier"
                    | "assignment_pattern"
                    | "object_pattern"
                    | "array_pattern"
                    | "rest_pattern"
            )),
            SourceLanguage::TypeScript | SourceLanguage::Tsx => u32::from(matches!(
                child.kind(),
                "required_parameter"
                    | "optional_parameter"
                    | "identifier"
                    | "assignment_pattern"
                    | "object_pattern"
                    | "array_pattern"
                    | "rest_pattern"
            )),
            SourceLanguage::Go => {
                if matches!(
                    child.kind(),
                    "parameter_declaration" | "variadic_parameter_declaration"
                ) {
                    let mut names = child.walk();
                    child
                        .children_by_field_name("name", &mut names)
                        .count()
                        .max(1) as u32
                } else {
                    0
                }
            }
        };
    }
    count
}

fn python_method(node: Node<'_>) -> bool {
    let mut parent = node.parent();
    while let Some(current) = parent {
        if current.kind() == "class_definition" {
            return true;
        }
        if is_function_space(SourceLanguage::Python, current.kind()) {
            return false;
        }
        parent = current.parent();
    }
    false
}

fn count_type_parameters(language: SourceLanguage, node: Node<'_>) -> u32 {
    let Some(parameters) = node
        .child_by_field_name("type_parameters")
        .or_else(|| node.child_by_field_name("type_parameter"))
    else {
        return 0;
    };
    count_type_parameter_nodes(language, parameters)
}

fn count_type_parameter_nodes(language: SourceLanguage, node: Node<'_>) -> u32 {
    let own = match language {
        SourceLanguage::Rust => u32::from(matches!(
            node.kind(),
            "type_parameter" | "const_parameter" | "lifetime"
        )),
        SourceLanguage::Python | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            u32::from(node.kind() == "type_parameter")
        }
        SourceLanguage::Go if node.kind() == "type_parameter_declaration" => {
            let mut cursor = node.walk();
            node.children_by_field_name("name", &mut cursor)
                .count()
                .max(1) as u32
        }
        SourceLanguage::JavaScript | SourceLanguage::Go => 0,
    };
    if own != 0 {
        return own;
    }
    let mut cursor = node.walk();
    own + node
        .named_children(&mut cursor)
        .map(|child| count_type_parameter_nodes(language, child))
        .sum::<u32>()
}

fn has_non_unit_return(file: &SourceFile, node: Node<'_>) -> bool {
    match file.language {
        SourceLanguage::Rust => node
            .child_by_field_name("return_type")
            .is_some_and(|value| text(file, value).trim() != "()"),
        SourceLanguage::Go => node.child_by_field_name("result").is_some(),
        SourceLanguage::TypeScript | SourceLanguage::Tsx => node
            .child_by_field_name("return_type")
            .is_some_and(|value| !text(file, value).trim().ends_with("void")),
        SourceLanguage::JavaScript | SourceLanguage::Python => {
            if node.kind() == "lambda"
                || (node.kind() == "arrow_function"
                    && node
                        .child_by_field_name("body")
                        .is_some_and(|body| body.kind() != "statement_block"))
            {
                true
            } else {
                has_return_value(file, node, node)
            }
        }
    }
}

fn has_return_value(file: &SourceFile, node: Node<'_>, root: Node<'_>) -> bool {
    if node.id() != root.id() && is_function_space(file.language, node.kind()) {
        return false;
    }
    if node.kind() == "return_statement" {
        return node.named_child_count() > 0;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| has_return_value(file, child, root))
}

fn is_statement_kind(language: SourceLanguage, kind: &str) -> bool {
    match language {
        SourceLanguage::Rust => matches!(kind, "expression_statement" | "let_declaration"),
        SourceLanguage::Python => kind.ends_with("_statement"),
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            kind.ends_with("_statement")
                || matches!(kind, "lexical_declaration" | "variable_declaration")
        }
        SourceLanguage::Go => {
            kind.ends_with("_statement")
                || matches!(
                    kind,
                    "short_var_declaration" | "var_declaration" | "const_declaration"
                )
        }
    }
}

fn is_if(language: SourceLanguage, kind: &str) -> bool {
    match language {
        SourceLanguage::Rust => kind == "if_expression",
        SourceLanguage::Python => matches!(kind, "if_statement" | "elif_clause"),
        SourceLanguage::JavaScript
        | SourceLanguage::TypeScript
        | SourceLanguage::Tsx
        | SourceLanguage::Go => kind == "if_statement",
    }
}

fn is_loop(language: SourceLanguage, kind: &str) -> bool {
    match language {
        SourceLanguage::Rust => matches!(
            kind,
            "for_expression" | "while_expression" | "loop_expression"
        ),
        SourceLanguage::Python => matches!(kind, "for_statement" | "while_statement"),
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => matches!(
            kind,
            "for_statement" | "for_in_statement" | "while_statement" | "do_statement"
        ),
        SourceLanguage::Go => kind == "for_statement",
    }
}

fn is_match_or_switch(language: SourceLanguage, kind: &str) -> bool {
    match language {
        SourceLanguage::Rust => kind == "match_expression",
        SourceLanguage::Python => kind == "match_statement",
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            kind == "switch_statement"
        }
        SourceLanguage::Go => matches!(
            kind,
            "expression_switch_statement" | "type_switch_statement" | "select_statement"
        ),
    }
}

fn is_catch(language: SourceLanguage, kind: &str) -> bool {
    match language {
        SourceLanguage::Rust => false,
        SourceLanguage::Python => kind == "except_clause",
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            kind == "catch_clause"
        }
        SourceLanguage::Go => false,
    }
}

fn is_call(language: SourceLanguage, kind: &str) -> bool {
    match language {
        SourceLanguage::Python => kind == "call",
        SourceLanguage::Rust
        | SourceLanguage::JavaScript
        | SourceLanguage::TypeScript
        | SourceLanguage::Tsx
        | SourceLanguage::Go => kind == "call_expression",
    }
}

fn is_logical_operator(file: &SourceFile, node: Node<'_>) -> bool {
    logical_operator(file, node).is_some()
}

fn logical_operator<'a>(file: &SourceFile, node: Node<'a>) -> Option<&'a str> {
    let kind = node.kind();
    let candidate = match file.language {
        SourceLanguage::Rust
        | SourceLanguage::JavaScript
        | SourceLanguage::TypeScript
        | SourceLanguage::Tsx => kind == "binary_expression",
        SourceLanguage::Python => kind == "boolean_operator",
        SourceLanguage::Go => kind == "binary_expression",
    };
    if !candidate {
        return None;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| matches!(child.kind(), "&&" | "||" | "and" | "or"))
        .map(|child| child.kind())
}

fn starts_boolean_sequence(file: &SourceFile, node: Node<'_>) -> bool {
    let Some(operator) = logical_operator(file, node) else {
        return false;
    };
    node.parent()
        .and_then(|parent| logical_operator(file, parent))
        .is_none_or(|parent_operator| parent_operator != operator)
}

fn is_standalone_block(node: Node<'_>) -> bool {
    if !matches!(node.kind(), "block" | "statement_block") {
        return false;
    }
    node.parent().is_some_and(|parent| {
        !is_function_space_for_any(parent.kind())
            && !matches!(
                parent.kind(),
                "if_expression"
                    | "if_statement"
                    | "elif_clause"
                    | "else_clause"
                    | "for_expression"
                    | "for_statement"
                    | "for_in_statement"
                    | "while_expression"
                    | "while_statement"
                    | "loop_expression"
                    | "do_statement"
                    | "match_expression"
                    | "switch_statement"
                    | "expression_switch_statement"
                    | "type_switch_statement"
                    | "select_statement"
                    | "catch_clause"
                    | "except_clause"
            )
    })
}

fn is_function_space_for_any(kind: &str) -> bool {
    matches!(
        kind,
        "function_item"
            | "closure_expression"
            | "function_definition"
            | "lambda"
            | "function_declaration"
            | "function_expression"
            | "arrow_function"
            | "method_definition"
            | "generator_function"
            | "generator_function_declaration"
            | "method_declaration"
            | "func_literal"
    )
}

fn if_arms(language: SourceLanguage, node: Node<'_>) -> Vec<Node<'_>> {
    let mut arms = Vec::new();
    if let Some(consequence) = node
        .child_by_field_name("consequence")
        .or_else(|| node.child_by_field_name("body"))
    {
        arms.push(consequence);
    }
    let mut alternative = node.child_by_field_name("alternative");
    while let Some(current) = alternative {
        if is_if(language, current.kind()) || current.kind() == "elif_clause" {
            if let Some(consequence) = current
                .child_by_field_name("consequence")
                .or_else(|| current.child_by_field_name("body"))
            {
                arms.push(consequence);
            }
            alternative = current.child_by_field_name("alternative");
        } else {
            arms.push(current.child_by_field_name("body").unwrap_or(current));
            break;
        }
    }
    arms
}

fn is_else_if_child(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        parent
            .child_by_field_name("alternative")
            .is_some_and(|alternative| alternative.id() == node.id())
    }) || node.kind() == "elif_clause"
}

fn branch_arms(language: SourceLanguage, node: Node<'_>) -> Vec<Node<'_>> {
    let arm_kinds: &[&str] = match language {
        SourceLanguage::Rust => &["match_arm"],
        SourceLanguage::Python => &["case_clause"],
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            &["switch_case", "switch_default"]
        }
        SourceLanguage::Go => &[
            "expression_case",
            "type_case",
            "default_case",
            "communication_case",
            "default_communication_case",
        ],
    };
    let mut arms = Vec::new();
    collect_arm_nodes(language, node, node, arm_kinds, &mut arms);
    arms
}

fn collect_arm_nodes<'a>(
    language: SourceLanguage,
    root: Node<'a>,
    node: Node<'a>,
    kinds: &[&str],
    output: &mut Vec<Node<'a>>,
) {
    if kinds.contains(&node.kind()) {
        output.push(node);
        return;
    }
    if node.id() != root.id()
        && (is_match_or_switch(language, node.kind()) || is_function_space(language, node.kind()))
    {
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_arm_nodes(language, root, child, kinds, output);
    }
}

fn statement_count(language: SourceLanguage, node: Node<'_>, function_root: Node<'_>) -> u32 {
    if node.id() != function_root.id() && is_function_space(language, node.kind()) {
        return 0;
    }
    let own = u32::from(is_statement_kind(language, node.kind()));
    let mut cursor = node.walk();
    own + node
        .named_children(&mut cursor)
        .map(|child| statement_count(language, child, function_root))
        .sum::<u32>()
}

fn has_named_descendant(node: Node<'_>, kind: &str) -> bool {
    if node.kind() == kind {
        return true;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| has_named_descendant(child, kind))
}

fn last_identifier(node: Node<'_>) -> Option<Node<'_>> {
    if matches!(
        node.kind(),
        "identifier" | "field_identifier" | "property_identifier"
    ) {
        return Some(node);
    }
    let mut result = None;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(identifier) = last_identifier(child) {
            result = Some(identifier);
        }
    }
    result
}

fn text<'a>(file: &'a SourceFile, node: Node<'_>) -> &'a str {
    std::str::from_utf8(&file.bytes[node.byte_range()]).unwrap_or("")
}
