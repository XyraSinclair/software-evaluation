//! Deterministic, syntax-only census of declared type-space shape.
//!
//! This module observes type spellings in declarations and signatures. It does
//! not resolve aliases, infer types, expand macros, or establish algebraic laws.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;
use thiserror::Error;
use tree_sitter::Node;

use crate::source::{SourceError, SourceFile, SourceLanguage, load_source_tree, parse_source};

const ANALYZER: &str = "tree-sitter-type-space-census-v1";
const EPISTEMIC_CLASS: &str = "proxy over declared type syntax";

#[derive(Debug, Error)]
pub enum TypeSpaceError {
    #[error(transparent)]
    Source(#[from] SourceError),
}

#[derive(Debug, Clone, Serialize)]
pub struct TypeSpaceReport {
    pub root: String,
    pub analyzer: String,
    pub epistemic_class: String,
    pub coverage: TypeSpaceCoverage,
    pub t1: AlgebraicShape,
    pub t2: DynamicState,
    pub t3: SignatureParametricity,
    pub t4: EndomorphicClosure,
    pub t5: OwnershipEvasion,
    pub t6: NewtypeAdoption,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TypeSpaceCoverage {
    pub enumerated_files: usize,
    pub supported_files: usize,
    pub skipped_unsupported_files: usize,
    pub syntax_error_files: usize,
    pub files_per_language: BTreeMap<String, usize>,
    pub determinants_per_language: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AlgebraicShape {
    pub all_type_definitions: u64,
    pub structs: u64,
    pub data_bearing_enums: u64,
    pub fieldless_tag_enums: u64,
    pub other_type_definitions: u64,
    pub structs_with_at_least_two_fields: u64,
    pub structs_with_at_least_two_option_bool_fields: u64,
    pub option_bool_fields: u64,
    pub structs_detail: Vec<StructShapeRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StructShapeRow {
    pub path: String,
    pub line: usize,
    pub name: String,
    pub fields: u64,
    pub option_bool_fields: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DynamicState {
    pub dynamic_state_mentions: u64,
    pub type_constructor_leaf_mentions: u64,
    pub by_language: BTreeMap<String, RatioCount>,
    pub items: Vec<DynamicItemRow>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RatioCount {
    pub numerator: u64,
    pub denominator: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DynamicItemRow {
    pub path: String,
    pub line: usize,
    pub name: String,
    pub language: String,
    pub dynamic_mentions: u64,
    pub type_leaf_mentions: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SignatureParametricity {
    pub public_functions: u64,
    pub generic_public_functions: u64,
    pub return_parametric_public_functions: u64,
    pub abstract_type_leaf_mentions: u64,
    pub concrete_type_leaf_mentions: u64,
    pub signature_type_leaf_mentions: u64,
    pub generic_parameters: u64,
    pub generic_washing_parameters: u64,
    pub bounds_per_parameter: IntegerDistribution,
    #[serde(skip)]
    bound_counts: Vec<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct IntegerDistribution {
    pub min: u64,
    pub p50: u64,
    pub p90: u64,
    pub max: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct EndomorphicClosure {
    pub public_methods: u64,
    pub endomorphic_methods: u64,
    pub owned_endomorphic_methods: u64,
    pub borrowed_endomorphic_methods: u64,
    pub mutant_endomorphic_methods: u64,
    pub binary_closures: u64,
    pub functions_censused_for_binary_closure: u64,
    pub types: Vec<EndomorphicTypeRow>,
    pub binary_items: Vec<LocationRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EndomorphicTypeRow {
    pub path: String,
    pub line: usize,
    pub name: String,
    pub public_methods: u64,
    pub endomorphic_methods: u64,
    pub owned_endomorphic_methods: u64,
    pub borrowed_endomorphic_methods: u64,
    pub mutant_endomorphic_methods: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct OwnershipEvasion {
    pub shared_mutable_type_mentions: u64,
    pub borrow_lock_calls: u64,
    pub shared_ownership_type_mentions: u64,
    pub clone_calls: u64,
    pub call_expressions: u64,
    pub type_constructor_leaf_mentions: u64,
    pub files: Vec<OwnershipFileRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnershipFileRow {
    pub path: String,
    pub shared_mutable_type_mentions: u64,
    pub borrow_lock_calls: u64,
    pub shared_ownership_type_mentions: u64,
    pub clone_calls: u64,
    pub call_expressions: u64,
    pub type_constructor_leaf_mentions: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct NewtypeAdoption {
    pub wide_primitive_mentions: u64,
    pub non_primitive_mentions: u64,
    pub public_boundary_type_mentions: u64,
    pub newtype_supply: u64,
    pub costume_newtypes: u64,
    pub primitive_items: Vec<PrimitiveItemRow>,
    pub newtypes: Vec<NewtypeRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrimitiveItemRow {
    pub path: String,
    pub line: usize,
    pub name: String,
    pub primitive_mentions: u64,
    pub type_mentions: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct NewtypeRow {
    pub path: String,
    pub line: usize,
    pub name: String,
    pub wrapped_type: String,
    pub costume: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocationRow {
    pub path: String,
    pub line: usize,
    pub name: String,
}

#[derive(Default)]
struct FileState {
    ownership: OwnershipFileRowBuilder,
}

#[derive(Default)]
struct OwnershipFileRowBuilder {
    shared_mutable_type_mentions: u64,
    borrow_lock_calls: u64,
    shared_ownership_type_mentions: u64,
    clone_calls: u64,
    call_expressions: u64,
    type_constructor_leaf_mentions: u64,
}

#[derive(Clone, Default)]
struct GenericParameter {
    bounds: Vec<String>,
}

pub fn analyze_typespace(input: &Path) -> Result<TypeSpaceReport, TypeSpaceError> {
    let tree = load_source_tree(input)?;
    let mut report = TypeSpaceReport {
        root: tree.root,
        analyzer: ANALYZER.to_owned(),
        epistemic_class: EPISTEMIC_CLASS.to_owned(),
        coverage: TypeSpaceCoverage {
            enumerated_files: tree.enumerated,
            supported_files: tree.files.len(),
            skipped_unsupported_files: tree.skipped,
            determinants_per_language: BTreeMap::from([
                ("go".to_owned(), "none".to_owned()),
                ("javascript".to_owned(), "none".to_owned()),
                (
                    "python".to_owned(),
                    "T2 dynamic-state field/signature list only".to_owned(),
                ),
                ("rust".to_owned(), "T1,T2,T3,T4,T5,T6".to_owned()),
                (
                    "typescript".to_owned(),
                    "T2 dynamic-state field/signature list only".to_owned(),
                ),
            ]),
            ..TypeSpaceCoverage::default()
        },
        t1: AlgebraicShape::default(),
        t2: DynamicState::default(),
        t3: SignatureParametricity::default(),
        t4: EndomorphicClosure::default(),
        t5: OwnershipEvasion::default(),
        t6: NewtypeAdoption::default(),
        limitations: limitations(),
    };

    for file in &tree.files {
        *report
            .coverage
            .files_per_language
            .entry(file.language.name().to_owned())
            .or_default() += 1;
        let parsed = parse_source(file)?;
        report.coverage.syntax_error_files += usize::from(parsed.has_syntax_errors);
        let mut state = FileState::default();
        match file.language {
            SourceLanguage::Rust => {
                walk_rust(file, parsed.tree.root_node(), &mut report, &mut state)
            }
            SourceLanguage::TypeScript | SourceLanguage::Tsx => {
                walk_typescript(file, parsed.tree.root_node(), &mut report)
            }
            SourceLanguage::Python => walk_python(file, parsed.tree.root_node(), &mut report),
            SourceLanguage::JavaScript | SourceLanguage::Go => {}
        }
        if file.language == SourceLanguage::Rust {
            let row = OwnershipFileRow {
                path: file.path.clone(),
                shared_mutable_type_mentions: state.ownership.shared_mutable_type_mentions,
                borrow_lock_calls: state.ownership.borrow_lock_calls,
                shared_ownership_type_mentions: state.ownership.shared_ownership_type_mentions,
                clone_calls: state.ownership.clone_calls,
                call_expressions: state.ownership.call_expressions,
                type_constructor_leaf_mentions: state.ownership.type_constructor_leaf_mentions,
            };
            report.t5.shared_mutable_type_mentions += row.shared_mutable_type_mentions;
            report.t5.borrow_lock_calls += row.borrow_lock_calls;
            report.t5.shared_ownership_type_mentions += row.shared_ownership_type_mentions;
            report.t5.clone_calls += row.clone_calls;
            report.t5.call_expressions += row.call_expressions;
            report.t5.type_constructor_leaf_mentions += row.type_constructor_leaf_mentions;
            report.t5.files.push(row);
        }
    }
    report.t1.structs_detail.sort_by(location_order_struct);
    report.t2.items.sort_by(location_order_dynamic);
    report.t3.bounds_per_parameter = integer_distribution(&mut report.t3.bound_counts);
    report.t4.types.sort_by(location_order_endo);
    report.t4.binary_items.sort_by(location_order);
    report.t5.files.sort_by(|a, b| a.path.cmp(&b.path));
    report.t6.primitive_items.sort_by(location_order_primitive);
    report.t6.newtypes.sort_by(location_order_newtype);
    Ok(report)
}

fn limitations() -> Vec<String> {
    vec![
        "All determinants are proxies over declared type syntax; aliases, imports, inference, macros, and semantic equivalence are unresolved until a rust-analyzer/HIR bridge.".to_owned(),
        "T1: two-variant enums can still encode booleans; optional fields can be hoisted; builder/config structs can legitimately contain orthogonal options.".to_owned(),
        "T2: aliases defeat the list entirely (for example `type Json = Value`); serialization and interpreter domains legitimately use dynamic state. This overlaps discipline's bare-any count but is field/signature-scoped and includes container forms.".to_owned(),
        "T3: Rust itself breaks parametricity through mechanisms including TypeId, downcast, and specialization, so even a resolved implementation remains a proxy. Generic-washing and bound soup are measured explicitly; a domain CLI can legitimately sit near zero, so compare only within-language distributions and never treat the fractions as targets.".to_owned(),
        "T4: syntactic closure does not establish identity, associativity, or any other law; mutable builder returns are reported separately.".to_owned(),
        "T5: the census cannot distinguish necessary concurrency from ownership evasion; hand-written unsafe interior mutability is missed, so cross-check discipline's unsafe count.".to_owned(),
        "T6: a primitive can be the honest domain representation; compare distributions within Rust rather than treating the ratio as a target.".to_owned(),
        "Local state that never crosses a declared boundary remains outside the type-space census.".to_owned(),
    ]
}

fn walk_rust(
    file: &SourceFile,
    node: Node<'_>,
    report: &mut TypeSpaceReport,
    state: &mut FileState,
) {
    match node.kind() {
        "struct_item" => collect_struct(file, node, report),
        "enum_item" => collect_enum(file, node, report),
        "union_item" | "type_item" => {
            report.t1.all_type_definitions += 1;
            report.t1.other_type_definitions += 1;
        }
        "impl_item" => {
            collect_impl(file, node, report);
            collect_rust_ownership(file, node, state);
            return;
        }
        "function_item" => {
            collect_rust_signature(file, node, &BTreeMap::new(), report);
            collect_binary_closure(file, node, None, report);
        }
        "field_declaration" | "ordered_field_declaration" => {
            if has_ancestor(node, "struct_item")
                && let Some(ty) = node
                    .child_by_field_name("type")
                    .or_else(|| node.named_child(0))
            {
                let name = field_text(file, node, "name").unwrap_or_else(|| "<field>".to_owned());
                collect_dynamic_region(file, ty, &name, report);
            }
        }
        "call_expression" => collect_call(file, node, state),
        _ => {}
    }
    if is_rust_type_region(node) {
        collect_ownership_type_region(file, node, state);
        return;
    }
    visit_named(node, |child| walk_rust(file, child, report, state));
}

fn collect_rust_ownership(file: &SourceFile, node: Node<'_>, state: &mut FileState) {
    fn walk(file: &SourceFile, node: Node<'_>, state: &mut FileState) {
        if node.kind() == "call_expression" {
            collect_call(file, node, state);
        }
        if is_rust_type_region(node) {
            collect_ownership_type_region(file, node, state);
            return;
        }
        visit_named(node, |child| walk(file, child, state));
    }
    walk(file, node, state);
}

fn collect_struct(file: &SourceFile, node: Node<'_>, report: &mut TypeSpaceReport) {
    report.t1.all_type_definitions += 1;
    report.t1.structs += 1;
    let name = field_text(file, node, "name").unwrap_or_else(|| "<anonymous>".to_owned());
    let mut field_types = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        collect_field_types(file, body, &mut field_types);
        if body.kind() == "ordered_field_declaration_list" {
            for field_type in &field_types {
                collect_dynamic_text(file, field_type, line(node), &name, report);
            }
        }
    }
    let fields = field_types.len() as u64;
    let option_bool_fields = field_types
        .iter()
        .filter(|type_text| contains_option_or_bool(type_text))
        .count() as u64;
    report.t1.option_bool_fields += option_bool_fields;
    if fields >= 2 {
        report.t1.structs_with_at_least_two_fields += 1;
        report.t1.structs_with_at_least_two_option_bool_fields +=
            u64::from(option_bool_fields >= 2);
    }
    report.t1.structs_detail.push(StructShapeRow {
        path: file.path.clone(),
        line: line(node),
        name: name.clone(),
        fields,
        option_bool_fields,
    });

    if let Some(body) = node.child_by_field_name("body")
        && body.kind() == "ordered_field_declaration_list"
        && let Some((wrapped, costume)) = single_tuple_field(file, body)
        && is_wide_primitive_spelling(&wrapped)
    {
        report.t6.newtype_supply += 1;
        report.t6.costume_newtypes += u64::from(costume);
        report.t6.newtypes.push(NewtypeRow {
            path: file.path.clone(),
            line: line(node),
            name: name.clone(),
            wrapped_type: wrapped,
            costume,
        });
    }

    if rust_is_pub(file, node)
        && let Some(body) = node.child_by_field_name("body")
    {
        let mut cursor = body.walk();
        for field in body.named_children(&mut cursor) {
            if field.kind() == "field_declaration"
                && rust_is_pub(file, field)
                && let Some(type_node) = field.child_by_field_name("type")
            {
                collect_primitive_region(file, type_node, &name, line(node), report);
            }
        }
    }
}

fn collect_enum(file: &SourceFile, node: Node<'_>, report: &mut TypeSpaceReport) {
    report.t1.all_type_definitions += 1;
    let mut variants = 0_u64;
    let mut payload_variants = 0_u64;
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for variant in body.named_children(&mut cursor) {
            if variant.kind() == "enum_variant" {
                variants += 1;
                let raw = text(file, variant);
                payload_variants += u64::from(raw.contains('(') || raw.contains('{'));
            }
        }
    }
    if variants >= 2 && payload_variants >= 1 {
        report.t1.data_bearing_enums += 1;
    } else if payload_variants == 0 {
        report.t1.fieldless_tag_enums += 1;
    } else {
        report.t1.other_type_definitions += 1;
    }
}

fn collect_impl(file: &SourceFile, node: Node<'_>, report: &mut TypeSpaceReport) {
    let Some(target_node) = node.child_by_field_name("type") else {
        return;
    };
    let target = compact(text(file, target_node));
    let impl_parameters = declared_type_parameters(file, node);
    let mut row = EndomorphicTypeRow {
        path: file.path.clone(),
        line: line(node),
        name: target.clone(),
        public_methods: 0,
        endomorphic_methods: 0,
        owned_endomorphic_methods: 0,
        borrowed_endomorphic_methods: 0,
        mutant_endomorphic_methods: 0,
    };
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for method in body.named_children(&mut cursor) {
            if method.kind() != "function_item" {
                continue;
            }
            collect_rust_signature(file, method, &impl_parameters, report);
            collect_binary_closure(file, method, Some(&target), report);
            if !rust_is_pub(file, method) {
                continue;
            }
            row.public_methods += 1;
            let return_type = method
                .child_by_field_name("return_type")
                .map(|n| compact(text(file, n)))
                .unwrap_or_default();
            match endo_kind(&return_type, &target) {
                Some(EndoKind::Owned) => {
                    row.endomorphic_methods += 1;
                    row.owned_endomorphic_methods += 1;
                }
                Some(EndoKind::Borrowed) => {
                    row.endomorphic_methods += 1;
                    row.borrowed_endomorphic_methods += 1;
                }
                Some(EndoKind::Mutant) => {
                    row.endomorphic_methods += 1;
                    row.mutant_endomorphic_methods += 1;
                }
                None => {}
            }
        }
    }
    if row.public_methods > 0 {
        report.t4.public_methods += row.public_methods;
        report.t4.endomorphic_methods += row.endomorphic_methods;
        report.t4.owned_endomorphic_methods += row.owned_endomorphic_methods;
        report.t4.borrowed_endomorphic_methods += row.borrowed_endomorphic_methods;
        report.t4.mutant_endomorphic_methods += row.mutant_endomorphic_methods;
        if let Some(existing) = report
            .t4
            .types
            .iter_mut()
            .find(|existing| existing.path == row.path && existing.name == row.name)
        {
            existing.line = existing.line.min(row.line);
            existing.public_methods += row.public_methods;
            existing.endomorphic_methods += row.endomorphic_methods;
            existing.owned_endomorphic_methods += row.owned_endomorphic_methods;
            existing.borrowed_endomorphic_methods += row.borrowed_endomorphic_methods;
            existing.mutant_endomorphic_methods += row.mutant_endomorphic_methods;
        } else {
            report.t4.types.push(row);
        }
    }
}

#[derive(Clone, Copy)]
enum EndoKind {
    Owned,
    Borrowed,
    Mutant,
}

fn endo_kind(return_type: &str, target: &str) -> Option<EndoKind> {
    let ret = return_type.trim_start_matches("->");
    let is_target = |value: &str| value == "Self" || value == target;
    if is_target(ret) {
        return Some(EndoKind::Owned);
    }
    let unlifetimed = ret
        .strip_prefix("&'")
        .and_then(|rest| rest.split_once(char::is_whitespace).map(|(_, value)| value))
        .unwrap_or(ret);
    if let Some(value) = unlifetimed.strip_prefix("&mut") {
        return is_target(value).then_some(EndoKind::Mutant);
    }
    if let Some(value) = ret.strip_prefix("&'") {
        if value.ends_with("mutSelf") || value.ends_with(&format!("mut{target}")) {
            return Some(EndoKind::Mutant);
        }
        if value.ends_with("Self") || value.ends_with(target) {
            return Some(EndoKind::Borrowed);
        }
    }
    if let Some(value) = unlifetimed.strip_prefix('&') {
        return is_target(value).then_some(EndoKind::Borrowed);
    }
    for wrapper in ["Box<", "Option<"] {
        if let Some(inner) = ret.strip_prefix(wrapper).and_then(|v| v.strip_suffix('>'))
            && is_target(inner)
        {
            return Some(EndoKind::Owned);
        }
    }
    if let Some(inner) = ret
        .strip_prefix("Result<")
        .and_then(|v| v.strip_suffix('>'))
        && let Some(first) = split_top_level(inner).first()
        && is_target(first)
    {
        return Some(EndoKind::Owned);
    }
    None
}

fn collect_binary_closure(
    file: &SourceFile,
    node: Node<'_>,
    target: Option<&str>,
    report: &mut TypeSpaceReport,
) {
    report.t4.functions_censused_for_binary_closure += 1;
    let Some(params) = node.child_by_field_name("parameters") else {
        return;
    };
    let Some(ret) = node.child_by_field_name("return_type") else {
        return;
    };
    let mut types = Vec::new();
    let mut cursor = params.walk();
    for param in params.named_children(&mut cursor) {
        if param.kind() == "self_parameter" {
            continue;
        }
        if let Some(ty) = param.child_by_field_name("type") {
            types.push(compact(text(file, ty)));
        }
    }
    if types.len() != 2 {
        return;
    }
    let ret = compact(text(file, ret)).trim_start_matches("->").to_owned();
    if types.iter().any(|value| value.starts_with("&mut")) {
        return;
    }
    let normalized = |value: &str| {
        let value = value.trim_start_matches('&');
        if value == "Self" {
            target.unwrap_or("Self").to_owned()
        } else {
            value.to_owned()
        }
    };
    if normalized(&types[0]) == normalized(&types[1]) && normalized(&types[0]) == normalized(&ret) {
        report.t4.binary_closures += 1;
        report.t4.binary_items.push(LocationRow {
            path: file.path.clone(),
            line: line(node),
            name: field_text(file, node, "name").unwrap_or_else(|| "<anonymous>".to_owned()),
        });
    }
}

fn collect_rust_signature(
    file: &SourceFile,
    node: Node<'_>,
    inherited_parameters: &BTreeMap<String, GenericParameter>,
    report: &mut TypeSpaceReport,
) {
    let name = field_text(file, node, "name").unwrap_or_else(|| "<anonymous>".to_owned());
    let is_public = rust_is_pub(file, node);
    let mut parameters = inherited_parameters.clone();
    merge_generic_parameters(&mut parameters, declared_type_parameters(file, node));
    let mut signature_regions = Vec::new();
    let mut anonymous_parameters = Vec::new();
    if let Some(params) = node.child_by_field_name("parameters") {
        let mut cursor = params.walk();
        for param in params.named_children(&mut cursor) {
            if let Some(ty) = param.child_by_field_name("type") {
                collect_dynamic_region(file, ty, &name, report);
                if is_public {
                    collect_primitive_region(file, ty, &name, line(node), report);
                    signature_regions.push(ty);
                    collect_argument_impl_traits(file, ty, &mut anonymous_parameters);
                }
            }
        }
    }
    for (index, parameter) in anonymous_parameters.into_iter().enumerate() {
        parameters.insert(format!("<impl-trait-{index}>"), parameter);
    }
    let mut return_type = None;
    if let Some(ret) = node.child_by_field_name("return_type") {
        collect_dynamic_region(file, ret, &name, report);
        if is_public {
            signature_regions.push(ret);
            return_type = Some(ret);
        }
    }
    if is_public {
        collect_signature_parametricity(file, &signature_regions, return_type, &parameters, report);
    }
}

fn declared_type_parameters(
    file: &SourceFile,
    node: Node<'_>,
) -> BTreeMap<String, GenericParameter> {
    let mut parameters = BTreeMap::new();
    if let Some(type_parameters) = node.child_by_field_name("type_parameters") {
        let mut cursor = type_parameters.walk();
        for parameter in type_parameters.named_children(&mut cursor) {
            if parameter.kind() != "type_parameter" {
                continue;
            }
            let Some(name) = parameter.child_by_field_name("name") else {
                continue;
            };
            let bounds = parameter
                .child_by_field_name("bounds")
                .map_or_else(Vec::new, |bounds| trait_bounds(file, bounds));
            parameters.insert(text(file, name).to_owned(), GenericParameter { bounds });
        }
    }
    let mut cursor = node.walk();
    for clause in node.named_children(&mut cursor) {
        if clause.kind() != "where_clause" {
            continue;
        }
        let mut clause_cursor = clause.walk();
        for predicate in clause.named_children(&mut clause_cursor) {
            let Some(left) = predicate.child_by_field_name("left") else {
                continue;
            };
            let key = compact(text(file, left));
            let Some(parameter) = parameters.get_mut(&key) else {
                continue;
            };
            if let Some(bounds) = predicate.child_by_field_name("bounds") {
                parameter.bounds.extend(trait_bounds(file, bounds));
            }
        }
    }
    parameters
}

fn merge_generic_parameters(
    target: &mut BTreeMap<String, GenericParameter>,
    source: BTreeMap<String, GenericParameter>,
) {
    for (name, parameter) in source {
        target
            .entry(name)
            .or_default()
            .bounds
            .extend(parameter.bounds);
    }
}

fn collect_argument_impl_traits(
    file: &SourceFile,
    node: Node<'_>,
    out: &mut Vec<GenericParameter>,
) {
    if node.kind() == "abstract_type" {
        let bounds = node
            .child_by_field_name("trait")
            .map_or_else(Vec::new, |bounds| trait_bounds(file, bounds));
        out.push(GenericParameter { bounds });
        return;
    }
    visit_named(node, |child| collect_argument_impl_traits(file, child, out));
}

fn trait_bounds(file: &SourceFile, node: Node<'_>) -> Vec<String> {
    if !matches!(node.kind(), "trait_bounds" | "bounded_type") {
        return vec![compact(text(file, node))];
    }
    let mut bounds = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if !matches!(child.kind(), "lifetime" | "use_bounds") {
            bounds.push(compact(text(file, child)));
        }
    }
    bounds
}

fn collect_signature_parametricity(
    file: &SourceFile,
    signature_regions: &[Node<'_>],
    return_type: Option<Node<'_>>,
    parameters: &BTreeMap<String, GenericParameter>,
    report: &mut TypeSpaceReport,
) {
    let parameter_names = parameters.keys().cloned().collect::<BTreeSet<_>>();
    let mut abstract_mentions = 0_u64;
    let mut total_mentions = 0_u64;
    for region in signature_regions {
        let identifiers = type_identifiers(text(file, *region));
        abstract_mentions += identifiers
            .iter()
            .filter(|identifier| parameter_names.contains(*identifier))
            .count() as u64;
        total_mentions += identifiers.len() as u64;
    }
    let anonymous_parameters = parameters
        .keys()
        .filter(|name| name.starts_with("<impl-trait-"))
        .count() as u64;
    abstract_mentions += anonymous_parameters;
    total_mentions += anonymous_parameters;

    let t3 = &mut report.t3;
    t3.public_functions += 1;
    t3.abstract_type_leaf_mentions += abstract_mentions;
    t3.concrete_type_leaf_mentions += total_mentions.saturating_sub(abstract_mentions);
    t3.signature_type_leaf_mentions += total_mentions;
    if parameters.is_empty() {
        return;
    }

    t3.generic_public_functions += 1;
    let return_parametric = return_type.is_some_and(|return_type| {
        has_descendant_kind(return_type, "abstract_type")
            || type_identifiers(text(file, return_type))
                .iter()
                .any(|identifier| parameter_names.contains(identifier))
    });
    t3.return_parametric_public_functions += u64::from(return_parametric);
    t3.generic_parameters += parameters.len() as u64;
    for parameter in parameters.values() {
        t3.bound_counts.push(parameter.bounds.len() as u64);
        t3.generic_washing_parameters += u64::from(
            parameter
                .bounds
                .iter()
                .any(|bound| is_generic_washing_bound(bound, &parameter_names)),
        );
    }
}

fn is_generic_washing_bound(bound: &str, generic_names: &BTreeSet<String>) -> bool {
    let bound = bound.trim_start_matches('?');
    let Some(open) = bound.find('<') else {
        return false;
    };
    let Some(arguments) = bound
        .get(open + 1..)
        .and_then(|rest| rest.strip_suffix('>'))
    else {
        return false;
    };
    let path = &bound[..open];
    let constructor = path.rsplit("::").next().unwrap_or(path);
    if !matches!(constructor, "Into" | "AsRef" | "From" | "TryInto")
        || split_top_level(arguments).len() != 1
    {
        return false;
    }
    !lexical_identifiers(arguments)
        .iter()
        .any(|(identifier, _)| generic_names.contains(identifier))
}

fn integer_distribution(values: &mut [u64]) -> IntegerDistribution {
    if values.is_empty() {
        return IntegerDistribution::default();
    }
    values.sort_unstable();
    let p50_index = values.len().div_ceil(2).saturating_sub(1);
    let p90_index = values
        .len()
        .saturating_mul(9)
        .div_ceil(10)
        .saturating_sub(1);
    IntegerDistribution {
        min: values[0],
        p50: values[p50_index],
        p90: values[p90_index],
        max: values[values.len() - 1],
    }
}

fn collect_field_types(file: &SourceFile, node: Node<'_>, out: &mut Vec<String>) {
    if node.kind() == "ordered_field_declaration_list" {
        let raw = text(file, node).trim();
        if let Some(inner) = raw
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
        {
            for field in split_top_level(inner) {
                let field = field.trim();
                if field.is_empty() {
                    continue;
                }
                let field = strip_tuple_visibility(field);
                out.push(field.to_owned());
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for field in node.named_children(&mut cursor) {
        if matches!(
            field.kind(),
            "field_declaration" | "ordered_field_declaration"
        ) && let Some(ty) = field
            .child_by_field_name("type")
            .or_else(|| field.named_child(0))
        {
            out.push(text(file, ty).to_owned());
        }
    }
}

fn collect_dynamic_region(
    file: &SourceFile,
    node: Node<'_>,
    name: &str,
    report: &mut TypeSpaceReport,
) {
    let raw = text(file, node);
    collect_dynamic_text(file, raw, line(node), name, report);
}

fn collect_dynamic_text(
    file: &SourceFile,
    raw: &str,
    item_line: usize,
    name: &str,
    report: &mut TypeSpaceReport,
) {
    let leaves = type_leaf_count(raw);
    let dynamic = dynamic_mentions(file.language, raw);
    report.t2.type_constructor_leaf_mentions += leaves;
    report.t2.dynamic_state_mentions += dynamic;
    let bucket = report
        .t2
        .by_language
        .entry(file.language.name().to_owned())
        .or_default();
    bucket.numerator += dynamic;
    bucket.denominator += leaves;
    if dynamic > 0 {
        if let Some(row) = report.t2.items.iter_mut().find(|row| {
            row.path == file.path
                && row.line == item_line
                && row.name == name
                && row.language == file.language.name()
        }) {
            row.dynamic_mentions += dynamic;
            row.type_leaf_mentions += leaves;
        } else {
            report.t2.items.push(DynamicItemRow {
                path: file.path.clone(),
                line: item_line,
                name: name.to_owned(),
                language: file.language.name().to_owned(),
                dynamic_mentions: dynamic,
                type_leaf_mentions: leaves,
            });
        }
    }
}

fn collect_primitive_region(
    file: &SourceFile,
    node: Node<'_>,
    name: &str,
    item_line: usize,
    report: &mut TypeSpaceReport,
) {
    let raw = text(file, node);
    let identifiers = type_identifiers(raw);
    let primitive_mentions = identifiers
        .iter()
        .filter(|value| is_wide_primitive(value))
        .count() as u64;
    let type_mentions = identifiers.len() as u64;
    report.t6.wide_primitive_mentions += primitive_mentions;
    report.t6.non_primitive_mentions += type_mentions.saturating_sub(primitive_mentions);
    report.t6.public_boundary_type_mentions += type_mentions;
    if primitive_mentions > 0 {
        if let Some(row) = report
            .t6
            .primitive_items
            .iter_mut()
            .find(|row| row.path == file.path && row.line == item_line && row.name == name)
        {
            row.primitive_mentions += primitive_mentions;
            row.type_mentions += type_mentions;
        } else {
            report.t6.primitive_items.push(PrimitiveItemRow {
                path: file.path.clone(),
                line: item_line,
                name: name.to_owned(),
                primitive_mentions,
                type_mentions,
            });
        }
    }
}

fn collect_ownership_type_region(file: &SourceFile, node: Node<'_>, state: &mut FileState) {
    let raw = text(file, node);
    let identifiers = type_identifiers(raw);
    state.ownership.type_constructor_leaf_mentions += identifiers.len() as u64;
    for name in identifiers {
        if matches!(name.as_str(), "RefCell" | "Cell" | "Mutex" | "RwLock") {
            state.ownership.shared_mutable_type_mentions += 1;
        }
        if matches!(name.as_str(), "Rc" | "Arc") {
            state.ownership.shared_ownership_type_mentions += 1;
        }
    }
}

fn collect_call(file: &SourceFile, node: Node<'_>, state: &mut FileState) {
    state.ownership.call_expressions += 1;
    let callee = node
        .child_by_field_name("function")
        .map(|n| compact(text(file, n)))
        .unwrap_or_default();
    state.ownership.clone_calls += u64::from(callee.ends_with(".clone"));
    state.ownership.borrow_lock_calls += u64::from(
        [".borrow", ".borrow_mut", ".lock"]
            .iter()
            .any(|suffix| callee.ends_with(suffix)),
    );
}

fn walk_typescript(file: &SourceFile, node: Node<'_>, report: &mut TypeSpaceReport) {
    if node.kind() == "index_signature" {
        let raw = text(file, node);
        let dynamic = dynamic_mentions(file.language, raw);
        // The first identifier is the index binding, not a type leaf.
        let leaves = (type_identifiers(raw).len() as u64).saturating_sub(1);
        report.t2.dynamic_state_mentions += dynamic;
        report.t2.type_constructor_leaf_mentions += leaves;
        let bucket = report
            .t2
            .by_language
            .entry(file.language.name().to_owned())
            .or_default();
        bucket.numerator += dynamic;
        bucket.denominator += leaves;
        report.t2.items.push(DynamicItemRow {
            path: file.path.clone(),
            line: line(node),
            name: item_name(file, node),
            language: file.language.name().to_owned(),
            dynamic_mentions: dynamic,
            type_leaf_mentions: leaves,
        });
        return;
    } else if is_ts_field(node) || is_ts_parameter(node) {
        if let Some(ty) = node
            .child_by_field_name("type")
            .or_else(|| named_child_of_kind(node, "type_annotation"))
        {
            collect_dynamic_region(file, ty, &item_name(file, node), report);
        }
    } else if is_ts_function(node)
        && let Some(ret) = node.child_by_field_name("return_type")
    {
        collect_dynamic_region(file, ret, &item_name(file, node), report);
    }
    visit_named(node, |child| walk_typescript(file, child, report));
}

fn walk_python(file: &SourceFile, node: Node<'_>, report: &mut TypeSpaceReport) {
    if node.kind() == "function_definition" {
        let name = item_name(file, node);
        if let Some(params) = node.child_by_field_name("parameters") {
            let mut cursor = params.walk();
            for param in params.named_children(&mut cursor) {
                if let Some(ty) = param.child_by_field_name("type") {
                    collect_dynamic_region(file, ty, &name, report);
                }
            }
        }
        if let Some(ret) = node.child_by_field_name("return_type") {
            collect_dynamic_region(file, ret, &name, report);
        }
        if let Some(body) = node.child_by_field_name("body") {
            walk_python(file, body, report);
        }
        return;
    }
    if node.kind() == "assignment"
        && node.child_by_field_name("type").is_some()
        && let Some(ty) = node.child_by_field_name("type")
    {
        collect_dynamic_region(file, ty, &item_name(file, node), report);
    }
    visit_named(node, |child| walk_python(file, child, report));
}

fn dynamic_mentions(language: SourceLanguage, raw: &str) -> u64 {
    let compacted = compact(raw);
    match language {
        SourceLanguage::Rust => {
            let serde_value = compacted.matches("serde_json::Value").count() as u64;
            let dyn_any = compacted.matches("dynAny").count() as u64;
            let maps = ["HashMap<String,", "BTreeMap<String,"]
                .iter()
                .map(|p| compacted.matches(p).count() as u64)
                .sum::<u64>();
            serde_value + dyn_any + maps
        }
        SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            let ids = type_identifiers(raw);
            ids.iter()
                .filter(|v| matches!(v.as_str(), "any" | "unknown"))
                .count() as u64
                + compacted.matches("Record<string,").count() as u64
                + u64::from(
                    compacted.starts_with('[')
                        && compacted.contains(":string]")
                        && compacted.contains("]:"),
                )
        }
        SourceLanguage::Python => {
            let ids = type_identifiers(raw);
            ids.iter().filter(|v| v.as_str() == "Any").count() as u64
                + compacted.matches("Dict[str,Any]").count() as u64
                + ids.iter().filter(|v| v.as_str() == "dict").count() as u64
        }
        SourceLanguage::JavaScript | SourceLanguage::Go => 0,
    }
}

fn type_leaf_count(raw: &str) -> u64 {
    type_identifiers(raw).len() as u64
}

fn type_identifiers(raw: &str) -> Vec<String> {
    lexical_identifiers(raw)
        .into_iter()
        .filter(|(current, trailing)| {
            !matches!(
                current.as_str(),
                "dyn" | "mut" | "const" | "pub" | "where" | "impl" | "fn"
            ) && !trailing.starts_with("::")
                && !trailing.starts_with('.')
        })
        .map(|(current, _)| current)
        .collect()
}

fn lexical_identifiers(raw: &str) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    let chars = raw.char_indices().collect::<Vec<_>>();
    let mut cursor = 0;
    while cursor < chars.len() {
        let (start, ch) = chars[cursor];
        if ch == '\'' {
            cursor += 1;
            while cursor < chars.len()
                && (chars[cursor].1 == '_' || chars[cursor].1.is_alphanumeric())
            {
                cursor += 1;
            }
            continue;
        }
        if ch != '_' && !ch.is_alphanumeric() {
            cursor += 1;
            continue;
        }
        cursor += 1;
        while cursor < chars.len() && (chars[cursor].1 == '_' || chars[cursor].1.is_alphanumeric())
        {
            cursor += 1;
        }
        let end = chars.get(cursor).map_or(raw.len(), |(index, _)| *index);
        let current = &raw[start..end];
        let trailing = &raw[end..];
        if current != "_" && !current.chars().all(|c| c.is_ascii_digit()) {
            out.push((current.to_owned(), trailing));
        }
    }
    out
}

fn is_wide_primitive(value: &str) -> bool {
    matches!(
        value,
        "String"
            | "str"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "usize"
            | "isize"
            | "f32"
            | "f64"
            | "bool"
    )
}

fn is_wide_primitive_spelling(value: &str) -> bool {
    if is_wide_primitive(value) || value == "&str" {
        return true;
    }
    value.starts_with("&'") && value.ends_with("str")
}

fn contains_option_or_bool(raw: &str) -> bool {
    type_identifiers(raw)
        .iter()
        .any(|value| matches!(value.as_str(), "Option" | "bool"))
}

fn single_tuple_field(file: &SourceFile, body: Node<'_>) -> Option<(String, bool)> {
    let raw = text(file, body).trim();
    let inner = raw.strip_prefix('(')?.strip_suffix(')')?.trim();
    if split_top_level(inner).len() != 1 || inner.is_empty() {
        return None;
    }
    let costume = inner.starts_with("pub ") || inner.starts_with("pub(");
    let type_text = strip_tuple_visibility(inner);
    Some((compact(type_text), costume))
}

fn strip_tuple_visibility(field: &str) -> &str {
    if let Some(rest) = field.strip_prefix("pub ") {
        return rest.trim();
    }
    if field.starts_with("pub(")
        && let Some(end) = field.find(')')
    {
        return field.get(end + 1..).unwrap_or("").trim();
    }
    field.trim()
}

fn is_rust_type_region(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "primitive_type"
            | "type_identifier"
            | "scoped_type_identifier"
            | "generic_type"
            | "reference_type"
            | "tuple_type"
            | "array_type"
            | "function_type"
            | "dynamic_type"
            | "abstract_type"
            | "bounded_type"
    ) && node
        .parent()
        .is_none_or(|parent| !is_rust_type_region(parent))
}

fn is_ts_field(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "public_field_definition"
            | "property_signature"
            | "required_parameter"
            | "optional_parameter"
    ) && node
        .parent()
        .is_some_and(|p| !matches!(p.kind(), "formal_parameters"))
}
fn is_ts_parameter(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "required_parameter" | "optional_parameter" | "rest_pattern"
    )
}
fn is_ts_function(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "function_declaration"
            | "function_expression"
            | "arrow_function"
            | "method_definition"
            | "method_signature"
            | "call_signature"
            | "function_signature"
    )
}

fn rust_is_pub(file: &SourceFile, node: Node<'_>) -> bool {
    node.child_by_field_name("visibility")
        .or_else(|| named_child_of_kind(node, "visibility_modifier"))
        .is_some_and(|visibility| text(file, visibility).trim() == "pub")
}

fn has_ancestor(mut node: Node<'_>, kind: &str) -> bool {
    while let Some(parent) = node.parent() {
        if parent.kind() == kind {
            return true;
        }
        node = parent;
    }
    false
}

fn has_descendant_kind(node: Node<'_>, kind: &str) -> bool {
    if node.kind() == kind {
        return true;
    }
    let mut found = false;
    visit_named(node, |child| found |= has_descendant_kind(child, kind));
    found
}

fn item_name(file: &SourceFile, node: Node<'_>) -> String {
    field_text(file, node, "name")
        .or_else(|| {
            node.child_by_field_name("left")
                .map(|n| text(file, n).to_owned())
        })
        .unwrap_or_else(|| format!("<{}>", node.kind()))
}

fn field_text(file: &SourceFile, node: Node<'_>, field: &str) -> Option<String> {
    node.child_by_field_name(field)
        .map(|child| text(file, child).to_owned())
}
fn text<'a>(file: &'a SourceFile, node: Node<'_>) -> &'a str {
    node.utf8_text(&file.bytes).unwrap_or("")
}
fn line(node: Node<'_>) -> usize {
    node.start_position().row + 1
}
fn compact(raw: &str) -> String {
    raw.chars().filter(|ch| !ch.is_whitespace()).collect()
}
fn split_top_level(raw: &str) -> Vec<&str> {
    let mut depth = 0_u32;
    let mut start = 0;
    let mut out = Vec::new();
    for (index, ch) in raw.char_indices() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&raw[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    out.push(&raw[start..]);
    out
}

fn named_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}
fn visit_named(node: Node<'_>, mut visitor: impl FnMut(Node<'_>)) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visitor(child);
    }
}

fn location_order(a: &LocationRow, b: &LocationRow) -> std::cmp::Ordering {
    (&a.path, a.line, &a.name).cmp(&(&b.path, b.line, &b.name))
}
fn location_order_struct(a: &StructShapeRow, b: &StructShapeRow) -> std::cmp::Ordering {
    (&a.path, a.line, &a.name).cmp(&(&b.path, b.line, &b.name))
}
fn location_order_dynamic(a: &DynamicItemRow, b: &DynamicItemRow) -> std::cmp::Ordering {
    (&a.path, a.line, &a.name).cmp(&(&b.path, b.line, &b.name))
}
fn location_order_endo(a: &EndomorphicTypeRow, b: &EndomorphicTypeRow) -> std::cmp::Ordering {
    (&a.path, a.line, &a.name).cmp(&(&b.path, b.line, &b.name))
}
fn location_order_primitive(a: &PrimitiveItemRow, b: &PrimitiveItemRow) -> std::cmp::Ordering {
    (&a.path, a.line, &a.name).cmp(&(&b.path, b.line, &b.name))
}
fn location_order_newtype(a: &NewtypeRow, b: &NewtypeRow) -> std::cmp::Ordering {
    (&a.path, a.line, &a.name).cmp(&(&b.path, b.line, &b.name))
}
