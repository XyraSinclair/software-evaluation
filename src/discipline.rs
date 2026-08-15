//! Deterministic per-function structural discipline inventory.
//!
//! Each counter is a syntactic proxy for one of the generative determinants in
//! DETERMINANTS.md (G1 effects, G2 types, G3 mutation, G5 shape, G6 errors).
//! None of them measures semantics: syntactic purity is not semantic purity,
//! the effect namespaces are a documented list rather than the truth, and no
//! call, type, or name is resolved across files. The instrument attributes
//! every construct to its innermost enclosing function space in a single walk.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;
use thiserror::Error;
use tree_sitter::Node;

use crate::metrics::nearest_rank;
use crate::source::{SourceError, SourceFile, SourceLanguage, load_source_tree, parse_source};
use crate::tests_analysis::{FileRole, classify_file};

// Effect namespaces (G1). A call is an `effect_call` when its callee text,
// stripped of whitespace, starts with one of these documented prefixes. This
// is a spelling list, not a resolved effect: it over-matches identifiers that
// merely share a prefix and misses effects reached through aliases or unlisted
// namespaces.
const RUST_EFFECT_PREFIXES: &[&str] = &[
    "std::fs", "std::io", "std::net", "std::process", "std::env", "std::thread", "std::time",
    "tokio::", "println!", "eprintln!", "print!", "eprint!", "dbg!", "log::", "tracing::",
];
const PYTHON_EFFECT_PREFIXES: &[&str] = &[
    "open", "print", "input", "os.", "sys.", "subprocess.", "socket.", "random.", "time.",
    "datetime.now", "logging.", "requests.", "shutil.", "pathlib.Path.write", "pathlib.Path.read",
];
const JS_EFFECT_PREFIXES: &[&str] = &[
    "console.", "fetch", "fs.", "process.", "Date.now", "new Date", "Math.random", "localStorage",
    "sessionStorage", "document.", "window.", "setTimeout", "setInterval", "require(",
];
const GO_EFFECT_PREFIXES: &[&str] = &[
    "fmt.Print", "fmt.Fprint", "os.", "io.", "net.", "log.", "time.Now", "time.Sleep", "rand.",
    "http.",
];

// Panic-like Rust macros (G6): `panic!`, `unreachable!`, `todo!`, `unimplemented!`.
const RUST_PANIC_MACROS: &[&str] = &["panic", "unreachable", "todo", "unimplemented"];

#[derive(Debug, Error)]
pub enum DisciplineError {
    #[error(transparent)]
    Source(#[from] SourceError),
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionDiscipline {
    pub path: String,
    pub language: SourceLanguage,
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub syntax_errors: bool,
    // G1 effects.
    /// Assignments/compound-assignments whose target root identifier is not a
    /// binding owned by this space (declared name, parameter), plus writes
    /// through a parameter (`param.field = x`, `*param = x`, `&mut` param) and
    /// Python `global`/`nonlocal`-declared names.
    pub nonlocal_writes: u32,
    /// Rust `&mut T`/`mut self`/`&mut self` parameters; Go pointer receivers
    /// and pointer parameters. Other languages do not encode parameter
    /// mutability syntactically and always report 0.
    pub mut_params: u32,
    /// Rust `unsafe_block` nodes; always 0 for other languages.
    pub unsafe_blocks: u32,
    /// Calls (`call_expression`/`macro_invocation`/`call`/`new_expression`)
    /// whose callee text starts with a documented effect prefix.
    pub effect_calls: u32,
    /// True when `nonlocal_writes`, `mut_params`, `unsafe_blocks`, and
    /// `effect_calls` are all zero. Syntactic only; not semantic purity.
    pub syntactically_pure: bool,
    // G3 mutation.
    /// Local bindings introduced: parameters plus Rust `let` patterns, Python
    /// first-seen assignment targets, JS `const`/`let`/`var` declarators, Go
    /// `:=`/`var` names.
    pub bindings: u32,
    /// Rust `let mut`/`mut` patterns; JS `let`/`var`; Go `:=`/`var` names later
    /// reassigned; Python names assigned more than once. Parameters excluded.
    pub mutable_bindings: u32,
    /// Assignment/augmented-assignment nodes writing a local binding, excluding
    /// the introducing declaration.
    pub reassignments: u32,
    /// Rust `let` re-declaring a name already bound in the space; JS/TS/Go a
    /// local declaration whose name is already bound earlier in the space.
    pub shadowings: u32,
    /// Over mutable bindings, max(last write line − declaration line); 0 when
    /// there are no mutable bindings.
    pub max_mutable_live_range_lines: u32,
    // G5 shape.
    /// Declared parameters (excluding a Rust `self` receiver and a Go receiver).
    pub params: u32,
    /// Rust `bool`, TS `boolean`, Go `bool` parameters; Python parameters with
    /// a `bool` annotation or a `True`/`False` default.
    pub bool_params: u32,
    /// Statement nodes in the space: Rust `expression_statement`/
    /// `let_declaration`; other languages any kind ending in `_statement` plus
    /// JS/Go local declaration kinds.
    pub statements: u32,
    /// Rust block with a tail expression and no statements (or a non-block
    /// closure body); JS/TS arrow with an expression body; Python `lambda`.
    pub single_expression_body: bool,
    /// Rust no `->` or `-> ()`; TS `: void`; Go no result list; Python/JS no
    /// `return <expr>` anywhere in the space.
    pub unit_return: bool,
    /// Longest run of chained method calls `a.b().c().d()`.
    pub max_call_chain_len: u32,
    // G6 errors.
    /// Rust `?` (`try_expression`); Go `if err != nil { return … }` sites.
    pub try_propagations: u32,
    /// Rust `.unwrap()`/`.expect(`.
    pub unwrap_expect: u32,
    /// Rust `panic!`/`unreachable!`/`todo!`/`unimplemented!`; Go `panic(`.
    pub panic_like: u32,
    /// Python bare `except`/`except Exception`/`except BaseException`; JS/TS
    /// `catch` whose body is empty or only console/log calls; Go `_ = err`,
    /// `_, _ =`.
    pub broad_catches: u32,
    /// JS/TS `catch` and Python `except` with an empty body or only `pass`.
    pub empty_catches: u32,
    /// Rust `let _ = <call>`; Go `_ = <call>`.
    pub ignored_results: u32,
    // G2 types.
    /// `if`/`match`/`switch` conditions comparing an identifier to a string
    /// literal with `==`/`!=`/`===`/`!==` (or a string-literal `case`/arm).
    pub string_literal_conditions: u32,
    /// TS `any` type occurrences (annotations and `as any`).
    pub any_annotations: u32,
    /// Python parameters without an annotation (excluding `self`/`cls`); TS
    /// parameters without a type annotation.
    pub unannotated_params: u32,
    /// Python `# type: ignore` comments; TS `// @ts-ignore`/`@ts-expect-error`.
    pub type_ignores: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DisciplineTotals {
    pub nonlocal_writes: u64,
    pub mut_params: u64,
    pub unsafe_blocks: u64,
    pub effect_calls: u64,
    pub bindings: u64,
    pub mutable_bindings: u64,
    pub reassignments: u64,
    pub shadowings: u64,
    pub max_mutable_live_range_lines: u64,
    pub params: u64,
    pub bool_params: u64,
    pub statements: u64,
    pub single_expression_bodies: u64,
    pub unit_returns: u64,
    pub max_call_chain_len: u64,
    pub try_propagations: u64,
    pub unwrap_expect: u64,
    pub panic_like: u64,
    pub broad_catches: u64,
    pub empty_catches: u64,
    pub ignored_results: u64,
    pub string_literal_conditions: u64,
    pub any_annotations: u64,
    pub unannotated_params: u64,
    pub type_ignores: u64,
    /// Numeric literals outside const/type declarations and test files, excluding 0, 1, -1, 2.
    pub magic_numbers: u64,
    /// String literals of interior length >= 2 in the same scope, excluding Python docstrings.
    pub magic_strings: u64,
    pub global_mutable_state: u64,
}

impl DisciplineTotals {
    fn add_function(&mut self, f: &FunctionDiscipline) {
        self.nonlocal_writes += u64::from(f.nonlocal_writes);
        self.mut_params += u64::from(f.mut_params);
        self.unsafe_blocks += u64::from(f.unsafe_blocks);
        self.effect_calls += u64::from(f.effect_calls);
        self.bindings += u64::from(f.bindings);
        self.mutable_bindings += u64::from(f.mutable_bindings);
        self.reassignments += u64::from(f.reassignments);
        self.shadowings += u64::from(f.shadowings);
        self.max_mutable_live_range_lines += u64::from(f.max_mutable_live_range_lines);
        self.params += u64::from(f.params);
        self.bool_params += u64::from(f.bool_params);
        self.statements += u64::from(f.statements);
        self.single_expression_bodies += u64::from(f.single_expression_body);
        self.unit_returns += u64::from(f.unit_return);
        self.max_call_chain_len += u64::from(f.max_call_chain_len);
        self.try_propagations += u64::from(f.try_propagations);
        self.unwrap_expect += u64::from(f.unwrap_expect);
        self.panic_like += u64::from(f.panic_like);
        self.broad_catches += u64::from(f.broad_catches);
        self.empty_catches += u64::from(f.empty_catches);
        self.ignored_results += u64::from(f.ignored_results);
        self.string_literal_conditions += u64::from(f.string_literal_conditions);
        self.any_annotations += u64::from(f.any_annotations);
        self.unannotated_params += u64::from(f.unannotated_params);
        self.type_ignores += u64::from(f.type_ignores);
    }

    fn add(&mut self, other: &DisciplineTotals) {
        self.nonlocal_writes += other.nonlocal_writes;
        self.mut_params += other.mut_params;
        self.unsafe_blocks += other.unsafe_blocks;
        self.effect_calls += other.effect_calls;
        self.bindings += other.bindings;
        self.mutable_bindings += other.mutable_bindings;
        self.reassignments += other.reassignments;
        self.shadowings += other.shadowings;
        self.max_mutable_live_range_lines += other.max_mutable_live_range_lines;
        self.params += other.params;
        self.bool_params += other.bool_params;
        self.statements += other.statements;
        self.single_expression_bodies += other.single_expression_bodies;
        self.unit_returns += other.unit_returns;
        self.max_call_chain_len += other.max_call_chain_len;
        self.try_propagations += other.try_propagations;
        self.unwrap_expect += other.unwrap_expect;
        self.panic_like += other.panic_like;
        self.broad_catches += other.broad_catches;
        self.empty_catches += other.empty_catches;
        self.ignored_results += other.ignored_results;
        self.string_literal_conditions += other.string_literal_conditions;
        self.any_annotations += other.any_annotations;
        self.unannotated_params += other.unannotated_params;
        self.type_ignores += other.type_ignores;
        self.magic_numbers += other.magic_numbers;
        self.magic_strings += other.magic_strings;
        self.global_mutable_state += other.global_mutable_state;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FileDiscipline {
    pub path: String,
    pub language: SourceLanguage,
    pub syntax_errors: bool,
    pub functions: u64,
    pub sums: DisciplineTotals,
}

#[derive(Debug, Clone, Serialize)]
pub struct LanguageFunctionCount {
    pub language: SourceLanguage,
    pub functions: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Tail {
    pub p50: u64,
    pub p90: u64,
    pub p99: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DisciplineTails {
    pub mutable_bindings: Tail,
    pub max_mutable_live_range_lines: Tail,
    pub max_call_chain_len: Tail,
    pub params: Tail,
}

#[derive(Debug, Clone, Serialize)]
pub struct DisciplineCoverage {
    pub enumerated_files: usize,
    pub supported_files: usize,
    pub skipped_unsupported_files: usize,
    pub syntax_error_files: u64,
    pub functions_total: u64,
    pub functions_per_language: Vec<LanguageFunctionCount>,
    pub totals: DisciplineTotals,
    pub pure_functions: u64,
    pub pure_fraction: Option<f64>,
    pub tails: DisciplineTails,
}

#[derive(Debug, Clone, Serialize)]
pub struct DisciplineReport {
    pub root: String,
    pub analyzer: String,
    pub coverage: DisciplineCoverage,
    pub files: Vec<FileDiscipline>,
    pub functions: Vec<FunctionDiscipline>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisciplineSort {
    Pure,
    Mutable,
    LiveRange,
    Chain,
    Errors,
    Params,
}

impl FunctionDiscipline {
    fn error_load(&self) -> u32 {
        self.unwrap_expect
            + self.panic_like
            + self.broad_catches
            + self.empty_catches
            + self.ignored_results
    }

    fn impurity(&self) -> u32 {
        self.nonlocal_writes + self.effect_calls + self.mut_params + self.unsafe_blocks
    }

    fn sort_key(&self, sort: DisciplineSort) -> u64 {
        u64::from(match sort {
            DisciplineSort::Pure => {
                if self.syntactically_pure {
                    0
                } else {
                    1 + self.impurity()
                }
            }
            DisciplineSort::Mutable => self.mutable_bindings,
            DisciplineSort::LiveRange => self.max_mutable_live_range_lines,
            DisciplineSort::Chain => self.max_call_chain_len,
            DisciplineSort::Errors => self.error_load(),
            DisciplineSort::Params => self.params,
        })
    }
}

impl FileDiscipline {
    fn sort_key(&self, sort: DisciplineSort) -> u64 {
        let sums = &self.sums;
        match sort {
            DisciplineSort::Pure => {
                sums.nonlocal_writes + sums.effect_calls + sums.mut_params + sums.unsafe_blocks
            }
            DisciplineSort::Mutable => sums.mutable_bindings,
            DisciplineSort::LiveRange => sums.max_mutable_live_range_lines,
            DisciplineSort::Chain => sums.max_call_chain_len,
            DisciplineSort::Errors => {
                sums.unwrap_expect
                    + sums.panic_like
                    + sums.broad_catches
                    + sums.empty_catches
                    + sums.ignored_results
            }
            DisciplineSort::Params => sums.params,
        }
    }
}

pub fn rank_functions(
    report: &DisciplineReport,
    sort: DisciplineSort,
    top: usize,
) -> Vec<&FunctionDiscipline> {
    let mut rows: Vec<&FunctionDiscipline> = report.functions.iter().collect();
    rows.sort_by(|a, b| {
        b.sort_key(sort).cmp(&a.sort_key(sort)).then_with(|| {
            a.path
                .cmp(&b.path)
                .then_with(|| a.start_line.cmp(&b.start_line))
                .then_with(|| a.name.cmp(&b.name))
        })
    });
    rows.truncate(top);
    rows
}

pub fn rank_files(report: &DisciplineReport, sort: DisciplineSort, top: usize) -> Vec<&FileDiscipline> {
    let mut rows: Vec<&FileDiscipline> = report.files.iter().collect();
    rows.sort_by(|a, b| {
        b.sort_key(sort)
            .cmp(&a.sort_key(sort))
            .then_with(|| a.path.cmp(&b.path))
    });
    rows.truncate(top);
    rows
}

pub fn analyze_discipline(input: &Path) -> Result<DisciplineReport, DisciplineError> {
    let tree = load_source_tree(input)?;
    let mut files = Vec::with_capacity(tree.files.len());
    let mut functions = Vec::new();
    let mut syntax_error_files = 0u64;

    for file in &tree.files {
        let parsed = parse_source(file)?;
        if parsed.has_syntax_errors {
            syntax_error_files += 1;
        }
        let is_test = classify_file(file) == FileRole::Test;
        let root = parsed.tree.root_node();

        let mut rows = Vec::new();
        let mut stack: Vec<Frame> = Vec::new();
        walk(file, root, &mut stack, &mut rows);

        let mut sums = DisciplineTotals::default();
        for row in &rows {
            sums.add_function(row);
        }
        if !is_test {
            (sums.magic_numbers, sums.magic_strings) = count_magic_literals(file, root);
        }
        sums.global_mutable_state = count_global_mutable_state(file, root);

        files.push(FileDiscipline {
            path: file.path.clone(),
            language: file.language,
            syntax_errors: parsed.has_syntax_errors,
            functions: rows.len() as u64,
            sums,
        });
        functions.append(&mut rows);
    }

    functions.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.start_line.cmp(&b.start_line))
            .then_with(|| a.name.cmp(&b.name))
    });
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let coverage = coverage(&tree, &files, &functions, syntax_error_files);

    Ok(DisciplineReport {
        root: tree.root,
        analyzer: "tree-sitter-structural-discipline-v1".to_owned(),
        coverage,
        files,
        functions,
        limitations: vec![
            "Syntactic purity is not semantic purity: unresolved calls, trait/dynamic dispatch, and closures passed as arguments can hide effects a pure-looking function triggers.".to_owned(),
            "Effect, panic, and error spellings are documented prefix/name lists, not resolved namespaces; they over-match shared prefixes and miss aliased or unlisted effects.".to_owned(),
            "No symbol, type, or call is resolved across files; bindings, shadowing, and mutability are judged within one function space only.".to_owned(),
            "Per-language uncovered fields report 0 by construction: mut_params (Python/JS/TS), unsafe_blocks/try_propagations(Rust `?`)/unwrap_expect/panic_like beyond Rust+Go, any_annotations (TS only), and shadowings for Python's flat function scope.".to_owned(),
            "Files with tree-sitter syntax errors stay in the denominator and are flagged; their error-tolerant trees may make individual counters partial, but they inflate nothing on their own.".to_owned(),
            "magic_numbers, magic_strings, and global_mutable_state are file-level structural proxies; magic_strings counts every string literal of interior length >= 2 outside const/type declarations (messages, keys, and Python non-docstring module strings included), so it is a volume, not a smell count.".to_owned(),
        ],
    })
}

fn coverage(
    tree: &crate::source::SourceTree,
    files: &[FileDiscipline],
    functions: &[FunctionDiscipline],
    syntax_error_files: u64,
) -> DisciplineCoverage {
    let mut totals = DisciplineTotals::default();
    for file in files {
        totals.add(&file.sums);
    }
    let functions_total = functions.len() as u64;
    let pure_functions = functions.iter().filter(|f| f.syntactically_pure).count() as u64;
    let pure_fraction = (functions_total != 0).then(|| pure_functions as f64 / functions_total as f64);

    let mut per_language: BTreeMap<SourceLanguage, u64> = BTreeMap::new();
    for function in functions {
        *per_language.entry(function.language).or_default() += 1;
    }
    let functions_per_language = per_language
        .into_iter()
        .map(|(language, functions)| LanguageFunctionCount { language, functions })
        .collect();

    let tails = DisciplineTails {
        mutable_bindings: tail(functions, |f| f.mutable_bindings),
        max_mutable_live_range_lines: tail(functions, |f| f.max_mutable_live_range_lines),
        max_call_chain_len: tail(functions, |f| f.max_call_chain_len),
        params: tail(functions, |f| f.params),
    };

    DisciplineCoverage {
        enumerated_files: tree.enumerated,
        supported_files: tree.files.len(),
        skipped_unsupported_files: tree.skipped,
        syntax_error_files,
        functions_total,
        functions_per_language,
        totals,
        pure_functions,
        pure_fraction,
        tails,
    }
}

fn tail(functions: &[FunctionDiscipline], select: impl Fn(&FunctionDiscipline) -> u32) -> Tail {
    let mut values = functions
        .iter()
        .map(|f| f64::from(select(f)))
        .collect::<Vec<_>>();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let at = |p| nearest_rank(&values, p).map_or(0, |value| value as u64);
    Tail {
        p50: at(50),
        p90: at(90),
        p99: at(99),
    }
}

// ---------------------------------------------------------------------------
// Single-walk attribution.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct BindingDecl {
    name: String,
    line: usize,
    mutable_decl: bool,
}

#[derive(Clone)]
struct Write {
    root: String,
    line: usize,
    augmented: bool,
    through_ref: bool,
}

struct Frame {
    language: SourceLanguage,
    name: String,
    start_line: usize,
    end_line: usize,
    syntax_errors: bool,
    // header-derived
    param_names: Vec<String>,
    params: u32,
    bool_params: u32,
    unannotated_params: u32,
    mut_params: u32,
    single_expression_body: bool,
    unit_return_structural: Option<bool>,
    // collected events
    decls: Vec<BindingDecl>,
    writes: Vec<Write>,
    forced_nonlocal: BTreeSet<String>,
    has_return_value: bool,
    // directly accumulated
    unsafe_blocks: u32,
    effect_calls: u32,
    try_propagations: u32,
    unwrap_expect: u32,
    panic_like: u32,
    broad_catches: u32,
    empty_catches: u32,
    ignored_results: u32,
    string_literal_conditions: u32,
    any_annotations: u32,
    type_ignores: u32,
    statements: u32,
    max_call_chain_len: u32,
}

fn walk<'a>(
    file: &'a SourceFile,
    node: Node<'a>,
    stack: &mut Vec<Frame>,
    rows: &mut Vec<FunctionDiscipline>,
) {
    let is_space = is_function_space(file.language, node.kind());
    if is_space {
        stack.push(Frame::new(file, node));
    } else if let Some(frame) = stack.last_mut() {
        observe(file, node, frame);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(file, child, stack, rows);
    }
    if is_space {
        let frame = stack.pop().expect("frame pushed on entry");
        let mut row = frame.finalize();
        row.path = file.path.clone();
        rows.push(row);
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
        SourceLanguage::Go => {
            matches!(kind, "function_declaration" | "method_declaration" | "func_literal")
        }
    }
}

impl Frame {
    fn new(file: &SourceFile, node: Node<'_>) -> Frame {
        let language = file.language;
        let name = function_name(file, node);
        let header = header_info(file, node);
        Frame {
            language,
            name,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            syntax_errors: node.has_error(),
            param_names: header.param_names,
            params: header.params,
            bool_params: header.bool_params,
            unannotated_params: header.unannotated_params,
            mut_params: header.mut_params,
            single_expression_body: header.single_expression_body,
            unit_return_structural: header.unit_return_structural,
            decls: Vec::new(),
            writes: Vec::new(),
            forced_nonlocal: BTreeSet::new(),
            has_return_value: false,
            unsafe_blocks: 0,
            effect_calls: 0,
            try_propagations: 0,
            unwrap_expect: 0,
            panic_like: 0,
            broad_catches: 0,
            empty_catches: 0,
            ignored_results: 0,
            string_literal_conditions: 0,
            any_annotations: 0,
            type_ignores: 0,
            statements: 0,
            max_call_chain_len: 0,
        }
    }

    fn finalize(self) -> FunctionDiscipline {
        let language = self.language;
        let param_set: BTreeSet<&str> = self.param_names.iter().map(String::as_str).collect();

        // Declared locals = parameters + language declarations. For Python the
        // declarations are the first-seen plain assignments, synthesized here.
        let mut decls = self.decls.clone();
        if language == SourceLanguage::Python {
            let mut seen: BTreeSet<String> = param_set.iter().map(|s| (*s).to_owned()).collect();
            for write in &self.writes {
                if write.augmented || write.through_ref {
                    continue;
                }
                if self.forced_nonlocal.contains(&write.root) {
                    continue;
                }
                if seen.insert(write.root.clone()) {
                    decls.push(BindingDecl {
                        name: write.root.clone(),
                        line: write.line,
                        mutable_decl: false,
                    });
                }
            }
        }

        let declared_locals: BTreeSet<&str> = param_set
            .iter()
            .copied()
            .chain(decls.iter().map(|d| d.name.as_str()))
            .collect();

        // nonlocal writes
        let nonlocal_writes = self
            .writes
            .iter()
            .filter(|w| {
                self.forced_nonlocal.contains(&w.root)
                    || !declared_locals.contains(w.root.as_str())
                    || (w.through_ref && param_set.contains(w.root.as_str()))
            })
            .count() as u32;

        // reassignments
        let reassignments = if language == SourceLanguage::Python {
            let mut seen: BTreeSet<&str> = param_set.clone();
            let mut count = 0u32;
            for write in &self.writes {
                if write.through_ref
                    || self.forced_nonlocal.contains(&write.root)
                    || !declared_locals.contains(write.root.as_str())
                {
                    continue;
                }
                let first_declaration = !write.augmented && seen.insert(write.root.as_str());
                if !first_declaration {
                    count += 1;
                }
            }
            count
        } else {
            self.writes
                .iter()
                .filter(|w| {
                    !w.through_ref
                        && !self.forced_nonlocal.contains(&w.root)
                        && declared_locals.contains(w.root.as_str())
                })
                .count() as u32
        };

        // shadowings: a non-parameter declaration whose name is already bound.
        let mut seen: BTreeSet<&str> = param_set.clone();
        let mut shadowings = 0u32;
        for decl in &decls {
            if !seen.insert(decl.name.as_str()) {
                shadowings += 1;
            }
        }

        // mutable bindings + live range
        let write_count = |name: &str| {
            self.writes
                .iter()
                .filter(|w| !w.through_ref && w.root == name)
                .count()
        };
        let last_write_line = |name: &str, decl_line: usize| {
            self.writes
                .iter()
                .filter(|w| !w.through_ref && w.root == name)
                .map(|w| w.line)
                .max()
                .unwrap_or(decl_line)
                .max(decl_line)
        };
        let mut mutable_bindings = 0u32;
        let mut max_live_range = 0u32;
        for decl in &decls {
            let is_mutable = match language {
                SourceLanguage::Rust
                | SourceLanguage::JavaScript
                | SourceLanguage::TypeScript
                | SourceLanguage::Tsx => decl.mutable_decl,
                SourceLanguage::Go => write_count(&decl.name) > 0,
                SourceLanguage::Python => write_count(&decl.name) > 1,
            };
            if is_mutable {
                mutable_bindings += 1;
                let range = last_write_line(&decl.name, decl.line).saturating_sub(decl.line) as u32;
                max_live_range = max_live_range.max(range);
            }
        }

        // Without a structural return type, a value-yielding expression body
        // (lambda / arrow) is not unit-returning even though it has no `return`.
        let unit_return = self
            .unit_return_structural
            .unwrap_or(!self.has_return_value && !self.single_expression_body);

        let syntactically_pure = nonlocal_writes == 0
            && self.mut_params == 0
            && self.unsafe_blocks == 0
            && self.effect_calls == 0;

        FunctionDiscipline {
            path: String::new(),
            language,
            name: self.name,
            start_line: self.start_line,
            end_line: self.end_line,
            syntax_errors: self.syntax_errors,
            nonlocal_writes,
            mut_params: self.mut_params,
            unsafe_blocks: self.unsafe_blocks,
            effect_calls: self.effect_calls,
            syntactically_pure,
            bindings: decls.len() as u32 + param_set.len() as u32,
            mutable_bindings,
            reassignments,
            shadowings,
            max_mutable_live_range_lines: max_live_range,
            params: self.params,
            bool_params: self.bool_params,
            statements: self.statements,
            single_expression_body: self.single_expression_body,
            unit_return,
            max_call_chain_len: self.max_call_chain_len,
            try_propagations: self.try_propagations,
            unwrap_expect: self.unwrap_expect,
            panic_like: self.panic_like,
            broad_catches: self.broad_catches,
            empty_catches: self.empty_catches,
            ignored_results: self.ignored_results,
            string_literal_conditions: self.string_literal_conditions,
            any_annotations: self.any_annotations,
            unannotated_params: self.unannotated_params,
            type_ignores: self.type_ignores,
        }
    }
}

// ---------------------------------------------------------------------------
// Header (parameters, body shape) derived once when a frame is pushed.
// ---------------------------------------------------------------------------

struct HeaderInfo {
    param_names: Vec<String>,
    params: u32,
    bool_params: u32,
    unannotated_params: u32,
    mut_params: u32,
    single_expression_body: bool,
    unit_return_structural: Option<bool>,
}

fn function_name(file: &SourceFile, node: Node<'_>) -> String {
    if let Some(name) = node.child_by_field_name("name") {
        return text(file, name).to_owned();
    }
    match file.language {
        SourceLanguage::Python if node.kind() == "lambda" => "<lambda>".to_owned(),
        _ => "<closure>".to_owned(),
    }
}

fn header_info(file: &SourceFile, node: Node<'_>) -> HeaderInfo {
    let language = file.language;
    let mut param_names = Vec::new();
    let mut params = 0u32;
    let mut bool_params = 0u32;
    let mut unannotated_params = 0u32;
    let mut mut_params = 0u32;

    // Go method receiver: a local binding and a possible pointer mutation.
    if language == SourceLanguage::Go
        && let Some(receiver) = node.child_by_field_name("receiver")
    {
        let mut cursor = receiver.walk();
        for decl in receiver.children(&mut cursor) {
            if decl.kind() == "parameter_declaration" {
                if let Some(name) = decl.child_by_field_name("name") {
                    param_names.push(text(file, name).to_owned());
                }
                if decl
                    .child_by_field_name("type")
                    .is_some_and(|t| t.kind() == "pointer_type")
                {
                    mut_params += 1;
                }
            }
        }
    }

    let params_node = node.child_by_field_name("parameters").or_else(|| {
        // A JS arrow function with a single bare parameter uses `parameter`.
        node.child_by_field_name("parameter")
    });
    if let Some(params_node) = params_node {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            visit_param(
                file,
                language,
                child,
                &mut param_names,
                &mut params,
                &mut bool_params,
                &mut unannotated_params,
                &mut mut_params,
            );
        }
        if params_node.kind() == "identifier" || params_node.kind() == "parameter" {
            // Single unparenthesized arrow parameter.
            visit_param(
                file,
                language,
                params_node,
                &mut param_names,
                &mut params,
                &mut bool_params,
                &mut unannotated_params,
                &mut mut_params,
            );
        }
    }

    let single_expression_body = single_expression_body(file, node);
    let unit_return_structural = unit_return_structural(file, node);

    HeaderInfo {
        param_names,
        params,
        bool_params,
        unannotated_params,
        mut_params,
        single_expression_body,
        unit_return_structural,
    }
}

#[allow(clippy::too_many_arguments)]
fn visit_param(
    file: &SourceFile,
    language: SourceLanguage,
    child: Node<'_>,
    names: &mut Vec<String>,
    params: &mut u32,
    bool_params: &mut u32,
    unannotated_params: &mut u32,
    mut_params: &mut u32,
) {
    match language {
        SourceLanguage::Rust => match child.kind() {
            "parameter" => {
                *params += 1;
                if let Some(pattern) = child.child_by_field_name("pattern") {
                    collect_pattern_identifiers(file, pattern, names);
                }
                if let Some(ty) = child.child_by_field_name("type") {
                    if ty.kind() == "primitive_type" && text(file, ty) == "bool" {
                        *bool_params += 1;
                    }
                    if ty.kind() == "reference_type" && has_child_kind(ty, "mutable_specifier") {
                        *mut_params += 1;
                    }
                }
            }
            "self_parameter" if has_child_kind(child, "mutable_specifier") => *mut_params += 1,
            _ => {}
        },
        SourceLanguage::Python => match child.kind() {
            "identifier" => {
                let name = text(file, child);
                names.push(name.to_owned());
                *params += 1;
                if name != "self" && name != "cls" {
                    *unannotated_params += 1;
                }
            }
            "default_parameter" => {
                *params += 1;
                if let Some(name) = child.child_by_field_name("name") {
                    let text = text(file, name);
                    names.push(text.to_owned());
                    if text != "self" && text != "cls" {
                        *unannotated_params += 1;
                    }
                }
                if let Some(value) = child.child_by_field_name("value")
                    && matches!(value.kind(), "true" | "false")
                {
                    *bool_params += 1;
                }
            }
            "typed_parameter" | "typed_default_parameter" => {
                *params += 1;
                if let Some(name) = child.named_child(0) {
                    names.push(text(file, name).to_owned());
                }
                if let Some(ty) = child.child_by_field_name("type")
                    && text(file, ty).trim() == "bool"
                {
                    *bool_params += 1;
                }
            }
            "list_splat_pattern" | "dictionary_splat_pattern" => {
                *params += 1;
                if let Some(name) = child.named_child(0) {
                    names.push(text(file, name).to_owned());
                }
            }
            _ => {}
        },
        SourceLanguage::JavaScript => match child.kind() {
            "identifier" | "assignment_pattern" | "object_pattern" | "array_pattern"
            | "rest_pattern" => {
                *params += 1;
                collect_pattern_identifiers(file, child, names);
            }
            _ => {}
        },
        SourceLanguage::TypeScript | SourceLanguage::Tsx => match child.kind() {
            "required_parameter" | "optional_parameter" => {
                *params += 1;
                if let Some(pattern) = child.child_by_field_name("pattern") {
                    collect_pattern_identifiers(file, pattern, names);
                }
                match child.child_by_field_name("type") {
                    None => *unannotated_params += 1,
                    Some(annotation) => {
                        if annotation_is(file, annotation, "boolean") {
                            *bool_params += 1;
                        }
                    }
                }
            }
            "identifier" | "assignment_pattern" | "object_pattern" | "array_pattern"
            | "rest_pattern" => {
                *params += 1;
                *unannotated_params += 1;
                collect_pattern_identifiers(file, child, names);
            }
            _ => {}
        },
        SourceLanguage::Go => {
            if child.kind() == "parameter_declaration"
                || child.kind() == "variadic_parameter_declaration"
            {
                let names_here: Vec<Node<'_>> = {
                    let mut cursor = child.walk();
                    child
                        .children_by_field_name("name", &mut cursor)
                        .collect()
                };
                let is_pointer = child
                    .child_by_field_name("type")
                    .is_some_and(|t| t.kind() == "pointer_type");
                let is_bool = child
                    .child_by_field_name("type")
                    .is_some_and(|t| t.kind() == "type_identifier" && text(file, t) == "bool");
                let count = names_here.len().max(1) as u32;
                *params += count;
                if is_pointer {
                    *mut_params += count;
                }
                if is_bool {
                    *bool_params += count;
                }
                for name in names_here {
                    names.push(text(file, name).to_owned());
                }
            }
        }
    }
}

fn annotation_is(file: &SourceFile, annotation: Node<'_>, wanted: &str) -> bool {
    // A type_annotation wraps `: <type>`; check its last named child.
    annotation
        .named_child((annotation.named_child_count() as u32).saturating_sub(1))
        .is_some_and(|ty| text(file, ty) == wanted)
}

fn collect_pattern_identifiers(file: &SourceFile, node: Node<'_>, names: &mut Vec<String>) {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            let text = text(file, node);
            if text != "_" {
                names.push(text.to_owned());
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_pattern_identifiers(file, child, names);
            }
        }
    }
}

fn single_expression_body(file: &SourceFile, node: Node<'_>) -> bool {
    match file.language {
        SourceLanguage::Rust => match node.kind() {
            "closure_expression" => node
                .child_by_field_name("body")
                .is_some_and(|body| body.kind() != "block"),
            _ => node.child_by_field_name("body").is_some_and(|body| {
                body.kind() == "block" && block_is_single_expression(body)
            }),
        },
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            node.kind() == "arrow_function"
                && node
                    .child_by_field_name("body")
                    .is_some_and(|body| body.kind() != "statement_block")
        }
        SourceLanguage::Python => node.kind() == "lambda",
        SourceLanguage::Go => false,
    }
}

fn block_is_single_expression(block: Node<'_>) -> bool {
    let mut cursor = block.walk();
    let mut statements = 0;
    let mut tail = false;
    for child in block.named_children(&mut cursor) {
        match child.kind() {
            "let_declaration" | "expression_statement" | "line_comment" | "block_comment"
            | "empty_statement" => statements += 1,
            _ => tail = true,
        }
    }
    statements == 0 && tail
}

fn unit_return_structural(file: &SourceFile, node: Node<'_>) -> Option<bool> {
    match file.language {
        SourceLanguage::Rust => Some(match node.child_by_field_name("return_type") {
            None => true,
            Some(ty) => text(file, ty).trim() == "()",
        }),
        SourceLanguage::Go => Some(node.child_by_field_name("result").is_none()),
        SourceLanguage::TypeScript | SourceLanguage::Tsx => node
            .child_by_field_name("return_type")
            .map(|annotation| annotation_is(file, annotation, "void")),
        SourceLanguage::JavaScript | SourceLanguage::Python => None,
    }
}

// ---------------------------------------------------------------------------
// Per-node attribution into the innermost frame.
// ---------------------------------------------------------------------------

fn observe(file: &SourceFile, node: Node<'_>, frame: &mut Frame) {
    let language = file.language;
    let kind = node.kind();
    let line = node.start_position().row + 1;

    if is_statement_kind(language, kind) {
        frame.statements += 1;
    }

    // Effect / method-chain accounting for any call-shaped node.
    if is_call_kind(language, kind) {
        if let Some(callee) = callee_text(file, node) {
            if matches_effect(language, &callee) {
                frame.effect_calls += 1;
            }
            if language == SourceLanguage::Go && callee == "panic" {
                frame.panic_like += 1;
            }
        }
        let chain = chain_length(node);
        frame.max_call_chain_len = frame.max_call_chain_len.max(chain);
    }

    // Comments carry type-suppression pragmas.
    if kind == "comment" {
        let body = text(file, node);
        match language {
            SourceLanguage::Python if body.contains("type: ignore") => frame.type_ignores += 1,
            SourceLanguage::TypeScript | SourceLanguage::Tsx
                if body.contains("@ts-ignore") || body.contains("@ts-expect-error") =>
            {
                frame.type_ignores += 1;
            }
            _ => {}
        }
    }

    if matches!(language, SourceLanguage::TypeScript | SourceLanguage::Tsx)
        && kind == "predefined_type"
        && text(file, node) == "any"
    {
        frame.any_annotations += 1;
    }

    match language {
        SourceLanguage::Rust => observe_rust(file, node, frame, line),
        SourceLanguage::Python => observe_python(file, node, frame, line),
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            observe_js(file, node, frame, line)
        }
        SourceLanguage::Go => observe_go(file, node, frame, line),
    }
}

fn observe_rust(file: &SourceFile, node: Node<'_>, frame: &mut Frame, line: usize) {
    match node.kind() {
        "unsafe_block" => frame.unsafe_blocks += 1,
        "try_expression" => frame.try_propagations += 1,
        "let_declaration" => {
            let mutable = has_child_kind(node, "mutable_specifier");
            if let Some(pattern) = node.child_by_field_name("pattern") {
                let mut names = Vec::new();
                collect_pattern_identifiers(file, pattern, &mut names);
                for name in names {
                    frame.decls.push(BindingDecl {
                        name,
                        line,
                        mutable_decl: mutable,
                    });
                }
                if text(file, pattern).trim() == "_"
                    && node
                        .child_by_field_name("value")
                        .is_some_and(|v| v.kind() == "call_expression")
                {
                    frame.ignored_results += 1;
                }
            }
        }
        "assignment_expression" | "compound_assignment_expr" => {
            let augmented = node.kind() == "compound_assignment_expr";
            if let Some(left) = node.child_by_field_name("left") {
                collect_targets(file, left, augmented, line, &mut frame.writes);
            }
        }
        "call_expression" => {
            if let Some(function) = node.child_by_field_name("function")
                && function.kind() == "field_expression"
                && let Some(field) = function.child_by_field_name("field")
                && matches!(text(file, field), "unwrap" | "expect")
            {
                frame.unwrap_expect += 1;
            }
        }
        "macro_invocation" => {
            if let Some(macro_name) = node.child_by_field_name("macro")
                && RUST_PANIC_MACROS.contains(&text(file, macro_name))
            {
                frame.panic_like += 1;
            }
        }
        "if_expression" | "while_expression" => {
            if let Some(condition) = node.child_by_field_name("condition") {
                scan_conditions(file, SourceLanguage::Rust, condition, &mut frame.string_literal_conditions);
            }
        }
        "match_expression" => {
            frame.string_literal_conditions += match_string_arms(node);
        }
        _ => {}
    }
}

fn observe_python(file: &SourceFile, node: Node<'_>, frame: &mut Frame, line: usize) {
    match node.kind() {
        "global_statement" | "nonlocal_statement" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "identifier" {
                    frame.forced_nonlocal.insert(text(file, child).to_owned());
                }
            }
        }
        "assignment" | "augmented_assignment" => {
            let augmented = node.kind() == "augmented_assignment";
            if let Some(left) = node.child_by_field_name("left") {
                collect_targets(file, left, augmented, line, &mut frame.writes);
            }
        }
        "return_statement" => {
            if node.named_child_count() > 0 {
                frame.has_return_value = true;
            }
        }
        "except_clause" => observe_except(file, node, frame),
        "if_statement" | "elif_clause" | "while_statement" => {
            if let Some(condition) = node.child_by_field_name("condition") {
                scan_conditions(file, SourceLanguage::Python, condition, &mut frame.string_literal_conditions);
            }
        }
        _ => {}
    }
}

fn observe_js(file: &SourceFile, node: Node<'_>, frame: &mut Frame, line: usize) {
    let language = file.language;
    match node.kind() {
        "lexical_declaration" | "variable_declaration" => {
            let mutable = node
                .child(0)
                .is_some_and(|keyword| matches!(text(file, keyword), "let" | "var"));
            let mut cursor = node.walk();
            for declarator in node.named_children(&mut cursor) {
                if declarator.kind() == "variable_declarator"
                    && let Some(name) = declarator.child_by_field_name("name")
                {
                    let mut names = Vec::new();
                    collect_pattern_identifiers(file, name, &mut names);
                    for name in names {
                        frame.decls.push(BindingDecl {
                            name,
                            line,
                            mutable_decl: mutable,
                        });
                    }
                }
            }
        }
        "assignment_expression" | "augmented_assignment_expression" => {
            let augmented = node.kind() == "augmented_assignment_expression";
            if let Some(left) = node.child_by_field_name("left") {
                collect_targets(file, left, augmented, line, &mut frame.writes);
            }
        }
        "return_statement" => {
            if node.named_child_count() > 0 {
                frame.has_return_value = true;
            }
        }
        "catch_clause" => observe_catch(file, node, frame),
        "if_statement" | "while_statement" | "do_statement" => {
            if let Some(condition) = node.child_by_field_name("condition") {
                scan_conditions(file, language, condition, &mut frame.string_literal_conditions);
            }
        }
        "switch_statement" => {
            frame.string_literal_conditions += switch_string_cases(language, node);
        }
        _ => {}
    }
}

fn observe_go(file: &SourceFile, node: Node<'_>, frame: &mut Frame, line: usize) {
    match node.kind() {
        "short_var_declaration" => {
            if let Some(left) = node.child_by_field_name("left") {
                let mut cursor = left.walk();
                for target in left.named_children(&mut cursor) {
                    if target.kind() == "identifier" {
                        let name = text(file, target);
                        if name != "_" {
                            frame.decls.push(BindingDecl {
                                name: name.to_owned(),
                                line,
                                mutable_decl: false,
                            });
                        }
                    }
                }
            }
        }
        "var_declaration" | "const_declaration" => {
            let is_var = node.kind() == "var_declaration";
            let mut cursor = node.walk();
            for spec in node.named_children(&mut cursor) {
                if matches!(spec.kind(), "var_spec" | "const_spec") {
                    let mut inner = spec.walk();
                    for name in spec.children_by_field_name("name", &mut inner) {
                        if is_var {
                            frame.decls.push(BindingDecl {
                                name: text(file, name).to_owned(),
                                line,
                                mutable_decl: false,
                            });
                        }
                    }
                }
            }
        }
        "assignment_statement" => observe_go_assignment(file, node, frame, line),
        "if_statement" => {
            if let Some(condition) = node.child_by_field_name("condition") {
                if is_go_err_check(file, condition)
                    && node
                        .child_by_field_name("consequence")
                        .is_some_and(|block| block_returns(block))
                {
                    frame.try_propagations += 1;
                }
                scan_conditions(file, SourceLanguage::Go, condition, &mut frame.string_literal_conditions);
            }
        }
        "expression_switch_statement" => {
            frame.string_literal_conditions += switch_string_cases(SourceLanguage::Go, node);
        }
        _ => {}
    }
}

fn observe_go_assignment(file: &SourceFile, node: Node<'_>, frame: &mut Frame, line: usize) {
    let operator = node
        .children(&mut node.walk())
        .find(|child| !child.is_named())
        .map(|op| text(file, op).to_owned())
        .unwrap_or_default();
    let augmented = operator != "=";
    let (Some(left), right) = (
        node.child_by_field_name("left"),
        node.child_by_field_name("right"),
    ) else {
        return;
    };
    let lefts: Vec<Node<'_>> = {
        let mut cursor = left.walk();
        left.named_children(&mut cursor).collect()
    };
    let all_blank =
        !lefts.is_empty() && lefts.iter().all(|n| n.kind() == "identifier" && text(file, *n) == "_");
    if operator == "=" && all_blank {
        let rights: Vec<Node<'_>> = right.map_or_else(Vec::new, |right| {
            let mut cursor = right.walk();
            right.named_children(&mut cursor).collect()
        });
        if lefts.len() == 1 && rights.len() == 1 && rights[0].kind() == "call_expression" {
            frame.ignored_results += 1;
        } else {
            frame.broad_catches += 1;
        }
        return;
    }
    collect_targets(file, left, augmented, line, &mut frame.writes);
}

fn observe_except(file: &SourceFile, node: Node<'_>, frame: &mut Frame) {
    // The exception type, if any, is the first named child before the block;
    // `except E as name` wraps it in an `as_pattern`, so unwrap that.
    let mut cursor = node.walk();
    let type_text = node
        .named_children(&mut cursor)
        .find(|child| child.kind() != "block")
        .map(|child| {
            let type_node = if child.kind() == "as_pattern" {
                child.named_child(0).unwrap_or(child)
            } else {
                child
            };
            text(file, type_node).trim().to_owned()
        });
    let broad = match &type_text {
        None => true,
        Some(text) => text == "Exception" || text == "BaseException",
    };
    if broad {
        frame.broad_catches += 1;
    }
    if let Some(block) = node.child_by_field_name("consequence").or_else(|| {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .find(|child| child.kind() == "block")
    }) && block_is_empty_or_pass(block)
    {
        frame.empty_catches += 1;
    }
}

fn observe_catch(file: &SourceFile, node: Node<'_>, frame: &mut Frame) {
    let Some(body) = node.child_by_field_name("body") else {
        return;
    };
    let statements: Vec<Node<'_>> = {
        let mut cursor = body.walk();
        body.named_children(&mut cursor)
            .filter(|child| child.kind() != "comment")
            .collect()
    };
    if statements.is_empty() {
        frame.empty_catches += 1;
        frame.broad_catches += 1;
        return;
    }
    let only_logging = statements.iter().all(|statement| {
        statement.kind() == "expression_statement"
            && statement
                .named_child(0)
                .is_some_and(|expression| is_logging_call(file, expression))
    });
    if only_logging {
        frame.broad_catches += 1;
    }
}

fn is_logging_call(file: &SourceFile, node: Node<'_>) -> bool {
    node.kind() == "call_expression"
        && node.child_by_field_name("function").is_some_and(|callee| {
            let text = normalize_ws(text(file, callee));
            text.starts_with("console.") || text.ends_with(".log") || text == "log"
        })
}

// ---------------------------------------------------------------------------
// Shared syntactic helpers.
// ---------------------------------------------------------------------------

fn text<'a>(file: &'a SourceFile, node: Node<'_>) -> &'a str {
    std::str::from_utf8(&file.bytes[node.byte_range()]).unwrap_or("")
}

fn normalize_ws(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has_child_kind(node: Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|child| child.kind() == kind)
}

fn is_statement_kind(language: SourceLanguage, kind: &str) -> bool {
    match language {
        SourceLanguage::Rust => matches!(kind, "expression_statement" | "let_declaration"),
        SourceLanguage::Python => kind.ends_with("_statement"),
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            kind.ends_with("_statement") || matches!(kind, "lexical_declaration" | "variable_declaration")
        }
        SourceLanguage::Go => {
            kind.ends_with("_statement")
                || matches!(kind, "short_var_declaration" | "var_declaration" | "const_declaration")
        }
    }
}

fn is_call_kind(language: SourceLanguage, kind: &str) -> bool {
    match language {
        SourceLanguage::Rust => matches!(kind, "call_expression" | "macro_invocation"),
        SourceLanguage::Python => kind == "call",
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            matches!(kind, "call_expression" | "new_expression")
        }
        SourceLanguage::Go => kind == "call_expression",
    }
}

fn callee_text(file: &SourceFile, node: Node<'_>) -> Option<String> {
    match node.kind() {
        "call" | "call_expression" => node
            .child_by_field_name("function")
            .map(|function| normalize_ws(text(file, function))),
        "macro_invocation" => node
            .child_by_field_name("macro")
            .map(|macro_name| format!("{}!", text(file, macro_name))),
        "new_expression" => node
            .child_by_field_name("constructor")
            .map(|constructor| format!("new {}", text(file, constructor))),
        _ => None,
    }
}

fn effect_prefixes(language: SourceLanguage) -> &'static [&'static str] {
    match language {
        SourceLanguage::Rust => RUST_EFFECT_PREFIXES,
        SourceLanguage::Python => PYTHON_EFFECT_PREFIXES,
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            JS_EFFECT_PREFIXES
        }
        SourceLanguage::Go => GO_EFFECT_PREFIXES,
    }
}

fn matches_effect(language: SourceLanguage, callee: &str) -> bool {
    let with_paren = format!("{callee}(");
    effect_prefixes(language)
        .iter()
        .any(|prefix| callee.starts_with(prefix) || with_paren.starts_with(prefix))
}

fn child_field<'a>(node: Node<'a>, field: &str) -> Option<Node<'a>> {
    node.child_by_field_name(field)
}

fn assign_target_root(file: &SourceFile, node: Node<'_>) -> Option<(String, bool)> {
    let mut cur = node;
    let mut through = false;
    loop {
        match cur.kind() {
            "identifier" | "field_identifier" | "property_identifier" | "type_identifier"
            | "shorthand_property_identifier" => {
                return Some((text(file, cur).to_owned(), through));
            }
            "field_expression" => {
                through = true;
                cur = child_field(cur, "value")?;
            }
            "member_expression" => {
                through = true;
                cur = child_field(cur, "object")?;
            }
            "subscript_expression" => {
                through = true;
                cur = child_field(cur, "object")?;
            }
            "attribute" => {
                through = true;
                cur = child_field(cur, "object")?;
            }
            "subscript" => {
                through = true;
                cur = child_field(cur, "value")?;
            }
            "selector_expression" => {
                through = true;
                cur = child_field(cur, "operand")?;
            }
            "index_expression" => {
                through = true;
                cur = cur
                    .child_by_field_name("operand")
                    .or_else(|| cur.named_child(0))?;
            }
            "unary_expression" | "pointer_expression" => {
                through = true;
                cur = cur.named_child((cur.named_child_count() as u32).saturating_sub(1))?;
            }
            "parenthesized_expression" => {
                cur = cur.named_child(0)?;
            }
            _ => return None,
        }
    }
}

fn collect_targets(
    file: &SourceFile,
    node: Node<'_>,
    augmented: bool,
    line: usize,
    out: &mut Vec<Write>,
) {
    match node.kind() {
        "expression_list" | "tuple_pattern" | "pattern_list" | "tuple" | "array_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_targets(file, child, augmented, line, out);
            }
        }
        _ => {
            if let Some((root, through_ref)) = assign_target_root(file, node)
                && root != "_"
            {
                out.push(Write {
                    root,
                    line,
                    augmented,
                    through_ref,
                });
            }
        }
    }
}

fn chain_length(call: Node<'_>) -> u32 {
    let mut cur = call;
    let mut length = 0;
    loop {
        let is_method = call_callee(cur).is_some_and(|callee| {
            matches!(
                callee.kind(),
                "field_expression" | "member_expression" | "attribute" | "selector_expression"
            )
        });
        if !is_method {
            break;
        }
        length += 1;
        match method_receiver(cur) {
            Some(receiver) => cur = receiver,
            None => break,
        }
    }
    length
}

fn call_callee(call: Node<'_>) -> Option<Node<'_>> {
    call.child_by_field_name("function")
}

fn method_receiver(call: Node<'_>) -> Option<Node<'_>> {
    let callee = call_callee(call)?;
    let object = match callee.kind() {
        "field_expression" => callee.child_by_field_name("value"),
        "member_expression" => callee.child_by_field_name("object"),
        "attribute" => callee.child_by_field_name("object"),
        "selector_expression" => callee.child_by_field_name("operand"),
        _ => None,
    }?;
    matches!(object.kind(), "call" | "call_expression").then_some(object)
}

fn is_string_literal(kind: &str) -> bool {
    matches!(
        kind,
        "string_literal"
            | "raw_string_literal"
            | "string"
            | "template_string"
            | "interpreted_string_literal"
    )
}

fn scan_conditions(file: &SourceFile, language: SourceLanguage, node: Node<'_>, count: &mut u32) {
    if is_eq_string_comparison(file, language, node) {
        *count += 1;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        scan_conditions(file, language, child, count);
    }
}

fn is_eq_string_comparison(file: &SourceFile, language: SourceLanguage, node: Node<'_>) -> bool {
    let is_comparison = match language {
        SourceLanguage::Python => matches!(node.kind(), "comparison_operator"),
        _ => node.kind() == "binary_expression",
    };
    if !is_comparison {
        return false;
    }
    let mut cursor = node.walk();
    let mut has_eq_op = false;
    let mut has_identifier = false;
    let mut has_string = false;
    for child in node.children(&mut cursor) {
        if child.is_named() {
            if child.kind() == "identifier" {
                has_identifier = true;
            }
            if is_string_literal(child.kind()) {
                has_string = true;
            }
        } else if matches!(text(file, child), "==" | "!=" | "===" | "!==") {
            has_eq_op = true;
        }
    }
    has_eq_op && has_identifier && has_string
}

fn match_string_arms(node: Node<'_>) -> u32 {
    let Some(value) = node.child_by_field_name("value") else {
        return 0;
    };
    if value.kind() != "identifier" {
        return 0;
    }
    let Some(body) = node.child_by_field_name("body") else {
        return 0;
    };
    let mut cursor = body.walk();
    body.named_children(&mut cursor)
        .filter(|arm| {
            arm.kind() == "match_arm"
                && arm
                    .child_by_field_name("pattern")
                    .is_some_and(|pattern| subtree_has_string(pattern))
        })
        .count() as u32
}

fn switch_string_cases(language: SourceLanguage, node: Node<'_>) -> u32 {
    let discriminant = node.child_by_field_name("value").map(|value| {
        if value.kind() == "parenthesized_expression" {
            value.named_child(0).unwrap_or(value)
        } else {
            value
        }
    });
    if discriminant.map(|value| value.kind()) != Some("identifier") {
        return 0;
    }
    let mut count = 0;
    let mut cursor = node.walk();
    let cases: Vec<Node<'_>> = match language {
        SourceLanguage::Go => node.named_children(&mut cursor).collect(),
        _ => node
            .child_by_field_name("body")
            .map(|body| {
                let mut inner = body.walk();
                body.named_children(&mut inner).collect()
            })
            .unwrap_or_default(),
    };
    for case in cases {
        match language {
            SourceLanguage::Go
                if case.kind() == "expression_case"
                    && case
                        .child_by_field_name("value")
                        .is_some_and(subtree_has_string) =>
            {
                count += 1;
            }
            SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx
                if case.kind() == "switch_case"
                    && case
                        .child_by_field_name("value")
                        .is_some_and(|value| is_string_literal(value.kind())) =>
            {
                count += 1;
            }
            _ => {}
        }
    }
    count
}

fn subtree_has_string(node: Node<'_>) -> bool {
    if is_string_literal(node.kind()) {
        return true;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| subtree_has_string(child))
}

fn block_is_empty_or_pass(block: Node<'_>) -> bool {
    let mut cursor = block.walk();
    block
        .named_children(&mut cursor)
        .all(|child| matches!(child.kind(), "pass_statement" | "comment"))
}

fn block_returns(block: Node<'_>) -> bool {
    // Go wraps a block body in a `statement_list`; recurse into the block but do
    // not descend into nested function literals.
    if block.kind() == "func_literal" {
        return false;
    }
    if block.kind() == "return_statement" {
        return true;
    }
    let mut cursor = block.walk();
    block
        .named_children(&mut cursor)
        .any(block_returns)
}

fn is_go_err_check(file: &SourceFile, condition: Node<'_>) -> bool {
    if condition.kind() != "binary_expression" {
        return false;
    }
    let mut cursor = condition.walk();
    let mut has_neq = false;
    let mut has_nil = false;
    let mut has_identifier = false;
    for child in condition.children(&mut cursor) {
        if child.is_named() {
            match child.kind() {
                "nil" => has_nil = true,
                "identifier" => has_identifier = true,
                _ => {}
            }
        } else if text(file, child) == "!=" {
            has_neq = true;
        }
    }
    has_neq && has_nil && has_identifier
}

// ---------------------------------------------------------------------------
// File-level structural proxies (not attributed to a single function space).
// ---------------------------------------------------------------------------

fn count_magic_literals(file: &SourceFile, root: Node<'_>) -> (u64, u64) {
    fn is_excluded_decl(language: SourceLanguage, kind: &str) -> bool {
        match language {
            SourceLanguage::Rust => {
                matches!(kind, "const_item" | "static_item" | "enum_item" | "type_item")
            }
            SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
                matches!(kind, "enum_declaration" | "type_alias_declaration")
            }
            SourceLanguage::Go => matches!(kind, "const_declaration" | "type_declaration"),
            SourceLanguage::Python => false,
        }
    }
    fn walk(file: &SourceFile, node: Node<'_>, excluded: bool, count: &mut (u64, u64)) {
        let excluded = excluded || is_excluded_decl(file.language, node.kind());
        // JS/TS `const` declarations are lexical_declaration with a `const` keyword.
        let excluded = excluded
            || (matches!(
                file.language,
                SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx
            ) && node.kind() == "lexical_declaration"
                && node
                    .child(0)
                    .is_some_and(|keyword| text(file, keyword) == "const"));
        if !excluded {
            match magic_literal_kind(file, node) {
                Some(MagicLiteral::Number) => count.0 += 1,
                Some(MagicLiteral::String) => count.1 += 1,
                None => {}
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(file, child, excluded, count);
        }
    }
    let mut count = (0, 0);
    walk(file, root, false, &mut count);
    count
}

enum MagicLiteral {
    Number,
    String,
}

fn magic_literal_kind(file: &SourceFile, node: Node<'_>) -> Option<MagicLiteral> {
    let kind = node.kind();
    if matches!(kind, "integer_literal" | "float_literal" | "integer" | "float" | "int_literal") {
        let raw = text(file, node).replace('_', "");
        let cleaned = raw.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        let Ok(mut value) = cleaned.parse::<f64>() else {
            return Some(MagicLiteral::Number);
        };
        if node
            .parent()
            .is_some_and(|parent| is_negation(file, parent))
        {
            value = -value;
        }
        (!(value == 0.0 || value == 1.0 || value == -1.0 || value == 2.0)).then_some(MagicLiteral::Number)
    } else if is_string_literal(kind) {
        // A Python docstring is a bare string expression statement.
        let docstring = file.language == SourceLanguage::Python
            && node
                .parent()
                .is_some_and(|parent| parent.kind() == "expression_statement");
        (!docstring && string_content_length(file, node) >= 2).then_some(MagicLiteral::String)
    } else {
        None
    }
}

fn is_negation(file: &SourceFile, parent: Node<'_>) -> bool {
    matches!(parent.kind(), "unary_expression" | "prefix_expression" | "negated_expression")
        && parent
            .child(0)
            .is_some_and(|operator| text(file, operator) == "-")
}

fn string_content_length(file: &SourceFile, node: Node<'_>) -> usize {
    // Interior text length: prefer string_content children, else strip quotes.
    let mut cursor = node.walk();
    let content: String = node
        .named_children(&mut cursor)
        .filter(|child| child.kind() == "string_content")
        .map(|child| text(file, child))
        .collect();
    if !content.is_empty() {
        return content.chars().count();
    }
    let raw = text(file, node);
    raw.trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .chars()
        .count()
}

fn count_global_mutable_state(file: &SourceFile, root: Node<'_>) -> u64 {
    match file.language {
        SourceLanguage::Rust => count_rust_static_mut(root),
        SourceLanguage::Go => count_go_package_vars(root),
        SourceLanguage::JavaScript | SourceLanguage::TypeScript | SourceLanguage::Tsx => {
            count_js_module_mutable(file, root)
        }
        SourceLanguage::Python => count_python_module_mutable(file, root),
    }
}

fn count_rust_static_mut(root: Node<'_>) -> u64 {
    fn walk(node: Node<'_>, count: &mut u64) {
        if node.kind() == "static_item" && has_child_kind(node, "mutable_specifier") {
            *count += 1;
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            walk(child, count);
        }
    }
    let mut count = 0;
    walk(root, &mut count);
    count
}

fn count_go_package_vars(root: Node<'_>) -> u64 {
    let mut count = 0;
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if child.kind() == "var_declaration" {
            let mut inner = child.walk();
            for spec in child.named_children(&mut inner) {
                if spec.kind() == "var_spec" {
                    let mut names = spec.walk();
                    count += spec.children_by_field_name("name", &mut names).count() as u64;
                }
            }
        }
    }
    count
}

fn count_js_module_mutable(file: &SourceFile, root: Node<'_>) -> u64 {
    // Top-level `let`/`var` names reassigned anywhere in the file.
    let mut declared = Vec::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        let node = if child.kind() == "export_statement" {
            child.named_child(0).unwrap_or(child)
        } else {
            child
        };
        if matches!(node.kind(), "lexical_declaration" | "variable_declaration") {
            let mutable = node
                .child(0)
                .is_some_and(|keyword| matches!(text(file, keyword), "let" | "var"));
            if mutable {
                let mut inner = node.walk();
                for declarator in node.named_children(&mut inner) {
                    if declarator.kind() == "variable_declarator"
                        && let Some(name) = declarator.child_by_field_name("name")
                    {
                        collect_pattern_identifiers(file, name, &mut declared);
                    }
                }
            }
        }
    }
    let reassigned = collect_reassigned_names(file, root);
    declared
        .into_iter()
        .filter(|name| reassigned.contains(name))
        .count() as u64
}

fn collect_reassigned_names(file: &SourceFile, root: Node<'_>) -> BTreeSet<String> {
    fn walk(file: &SourceFile, node: Node<'_>, out: &mut BTreeSet<String>) {
        if matches!(
            node.kind(),
            "assignment_expression" | "augmented_assignment_expression"
        ) && let Some(left) = node.child_by_field_name("left")
            && let Some((root, false)) = assign_target_root(file, left)
        {
            out.insert(root);
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            walk(file, child, out);
        }
    }
    let mut out = BTreeSet::new();
    walk(file, root, &mut out);
    out
}

fn count_python_module_mutable(file: &SourceFile, root: Node<'_>) -> u64 {
    // Module-level names assigned more than once at module level, plus any name
    // declared `global` inside a function.
    let mut module_assignments: BTreeMap<String, u32> = BTreeMap::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if child.kind() == "expression_statement"
            && let Some(assignment) = child.named_child(0)
            && matches!(assignment.kind(), "assignment" | "augmented_assignment")
            && let Some(left) = assignment.child_by_field_name("left")
            && let Some((name, false)) = assign_target_root(file, left)
        {
            *module_assignments.entry(name).or_default() += 1;
        }
    }
    let mut globals = BTreeSet::new();
    collect_python_globals(file, root, &mut globals);
    let mut names: BTreeSet<String> = globals;
    for (name, count) in module_assignments {
        if count > 1 {
            names.insert(name);
        }
    }
    names.len() as u64
}

fn collect_python_globals(file: &SourceFile, node: Node<'_>, out: &mut BTreeSet<String>) {
    if node.kind() == "global_statement" {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "identifier" {
                out.insert(text(file, child).to_owned());
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_python_globals(file, child, out);
    }
}
