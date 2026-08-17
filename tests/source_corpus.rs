use std::fs;
use std::path::Path;

use software_evaluation::discipline::analyze_discipline;
use software_evaluation::duplicates::{DuplicateConfig, analyze_duplicates};
use software_evaluation::shape::analyze_shape;
use software_evaluation::source::SourceCorpusSession;
use software_evaluation::symbols::analyze_symbols;
use tempfile::TempDir;

fn write_file(root: &Path, relative: &str, source: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create corpus fixture parent");
    }
    fs::write(path, source).expect("write corpus fixture");
}

#[test]
fn shared_corpus_preserves_every_analyzer_observation_byte_for_byte() {
    let directory = TempDir::new().expect("temporary analyzer corpus");
    let root = directory.path();
    write_file(
        root,
        "src/lib.rs",
        r#"
pub fn choose(flag: bool) -> usize {
    if flag { helper(1) } else { helper(2) }
}

fn helper(value: usize) -> usize {
    value + 1
}
"#,
    );
    write_file(
        root,
        "src/module.py",
        r#"
def choose(flag: bool) -> int:
    value = 1
    if flag:
        value = 2
    return value
"#,
    );
    write_file(
        root,
        "src/module.ts",
        r#"
export function choose(flag: boolean): number {
  let value = 1;
  if (flag) value = 2;
  return value;
}
"#,
    );
    write_file(
        root,
        "src/module.go",
        r#"
package fixture

func choose(flag bool) int {
    value := 1
    if flag { value = 2 }
    return value
}
"#,
    );
    write_file(root, "README.md", "unsupported fixture\n");

    let duplicate_config = DuplicateConfig::default();
    let uncached_shape = serde_json::to_vec(&analyze_shape(root).expect("uncached shape"))
        .expect("serialize uncached shape");
    let uncached_symbols = serde_json::to_vec(&analyze_symbols(root).expect("uncached symbols"))
        .expect("serialize uncached symbols");
    let uncached_discipline =
        serde_json::to_vec(&analyze_discipline(root).expect("uncached discipline"))
            .expect("serialize uncached discipline");
    let uncached_duplicates = serde_json::to_vec(
        &analyze_duplicates(root, &duplicate_config).expect("uncached duplicates"),
    )
    .expect("serialize uncached duplicates");

    let corpus = SourceCorpusSession::activate(root).expect("materialize shared source corpus");
    let (shape, symbols, discipline, duplicates) = std::thread::scope(|scope| {
        let shape = scope.spawn(|| corpus.scope(|| analyze_shape(root)));
        let symbols = scope.spawn(|| corpus.scope(|| analyze_symbols(root)));
        let discipline = scope.spawn(|| corpus.scope(|| analyze_discipline(root)));
        let duplicates = scope.spawn(|| {
            corpus.scope(|| analyze_duplicates(root, &duplicate_config))
        });
        (
            shape.join().expect("shape thread").expect("cached shape"),
            symbols
                .join()
                .expect("symbols thread")
                .expect("cached symbols"),
            discipline
                .join()
                .expect("discipline thread")
                .expect("cached discipline"),
            duplicates
                .join()
                .expect("duplicates thread")
                .expect("cached duplicates"),
        )
    });

    assert_eq!(
        serde_json::to_vec(&shape).expect("serialize cached shape"),
        uncached_shape
    );
    assert_eq!(
        serde_json::to_vec(&symbols).expect("serialize cached symbols"),
        uncached_symbols
    );
    assert_eq!(
        serde_json::to_vec(&discipline).expect("serialize cached discipline"),
        uncached_discipline
    );
    assert_eq!(
        serde_json::to_vec(&duplicates).expect("serialize cached duplicates"),
        uncached_duplicates
    );

    let receipt = corpus.receipt();
    let stats = corpus.cache_stats();
    assert_eq!(receipt.supported_files, 4);
    assert_eq!(receipt.file_reads, 4);
    assert_eq!(receipt.parses, 4);
    assert_eq!(stats.source_tree_hits, 4);
    assert_eq!(stats.parse_tree_hits, 13);
}
