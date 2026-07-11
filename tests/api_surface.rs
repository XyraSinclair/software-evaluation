use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use software_evaluation::api_surface::{ApiSymbolKind, analyze_api_surface};
use tempfile::TempDir;

struct Fixture {
    directory: TempDir,
    sources: Vec<(&'static str, &'static str)>,
}

impl Fixture {
    fn new() -> Self {
        let sources = vec![
            (
                "src/api.rs",
                r#"/// Adjacent even across an attribute.
#[inline]
pub fn rust_public<T, U>(left: T, right: U) -> T { left }
pub(crate) fn rust_restricted() {}
fn rust_private() {}

/// Detached documentation.

pub const RUST_DETACHED: usize = 1;

pub struct RustRecord<T> {
    /// Public field documentation.
    pub exposed: T,
    private: T,
}

impl<T> RustRecord<T> {
    pub fn method(&self, value: T) -> T { value }
    fn private_method(&self) {}
}

pub use crate::internal::Thing as RustAlias;
"#,
            ),
            (
                "src/module.py",
                r#"# Adjacent through a decorator.
@decorator
def python_public(first, second=1):
    return first

def _python_private(value):
    return value

PublicValue = 1
PUBLIC_LIMIT = 2
_private_value = 3

class PublicType:
    def member(self, value):
        return value

    def _private_member(self):
        return None

# Detached Python documentation.

def python_detached():
    return None

class _PrivateType:
    pass
"#,
            ),
            (
                "src/module.js",
                r#"/** Adjacent JavaScript documentation. */
export function jsPublic(first, second) { return first + second; }
function jsPrivate() {}
export default function (value) { return value; }
export const jsArrow = (left, right) => left + right;
export class JsWidget {
  /** Adjacent member documentation. */
  member(value) { return value; }
  visibleField = 1;
  #secret(value) { return value; }
}
const local = 1;
export { local as jsAlias };
export * from "./remote.js";
/** Detached documentation. */

export const jsDetached = 1;
"#,
            ),
            (
                "src/types.ts",
                r#"/** Generic interface documentation. */
export interface Contract<T, U> {
  run<V>(value: V): T;
  value: U;
}
export class TsClass<T> {
  public open(value: T): T { return value; }
  defaultMethod(): void {}
  visible = 1;
  protected restricted(): void {}
  private hidden = 2;
}
/** Detached TypeScript documentation. */

export function tsDetached(): void {}
class TsPrivate {}
export type { External as ExternalType } from "./external";
export const tsConstant: number = 1;
"#,
            ),
            (
                "src/view.tsx",
                r#"/** TSX generic component documentation. */
export function Panel<T>(props: { value: T }) {
  return <span>{String(props.value)}</span>;
}
/** Detached TSX documentation. */

export const TsxDetached = 1;
function HiddenPanel() { return <span />; }
"#,
            ),
            (
                "src/public.go",
                r#"package fixture

// Exported has adjacent Go documentation.
func Exported[T any](first T, count int) T { return first }
func unexported(value int) int { return value }

// PublicType is documented.
type PublicType[T any] struct {
	Visible T
	hidden T
}

func (item PublicType[T]) Method(value T) T { return value }
func (item PublicType[T]) privateMethod() {}

// Detached Go documentation.

func GoDetached() {}
func Éclair() {}
func éclair() {}

const ExportedConstant = 1
const privateConstant = 2
"#,
            ),
            ("src/z_syntax_error.rs", "fn broken( {\n"),
        ];
        let directory = TempDir::new().expect("temporary API-surface repository");
        for (relative, source) in &sources {
            write_file(directory.path(), relative, source);
        }
        Self { directory, sources }
    }

    fn path(&self) -> &Path {
        self.directory.path()
    }

    fn expected_source_lines(&self) -> usize {
        self.sources
            .iter()
            .map(|(_, source)| {
                source
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .count()
            })
            .sum()
    }
}

fn write_file(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create API fixture parent");
    }
    fs::write(&path, contents)
        .unwrap_or_else(|error| panic!("write API fixture {}: {error}", path.display()));
}

#[derive(Clone, Copy)]
struct ExpectedSymbol {
    path: &'static str,
    symbol: &'static str,
    kind: ApiSymbolKind,
    basis: &'static str,
    parameters: usize,
    generics: usize,
    documented: bool,
}

const EXPECTED: &[ExpectedSymbol] = &[
    ExpectedSymbol {
        path: "src/api.rs",
        symbol: "rust_public",
        kind: ApiSymbolKind::Function,
        basis: "Rust explicit visibility: pub",
        parameters: 2,
        generics: 2,
        documented: true,
    },
    ExpectedSymbol {
        path: "src/api.rs",
        symbol: "rust_restricted",
        kind: ApiSymbolKind::Function,
        basis: "Rust explicit visibility: pub(crate)",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/api.rs",
        symbol: "RUST_DETACHED",
        kind: ApiSymbolKind::Constant,
        basis: "Rust explicit visibility: pub",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/api.rs",
        symbol: "RustRecord",
        kind: ApiSymbolKind::Type,
        basis: "Rust explicit visibility: pub",
        parameters: 0,
        generics: 1,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/api.rs",
        symbol: "exposed",
        kind: ApiSymbolKind::Field,
        basis: "Rust explicit visibility: pub",
        parameters: 0,
        generics: 0,
        documented: true,
    },
    ExpectedSymbol {
        path: "src/api.rs",
        symbol: "method",
        kind: ApiSymbolKind::Method,
        basis: "Rust explicit visibility: pub",
        parameters: 2,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/api.rs",
        symbol: "RustAlias",
        kind: ApiSymbolKind::ReExport,
        basis: "Rust explicit visibility: pub; re-export syntax",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/module.js",
        symbol: "jsPublic",
        kind: ApiSymbolKind::Function,
        basis: "ECMAScript named export syntax",
        parameters: 2,
        generics: 0,
        documented: true,
    },
    ExpectedSymbol {
        path: "src/module.js",
        symbol: "default",
        kind: ApiSymbolKind::Function,
        basis: "ECMAScript export default syntax",
        parameters: 1,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/module.js",
        symbol: "jsArrow",
        kind: ApiSymbolKind::Function,
        basis: "ECMAScript named export syntax",
        parameters: 2,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/module.js",
        symbol: "JsWidget",
        kind: ApiSymbolKind::Type,
        basis: "ECMAScript named export syntax",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/module.js",
        symbol: "member",
        kind: ApiSymbolKind::Method,
        basis: "ECMAScript named export syntax; public/default member visibility",
        parameters: 1,
        generics: 0,
        documented: true,
    },
    ExpectedSymbol {
        path: "src/module.js",
        symbol: "visibleField",
        kind: ApiSymbolKind::Field,
        basis: "ECMAScript named export syntax; public/default member visibility",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/module.js",
        symbol: "jsAlias",
        kind: ApiSymbolKind::ReExport,
        basis: "ECMAScript named export syntax",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/module.js",
        symbol: "*",
        kind: ApiSymbolKind::ReExport,
        basis: "ECMAScript named export syntax",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/module.js",
        symbol: "jsDetached",
        kind: ApiSymbolKind::Constant,
        basis: "ECMAScript named export syntax",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/module.py",
        symbol: "python_public",
        kind: ApiSymbolKind::Function,
        basis: "Python module-level non-underscore convention proxy",
        parameters: 2,
        generics: 0,
        documented: true,
    },
    ExpectedSymbol {
        path: "src/module.py",
        symbol: "PublicValue",
        kind: ApiSymbolKind::Other,
        basis: "Python module-level non-underscore convention proxy",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/module.py",
        symbol: "PUBLIC_LIMIT",
        kind: ApiSymbolKind::Constant,
        basis: "Python module-level non-underscore convention proxy",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/module.py",
        symbol: "PublicType",
        kind: ApiSymbolKind::Type,
        basis: "Python module-level non-underscore convention proxy",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/module.py",
        symbol: "python_detached",
        kind: ApiSymbolKind::Function,
        basis: "Python module-level non-underscore convention proxy",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/public.go",
        symbol: "Exported",
        kind: ApiSymbolKind::Function,
        basis: "Go exported-name capitalization lexical proxy",
        parameters: 2,
        generics: 1,
        documented: true,
    },
    ExpectedSymbol {
        path: "src/public.go",
        symbol: "PublicType",
        kind: ApiSymbolKind::Type,
        basis: "Go exported-name capitalization lexical proxy",
        parameters: 0,
        generics: 1,
        documented: true,
    },
    ExpectedSymbol {
        path: "src/public.go",
        symbol: "Visible",
        kind: ApiSymbolKind::Field,
        basis: "Go exported-name capitalization lexical proxy",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/public.go",
        symbol: "Method",
        kind: ApiSymbolKind::Method,
        basis: "Go exported-name capitalization lexical proxy",
        parameters: 1,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/public.go",
        symbol: "ExportedConstant",
        kind: ApiSymbolKind::Constant,
        basis: "Go exported-name capitalization lexical proxy",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/public.go",
        symbol: "GoDetached",
        kind: ApiSymbolKind::Function,
        basis: "Go exported-name capitalization lexical proxy",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/public.go",
        symbol: "Éclair",
        kind: ApiSymbolKind::Function,
        basis: "Go exported-name capitalization lexical proxy",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/types.ts",
        symbol: "Contract",
        kind: ApiSymbolKind::Type,
        basis: "ECMAScript named export syntax",
        parameters: 0,
        generics: 2,
        documented: true,
    },
    ExpectedSymbol {
        path: "src/types.ts",
        symbol: "run",
        kind: ApiSymbolKind::Method,
        basis: "ECMAScript named export syntax; public/default member visibility",
        parameters: 1,
        generics: 1,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/types.ts",
        symbol: "value",
        kind: ApiSymbolKind::Field,
        basis: "ECMAScript named export syntax; public/default member visibility",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/types.ts",
        symbol: "TsClass",
        kind: ApiSymbolKind::Type,
        basis: "ECMAScript named export syntax",
        parameters: 0,
        generics: 1,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/types.ts",
        symbol: "open",
        kind: ApiSymbolKind::Method,
        basis: "ECMAScript named export syntax; public/default member visibility",
        parameters: 1,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/types.ts",
        symbol: "defaultMethod",
        kind: ApiSymbolKind::Method,
        basis: "ECMAScript named export syntax; public/default member visibility",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/types.ts",
        symbol: "visible",
        kind: ApiSymbolKind::Field,
        basis: "ECMAScript named export syntax; public/default member visibility",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/types.ts",
        symbol: "ExternalType",
        kind: ApiSymbolKind::ReExport,
        basis: "ECMAScript type-only export syntax",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/types.ts",
        symbol: "tsConstant",
        kind: ApiSymbolKind::Constant,
        basis: "ECMAScript named export syntax",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/types.ts",
        symbol: "tsDetached",
        kind: ApiSymbolKind::Function,
        basis: "ECMAScript named export syntax",
        parameters: 0,
        generics: 0,
        documented: false,
    },
    ExpectedSymbol {
        path: "src/view.tsx",
        symbol: "Panel",
        kind: ApiSymbolKind::Function,
        basis: "ECMAScript named export syntax",
        parameters: 1,
        generics: 1,
        documented: true,
    },
    ExpectedSymbol {
        path: "src/view.tsx",
        symbol: "TsxDetached",
        kind: ApiSymbolKind::Constant,
        basis: "ECMAScript named export syntax",
        parameters: 0,
        generics: 0,
        documented: false,
    },
];

const EXCLUDED: &[(&str, &str)] = &[
    ("src/api.rs", "rust_private"),
    ("src/api.rs", "private"),
    ("src/api.rs", "private_method"),
    ("src/module.py", "_python_private"),
    ("src/module.py", "_private_value"),
    ("src/module.py", "member"),
    ("src/module.py", "_private_member"),
    ("src/module.py", "_PrivateType"),
    ("src/module.js", "jsPrivate"),
    ("src/module.js", "#secret"),
    ("src/module.js", "local"),
    ("src/types.ts", "restricted"),
    ("src/types.ts", "hidden"),
    ("src/types.ts", "TsPrivate"),
    ("src/view.tsx", "HiddenPanel"),
    ("src/public.go", "unexported"),
    ("src/public.go", "hidden"),
    ("src/public.go", "privateMethod"),
    ("src/public.go", "privateConstant"),
    ("src/public.go", "éclair"),
];

fn run_cli(root: &Path, trailing: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seval"))
        .arg("api")
        .arg(root)
        .args(trailing)
        .output()
        .expect("run seval api")
}

fn assert_observation_root_has_no_verdict(value: &Value) {
    let root = value.as_object().expect("API JSON root must be an object");
    for forbidden in ["score", "quality_score", "verdict"] {
        assert!(
            !root.contains_key(forbidden),
            "API observations must not emit root judgment field {forbidden:?}"
        );
    }
}

#[test]
fn polyglot_surface_obeys_each_language_proxy_and_report_invariant() {
    let fixture = Fixture::new();

    // Independent oracle, committed before production runs: one row per expected
    // observable and density = that row count per nonblank supported-source kSLOC.
    let expected_source_lines = fixture.expected_source_lines();
    let expected_density = EXPECTED.len() as f64 * 1000.0 / expected_source_lines as f64;
    let expected_keys = EXPECTED
        .iter()
        .map(|row| (row.path, row.symbol, row.kind))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        expected_keys.len(),
        EXPECTED.len(),
        "oracle rows must be unique"
    );
    assert!(
        expected_source_lines > 0,
        "density oracle requires a positive denominator"
    );

    let report = analyze_api_surface(fixture.path()).expect("analyze polyglot API fixture");
    let repeated = analyze_api_surface(fixture.path()).expect("repeat API analysis");

    assert_eq!(report.coverage.source_files, fixture.sources.len());
    assert_eq!(report.coverage.parsed_files, fixture.sources.len());
    assert_eq!(report.coverage.syntax_error_files, 1);
    assert_eq!(report.coverage.source_lines, expected_source_lines);
    assert_eq!(
        report.coverage.density_denominator_source_lines,
        expected_source_lines
    );
    assert_eq!(report.counts.public_symbols, EXPECTED.len());
    assert!((report.counts.public_symbols_per_ksloc - expected_density).abs() < 1e-12);

    let actual_keys = report
        .symbols
        .iter()
        .map(|row| (row.path.as_str(), row.symbol.as_str(), row.kind))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual_keys, expected_keys,
        "symbol membership must match the proxy contract exactly"
    );

    for expected in EXPECTED {
        let matches = report
            .symbols
            .iter()
            .filter(|row| row.path == expected.path && row.symbol == expected.symbol)
            .collect::<Vec<_>>();
        assert_eq!(
            matches.len(),
            1,
            "expected exactly one row for {}:{}",
            expected.path,
            expected.symbol
        );
        let actual = matches[0];
        assert_eq!(
            actual.kind, expected.kind,
            "wrong kind for {}:{}",
            expected.path, expected.symbol
        );
        assert_eq!(
            actual.visibility_or_proxy_basis, expected.basis,
            "wrong visibility/proxy basis for {}:{}",
            expected.path, expected.symbol
        );
        assert_eq!(
            actual.parameter_count, expected.parameters,
            "wrong parameter count for {}:{}",
            expected.path, expected.symbol
        );
        assert_eq!(
            actual.generic_or_type_parameter_count, expected.generics,
            "wrong generic count for {}:{}",
            expected.path, expected.symbol
        );
        assert_eq!(
            actual.documentation_immediately_precedes, expected.documented,
            "wrong documentation adjacency for {}:{}",
            expected.path, expected.symbol
        );
    }
    for &(path, symbol) in EXCLUDED {
        assert!(
            !report
                .symbols
                .iter()
                .any(|row| row.path == path && row.symbol == symbol),
            "negative control {path}:{symbol} must be excluded"
        );
    }

    let order = report
        .symbols
        .iter()
        .map(|row| (&row.path, row.line, row.kind, &row.symbol))
        .collect::<Vec<_>>();
    let mut sorted_order = order.clone();
    sorted_order.sort();
    assert_eq!(
        order, sorted_order,
        "rows must use deterministic path/line/kind/symbol ordering"
    );
    assert_eq!(
        report.symbols.len(),
        actual_keys.len(),
        "rows must be deduplicated"
    );
    assert_eq!(
        report
            .symbols
            .iter()
            .map(|row| (
                &row.path,
                row.line,
                row.kind,
                &row.symbol,
                &row.visibility_or_proxy_basis
            ))
            .collect::<Vec<_>>(),
        repeated
            .symbols
            .iter()
            .map(|row| (
                &row.path,
                row.line,
                row.kind,
                &row.symbol,
                &row.visibility_or_proxy_basis
            ))
            .collect::<Vec<_>>(),
        "repeated analysis must preserve exact ordering and membership"
    );

    assert_eq!(
        report.counts.documented_symbols,
        EXPECTED.iter().filter(|row| row.documented).count()
    );
    assert_eq!(
        report.counts.total_parameters,
        EXPECTED.iter().map(|row| row.parameters).sum::<usize>()
    );
    assert_eq!(
        report.counts.total_generic_or_type_parameters,
        EXPECTED.iter().map(|row| row.generics).sum::<usize>()
    );
    let serialized = serde_json::to_value(&report).expect("serialize API report");
    assert_observation_root_has_no_verdict(&serialized);
}

#[test]
fn cli_json_preserves_full_observations_and_text_honors_top() {
    let fixture = Fixture::new();
    let expected_source_lines = fixture.expected_source_lines();
    let expected_public_symbols = EXPECTED.len();
    let expected_density = expected_public_symbols as f64 * 1000.0 / expected_source_lines as f64;

    let json_output = run_cli(fixture.path(), &["--top", "1", "--format", "json"]);
    assert!(
        json_output.status.success(),
        "JSON CLI failed: {}",
        String::from_utf8_lossy(&json_output.stderr)
    );
    let json: Value = serde_json::from_slice(&json_output.stdout).unwrap_or_else(|error| {
        panic!(
            "API CLI emitted invalid JSON: {error}; stdout={:?}",
            String::from_utf8_lossy(&json_output.stdout)
        )
    });
    assert_observation_root_has_no_verdict(&json);
    let symbols = json
        .get("symbols")
        .and_then(Value::as_array)
        .expect("JSON symbols array");
    assert_eq!(
        symbols.len(),
        expected_public_symbols,
        "--top limits text presentation, not JSON evidence"
    );
    assert_eq!(
        json.pointer("/coverage/syntax_error_files")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        json.pointer("/coverage/density_denominator_source_lines")
            .and_then(Value::as_u64),
        Some(expected_source_lines as u64)
    );
    let actual_density = json
        .pointer("/counts/public_symbols_per_ksloc")
        .and_then(Value::as_f64)
        .expect("JSON density");
    assert!((actual_density - expected_density).abs() < 1e-12);

    let text_output = run_cli(fixture.path(), &["--top", "1", "--format", "text"]);
    assert!(
        text_output.status.success(),
        "text CLI failed: {}",
        String::from_utf8_lossy(&text_output.stderr)
    );
    let text = String::from_utf8(text_output.stdout).expect("UTF-8 API text output");
    assert!(text.contains(&format!("symbols: 1 / {expected_public_symbols} shown")));
    assert!(text.contains("src/api.rs"));
    assert!(
        text.contains("rust_public"),
        "first deterministic row must be rendered"
    );
    assert!(
        !text.contains("rust_restricted"),
        "--top 1 must suppress later symbol rows"
    );
    assert!(text.contains("basis: Rust explicit visibility: pub"));
    assert!(text.contains("syntax-error-files=1"));
    assert!(text.contains(&format!("symbols/kSLOC={expected_density:.3}")));
}
