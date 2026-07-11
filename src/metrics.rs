use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use big_code_analysis::{Ast, LANG, Metric, MetricsOptions, Source, SpaceKind, get_from_ext};
use ignore::WalkBuilder;
use rayon::prelude::*;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("analysis input does not exist: {0}")]
    NotFound(PathBuf),
    #[error("analysis input is a symbolic link: {0}")]
    Symlink(PathBuf),
    #[error("failed to inspect {path}: {source}")]
    Metadata {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed while traversing {path}: {message}")]
    Traversal { path: PathBuf, message: String },
    #[error("path is not valid UTF-8: {0:?}")]
    NonUtf8Path(PathBuf),
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to analyze {path}: {message}")]
    Analyze { path: PathBuf, message: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsReport {
    pub root: String,
    pub analyzer: String,
    pub coverage: Coverage,
    pub summary: MetricSummary,
    pub rates: MetricRates,
    pub distributions: FunctionDistributions,
    pub languages: Vec<LanguageSummary>,
    pub files: Vec<FileMetric>,
    pub functions: Vec<FunctionMetric>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Coverage {
    pub enumerated: usize,
    pub analyzed: usize,
    pub skipped: usize,
    pub syntax_error_files: usize,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MetricSummary {
    pub files: usize,
    pub functions: usize,
    pub lines: usize,
    pub sloc: usize,
    pub ploc: usize,
    pub lloc: usize,
    pub cloc: usize,
    pub blank: usize,
    pub cognitive: f64,
    pub cyclomatic: f64,
    pub modified_cyclomatic: f64,
    pub arguments: usize,
    pub exits: usize,
    pub mean_maintainability: Option<f64>,
    pub mean_maintainability_sei: Option<f64>,
    pub mean_maintainability_visual_studio: Option<f64>,
    pub mean_halstead_volume: Option<f64>,
    pub mean_halstead_difficulty: Option<f64>,
    pub mean_halstead_effort: Option<f64>,
    pub abc_assignments: f64,
    pub abc_branches: f64,
    pub abc_conditions: f64,
    pub abc_magnitude: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LanguageSummary {
    pub language: String,
    pub files: usize,
    pub functions: usize,
    pub lines: usize,
    pub sloc: usize,
    pub ploc: usize,
    pub lloc: usize,
    pub cloc: usize,
    pub blank: usize,
    pub cognitive: f64,
    pub cyclomatic: f64,
    pub modified_cyclomatic: f64,
    pub arguments: usize,
    pub exits: usize,
    pub mean_maintainability: Option<f64>,
    pub mean_maintainability_sei: Option<f64>,
    pub mean_maintainability_visual_studio: Option<f64>,
    pub mean_halstead_volume: Option<f64>,
    pub mean_halstead_difficulty: Option<f64>,
    pub mean_halstead_effort: Option<f64>,
    pub abc_assignments: f64,
    pub abc_branches: f64,
    pub abc_conditions: f64,
    pub abc_magnitude: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileMetric {
    pub path: String,
    pub language: String,
    pub functions: usize,
    pub lines: usize,
    pub sloc: usize,
    pub ploc: usize,
    pub lloc: usize,
    pub cloc: usize,
    pub blank: usize,
    pub cognitive: f64,
    pub cyclomatic: f64,
    pub modified_cyclomatic: f64,
    pub arguments: usize,
    pub exits: usize,
    pub maintainability: Option<f64>,
    pub maintainability_sei: Option<f64>,
    pub maintainability_visual_studio: Option<f64>,
    pub halstead_volume: Option<f64>,
    pub halstead_difficulty: Option<f64>,
    pub halstead_effort: Option<f64>,
    pub abc_assignments: f64,
    pub abc_branches: f64,
    pub abc_conditions: f64,
    pub abc_magnitude: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionMetric {
    pub path: String,
    pub language: String,
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub lines: usize,
    pub sloc: usize,
    pub ploc: usize,
    pub lloc: usize,
    pub cloc: usize,
    pub blank: usize,
    pub cognitive: f64,
    pub cyclomatic: f64,
    pub modified_cyclomatic: f64,
    pub arguments: usize,
    pub exits: usize,
    pub maintainability: Option<f64>,
    pub maintainability_sei: Option<f64>,
    pub maintainability_visual_studio: Option<f64>,
    pub halstead_volume: Option<f64>,
    pub halstead_difficulty: Option<f64>,
    pub halstead_effort: Option<f64>,
    pub abc_assignments: f64,
    pub abc_branches: f64,
    pub abc_conditions: f64,
    pub abc_magnitude: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MetricRates {
    pub functions_per_ksloc: Option<f64>,
    pub cognitive_per_ksloc: Option<f64>,
    pub cyclomatic_per_ksloc: Option<f64>,
    pub arguments_per_function: Option<f64>,
    pub exits_per_function: Option<f64>,
    pub comment_fraction: Option<f64>,
    pub blank_fraction: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Distribution {
    pub count: usize,
    pub min: Option<f64>,
    pub p50: Option<f64>,
    pub p90: Option<f64>,
    pub p99: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct FunctionDistributions {
    pub cognitive: Distribution,
    pub cyclomatic: Distribution,
    pub sloc: Distribution,
    pub arguments: Distribution,
    pub exits: Distribution,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsComparison {
    pub left: MetricsComparisonSide,
    pub right: MetricsComparisonSide,
    pub differences: Vec<NumericDifference>,
    pub matched_files: Vec<MatchedFileDifference>,
    pub only_left: Vec<FileIdentity>,
    pub only_right: Vec<FileIdentity>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsComparisonSide {
    pub root: String,
    pub analyzer: String,
    pub coverage: Coverage,
    pub summary: MetricSummary,
    pub rates: MetricRates,
    pub distributions: FunctionDistributions,
}

#[derive(Debug, Clone, Serialize)]
pub struct NumericDifference {
    pub metric: String,
    pub left: f64,
    pub right: f64,
    pub right_minus_left: f64,
    pub relative_change_from_left: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileIdentity {
    pub path: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileCoreMetrics {
    pub sloc: usize,
    pub cognitive: f64,
    pub cyclomatic: f64,
    pub arguments: usize,
    pub exits: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileCoreDelta {
    pub sloc: i128,
    pub cognitive: f64,
    pub cyclomatic: f64,
    pub arguments: i128,
    pub exits: i128,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchedFileDifference {
    pub path: String,
    pub language: String,
    pub left: FileCoreMetrics,
    pub right: FileCoreMetrics,
    pub right_minus_left: FileCoreDelta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricSort {
    Cognitive,
    Cyclomatic,
    Sloc,
    Arguments,
    Exits,
    Maintainability,
    HalsteadEffort,
}

pub fn analyze_path(input: &Path) -> Result<MetricsReport, MetricsError> {
    let started = Instant::now();
    let metadata = fs::symlink_metadata(input).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            MetricsError::NotFound(input.to_owned())
        } else {
            MetricsError::Metadata {
                path: input.to_owned(),
                source,
            }
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(MetricsError::Symlink(input.to_owned()));
    }

    let (root, candidates) = if metadata.is_file() {
        let root = input.parent().unwrap_or_else(|| Path::new(".")).to_owned();
        (root, vec![input.to_owned()])
    } else if metadata.is_dir() {
        let mut paths = Vec::new();
        let mut walker = WalkBuilder::new(input);
        walker
            .follow_links(false)
            .standard_filters(true)
            .require_git(false);
        for entry in walker.build() {
            let entry = entry.map_err(|error| MetricsError::Traversal {
                path: input.to_owned(),
                message: error.to_string(),
            })?;
            if entry.file_type().is_some_and(|kind| kind.is_file()) {
                paths.push(entry.into_path());
            }
        }
        (input.to_owned(), paths)
    } else {
        return Err(MetricsError::Traversal {
            path: input.to_owned(),
            message: "input is neither a regular file nor a directory".to_owned(),
        });
    };

    let mut candidates = candidates;
    candidates.sort_by_key(|path| normalized_relative(&root, path));
    let enumerated = candidates.len();
    let mut skipped = 0;
    let supported = candidates
        .into_iter()
        .filter_map(|path| match language_for_path(&path) {
            Some(language) => Some((path, language)),
            None => {
                skipped += 1;
                None
            }
        })
        .collect::<Vec<_>>();

    let analyzed = supported
        .par_iter()
        .map(|(path, language)| {
            let relative = relative_path(&root, path)?;
            let bytes = fs::read(path).map_err(|source| MetricsError::Read {
                path: path.clone(),
                source,
            })?;
            let source = Source::from_bytes(*language, bytes).with_name(Some(relative.clone()));
            let options = MetricsOptions::default().with_only(&[
                Metric::Abc,
                Metric::Cognitive,
                Metric::Mi,
                Metric::Nargs,
                Metric::Nexits,
            ]);
            let ast = Ast::parse(source).map_err(|error| MetricsError::Analyze {
                path: path.clone(),
                message: error.to_string(),
            })?;
            let has_syntax_errors = ast.as_tree_sitter().root_node().has_error();
            let space = ast
                .metrics(options)
                .map_err(|error| MetricsError::Analyze {
                    path: path.clone(),
                    message: error.to_string(),
                })?;
            let wire = space.to_wire();
            let language_name = if *language == LANG::Tsx {
                "typescript"
            } else {
                language.name()
            }
            .to_owned();
            let mut functions = Vec::new();
            collect_functions(&wire.spaces, &relative, &language_name, &mut functions);
            Ok((
                file_metric(&wire, relative, language_name),
                functions,
                has_syntax_errors,
            ))
        })
        .collect::<Vec<Result<(FileMetric, Vec<FunctionMetric>, bool), MetricsError>>>();

    let mut files = Vec::with_capacity(analyzed.len());
    let mut functions = Vec::new();
    let mut syntax_error_files = 0;
    for result in analyzed {
        let (file, mut file_functions, has_syntax_errors) = result?;
        files.push(file);
        functions.append(&mut file_functions);
        syntax_error_files += usize::from(has_syntax_errors);
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    functions.sort_by(function_tie);
    let summary = summarize_files(&files);
    let languages = summarize_languages(&files);
    let rates = metric_rates(&summary);
    let distributions = function_distributions(&functions);
    Ok(MetricsReport {
        root: normalized_path(input)?,
        analyzer: "big-code-analysis 2.0.0".to_owned(),
        coverage: Coverage {
            enumerated,
            analyzed: files.len(),
            skipped,
            syntax_error_files,
            elapsed_ms: started.elapsed().as_millis(),
        },
        summary,
        rates,
        distributions,
        languages,
        files,
        functions,
        limitations: vec![
            "AST metrics describe source structure; they do not measure runtime behavior, correctness, or quality.".to_owned(),
            "Only Rust, Python, JavaScript, TypeScript, and Go files with recognized extensions are analyzed.".to_owned(),
        ],
    })
}

fn language_for_path(path: &Path) -> Option<LANG> {
    let ext = path.extension()?.to_str()?;
    let lang = get_from_ext(ext)?;
    matches!(
        lang,
        LANG::Rust | LANG::Python | LANG::Javascript | LANG::Typescript | LANG::Tsx | LANG::Go
    )
    .then_some(lang)
}

fn relative_path(root: &Path, path: &Path) -> Result<String, MetricsError> {
    let relative = path.strip_prefix(root).unwrap_or(path);
    normalized_path(relative)
}

fn normalized_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn normalized_path(path: &Path) -> Result<String, MetricsError> {
    let value = path
        .to_str()
        .ok_or_else(|| MetricsError::NonUtf8Path(path.to_owned()))?;
    let normalized = path
        .components()
        .filter_map(|component| match component {
            Component::CurDir => None,
            Component::Normal(part) => part.to_str().map(str::to_owned),
            Component::ParentDir => Some("..".to_owned()),
            Component::RootDir => Some(String::new()),
            Component::Prefix(prefix) => Some(prefix.as_os_str().to_string_lossy().into_owned()),
        })
        .collect::<Vec<_>>()
        .join("/");
    if normalized.is_empty() {
        Ok(if value == "." || value.is_empty() {
            ".".to_owned()
        } else {
            "/".to_owned()
        })
    } else {
        Ok(normalized)
    }
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn file_metric(
    space: &big_code_analysis::wire::FuncSpace,
    path: String,
    language: String,
) -> FileMetric {
    let metrics = &space.metrics;
    let loc = metrics.loc.as_ref().expect("default metrics include LOC");
    let cognitive = metrics
        .cognitive
        .as_ref()
        .expect("default metrics include cognitive");
    let cyclomatic = metrics
        .cyclomatic
        .as_ref()
        .expect("default metrics include cyclomatic");
    let nargs = metrics
        .nargs
        .as_ref()
        .expect("default metrics include nargs");
    let nexits = metrics
        .nexits
        .as_ref()
        .expect("default metrics include nexits");
    let mi = metrics.mi.as_ref().expect("default metrics include MI");
    let halstead = metrics
        .halstead
        .as_ref()
        .expect("default metrics include Halstead");
    let abc = metrics.abc.as_ref().expect("default metrics include ABC");
    FileMetric {
        path,
        language,
        functions: count_functions(&space.spaces),
        lines: space
            .end_line
            .saturating_sub(space.start_line)
            .saturating_add(1),
        sloc: loc.sloc as usize,
        ploc: loc.ploc as usize,
        lloc: loc.lloc as usize,
        cloc: loc.cloc as usize,
        blank: loc.blank as usize,
        cognitive: cognitive.sum as f64,
        cyclomatic: cyclomatic.sum as f64,
        modified_cyclomatic: cyclomatic.modified.sum as f64,
        arguments: nargs.total as usize,
        exits: nexits.sum as usize,
        maintainability: finite(mi.original),
        maintainability_sei: finite(mi.sei),
        maintainability_visual_studio: finite(mi.visual_studio),
        halstead_volume: finite(halstead.volume),
        halstead_difficulty: finite(halstead.difficulty),
        halstead_effort: finite(halstead.effort),
        abc_assignments: abc.assignments as f64,
        abc_branches: abc.branches as f64,
        abc_conditions: abc.conditions as f64,
        abc_magnitude: abc.magnitude,
    }
}

fn collect_functions(
    spaces: &[big_code_analysis::wire::FuncSpace],
    path: &str,
    language: &str,
    out: &mut Vec<FunctionMetric>,
) {
    for space in spaces {
        if space.kind == SpaceKind::Function {
            let metrics = &space.metrics;
            let loc = metrics.loc.as_ref().expect("default metrics include LOC");
            let cognitive = metrics
                .cognitive
                .as_ref()
                .expect("default metrics include cognitive");
            let cyclomatic = metrics
                .cyclomatic
                .as_ref()
                .expect("default metrics include cyclomatic");
            let nargs = metrics
                .nargs
                .as_ref()
                .expect("default metrics include nargs");
            let nexits = metrics
                .nexits
                .as_ref()
                .expect("default metrics include nexits");
            let mi = metrics.mi.as_ref().expect("default metrics include MI");
            let halstead = metrics
                .halstead
                .as_ref()
                .expect("default metrics include Halstead");
            let abc = metrics.abc.as_ref().expect("default metrics include ABC");
            out.push(FunctionMetric {
                path: path.to_owned(),
                language: language.to_owned(),
                name: space
                    .name
                    .clone()
                    .unwrap_or_else(|| "<anonymous>".to_owned()),
                start_line: space.start_line,
                end_line: space.end_line,
                lines: space
                    .end_line
                    .saturating_sub(space.start_line)
                    .saturating_add(1),
                sloc: loc.sloc as usize,
                ploc: loc.ploc as usize,
                lloc: loc.lloc as usize,
                cloc: loc.cloc as usize,
                blank: loc.blank as usize,
                cognitive: cognitive.value as f64,
                cyclomatic: cyclomatic.value as f64,
                modified_cyclomatic: cyclomatic.modified.value as f64,
                arguments: nargs.total as usize,
                exits: nexits.sum as usize,
                maintainability: finite(mi.original),
                maintainability_sei: finite(mi.sei),
                maintainability_visual_studio: finite(mi.visual_studio),
                halstead_volume: finite(halstead.volume),
                halstead_difficulty: finite(halstead.difficulty),
                halstead_effort: finite(halstead.effort),
                abc_assignments: abc.assignments as f64,
                abc_branches: abc.branches as f64,
                abc_conditions: abc.conditions as f64,
                abc_magnitude: abc.value,
            });
        }
        collect_functions(&space.spaces, path, language, out);
    }
}

fn count_functions(spaces: &[big_code_analysis::wire::FuncSpace]) -> usize {
    spaces
        .iter()
        .map(|space| {
            usize::from(space.kind == SpaceKind::Function) + count_functions(&space.spaces)
        })
        .sum()
}

fn summarize_files(files: &[FileMetric]) -> MetricSummary {
    let mut summary = MetricSummary::default();
    for file in files {
        add_file(&mut summary, file);
    }
    average_options(&mut summary, files.len());
    summary
}

fn add_file(s: &mut MetricSummary, f: &FileMetric) {
    s.files += 1;
    s.functions += f.functions;
    s.lines += f.lines;
    s.sloc += f.sloc;
    s.ploc += f.ploc;
    s.lloc += f.lloc;
    s.cloc += f.cloc;
    s.blank += f.blank;
    s.cognitive += f.cognitive;
    s.cyclomatic += f.cyclomatic;
    s.modified_cyclomatic += f.modified_cyclomatic;
    s.arguments += f.arguments;
    s.exits += f.exits;
    add_option(&mut s.mean_maintainability, f.maintainability);
    add_option(&mut s.mean_maintainability_sei, f.maintainability_sei);
    add_option(
        &mut s.mean_maintainability_visual_studio,
        f.maintainability_visual_studio,
    );
    add_option(&mut s.mean_halstead_volume, f.halstead_volume);
    add_option(&mut s.mean_halstead_difficulty, f.halstead_difficulty);
    add_option(&mut s.mean_halstead_effort, f.halstead_effort);
    s.abc_assignments += f.abc_assignments;
    s.abc_branches += f.abc_branches;
    s.abc_conditions += f.abc_conditions;
    s.abc_magnitude =
        (s.abc_assignments.powi(2) + s.abc_branches.powi(2) + s.abc_conditions.powi(2)).sqrt();
}

fn add_option(total: &mut Option<f64>, value: Option<f64>) {
    if let Some(value) = value {
        *total = Some(total.unwrap_or(0.0) + value);
    }
}
fn average(value: &mut Option<f64>, count: usize) {
    if let Some(value) = value {
        *value /= count as f64;
    }
}
fn average_options(s: &mut MetricSummary, count: usize) {
    if count == 0 {
        return;
    }
    average(&mut s.mean_maintainability, count);
    average(&mut s.mean_maintainability_sei, count);
    average(&mut s.mean_maintainability_visual_studio, count);
    average(&mut s.mean_halstead_volume, count);
    average(&mut s.mean_halstead_difficulty, count);
    average(&mut s.mean_halstead_effort, count);
}

fn summarize_languages(files: &[FileMetric]) -> Vec<LanguageSummary> {
    let mut groups: BTreeMap<String, Vec<&FileMetric>> = BTreeMap::new();
    for file in files {
        groups.entry(file.language.clone()).or_default().push(file);
    }
    groups
        .into_iter()
        .map(|(language, group)| {
            let owned: Vec<FileMetric> = group.into_iter().cloned().collect();
            let s = summarize_files(&owned);
            LanguageSummary {
                language,
                files: s.files,
                functions: s.functions,
                lines: s.lines,
                sloc: s.sloc,
                ploc: s.ploc,
                lloc: s.lloc,
                cloc: s.cloc,
                blank: s.blank,
                cognitive: s.cognitive,
                cyclomatic: s.cyclomatic,
                modified_cyclomatic: s.modified_cyclomatic,
                arguments: s.arguments,
                exits: s.exits,
                mean_maintainability: s.mean_maintainability,
                mean_maintainability_sei: s.mean_maintainability_sei,
                mean_maintainability_visual_studio: s.mean_maintainability_visual_studio,
                mean_halstead_volume: s.mean_halstead_volume,
                mean_halstead_difficulty: s.mean_halstead_difficulty,
                mean_halstead_effort: s.mean_halstead_effort,
                abc_assignments: s.abc_assignments,
                abc_branches: s.abc_branches,
                abc_conditions: s.abc_conditions,
                abc_magnitude: s.abc_magnitude,
            }
        })
        .collect()
}

fn metric_rates(summary: &MetricSummary) -> MetricRates {
    MetricRates {
        functions_per_ksloc: per_ksloc(summary.functions as f64, summary.sloc),
        cognitive_per_ksloc: per_ksloc(summary.cognitive, summary.sloc),
        cyclomatic_per_ksloc: per_ksloc(summary.cyclomatic, summary.sloc),
        arguments_per_function: ratio(summary.arguments as f64, summary.functions),
        exits_per_function: ratio(summary.exits as f64, summary.functions),
        comment_fraction: ratio(summary.cloc as f64, summary.lines),
        blank_fraction: ratio(summary.blank as f64, summary.lines),
    }
}

fn per_ksloc(value: f64, sloc: usize) -> Option<f64> {
    ratio(value * 1_000.0, sloc)
}

fn ratio(numerator: f64, denominator: usize) -> Option<f64> {
    (denominator != 0).then_some(numerator / denominator as f64)
}

fn function_distributions(functions: &[FunctionMetric]) -> FunctionDistributions {
    FunctionDistributions {
        cognitive: distribution(
            functions
                .iter()
                .map(|function| function.cognitive)
                .collect(),
        ),
        cyclomatic: distribution(
            functions
                .iter()
                .map(|function| function.cyclomatic)
                .collect(),
        ),
        sloc: distribution(
            functions
                .iter()
                .map(|function| function.sloc as f64)
                .collect(),
        ),
        arguments: distribution(
            functions
                .iter()
                .map(|function| function.arguments as f64)
                .collect(),
        ),
        exits: distribution(
            functions
                .iter()
                .map(|function| function.exits as f64)
                .collect(),
        ),
    }
}

fn distribution(mut values: Vec<f64>) -> Distribution {
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    if values.is_empty() {
        return Distribution::default();
    }
    let count = values.len();
    let sum = values.iter().sum::<f64>();
    Distribution {
        count,
        min: values.first().copied(),
        p50: nearest_rank(&values, 50),
        p90: nearest_rank(&values, 90),
        p99: nearest_rank(&values, 99),
        max: values.last().copied(),
        mean: Some(sum / count as f64),
    }
}

fn nearest_rank(values: &[f64], percentile: usize) -> Option<f64> {
    if values.is_empty() || percentile == 0 || percentile > 100 {
        return None;
    }
    let rank = percentile.saturating_mul(values.len()).div_ceil(100);
    values.get(rank.saturating_sub(1)).copied()
}

pub fn compare_paths(left: &Path, right: &Path) -> Result<MetricsComparison, MetricsError> {
    let (left, right) = rayon::join(|| analyze_path(left), || analyze_path(right));
    Ok(compare_reports(&left?, &right?))
}

pub fn compare_reports(left: &MetricsReport, right: &MetricsReport) -> MetricsComparison {
    let mut differences = Vec::new();
    push_difference(
        &mut differences,
        "files",
        left.summary.files as f64,
        right.summary.files as f64,
    );
    push_difference(
        &mut differences,
        "functions",
        left.summary.functions as f64,
        right.summary.functions as f64,
    );
    push_difference(
        &mut differences,
        "sloc",
        left.summary.sloc as f64,
        right.summary.sloc as f64,
    );
    push_difference(
        &mut differences,
        "ploc",
        left.summary.ploc as f64,
        right.summary.ploc as f64,
    );
    push_difference(
        &mut differences,
        "lloc",
        left.summary.lloc as f64,
        right.summary.lloc as f64,
    );
    push_difference(
        &mut differences,
        "cloc",
        left.summary.cloc as f64,
        right.summary.cloc as f64,
    );
    push_difference(
        &mut differences,
        "cognitive",
        left.summary.cognitive,
        right.summary.cognitive,
    );
    push_difference(
        &mut differences,
        "cyclomatic",
        left.summary.cyclomatic,
        right.summary.cyclomatic,
    );
    push_difference(
        &mut differences,
        "modified_cyclomatic",
        left.summary.modified_cyclomatic,
        right.summary.modified_cyclomatic,
    );
    push_difference(
        &mut differences,
        "arguments",
        left.summary.arguments as f64,
        right.summary.arguments as f64,
    );
    push_difference(
        &mut differences,
        "exits",
        left.summary.exits as f64,
        right.summary.exits as f64,
    );
    push_optional_difference(
        &mut differences,
        "mean_maintainability",
        left.summary.mean_maintainability,
        right.summary.mean_maintainability,
    );
    push_optional_difference(
        &mut differences,
        "mean_halstead_effort",
        left.summary.mean_halstead_effort,
        right.summary.mean_halstead_effort,
    );
    push_optional_difference(
        &mut differences,
        "functions_per_ksloc",
        left.rates.functions_per_ksloc,
        right.rates.functions_per_ksloc,
    );
    push_optional_difference(
        &mut differences,
        "cognitive_per_ksloc",
        left.rates.cognitive_per_ksloc,
        right.rates.cognitive_per_ksloc,
    );
    push_optional_difference(
        &mut differences,
        "cyclomatic_per_ksloc",
        left.rates.cyclomatic_per_ksloc,
        right.rates.cyclomatic_per_ksloc,
    );
    push_optional_difference(
        &mut differences,
        "arguments_per_function",
        left.rates.arguments_per_function,
        right.rates.arguments_per_function,
    );
    push_optional_difference(
        &mut differences,
        "exits_per_function",
        left.rates.exits_per_function,
        right.rates.exits_per_function,
    );
    push_optional_difference(
        &mut differences,
        "comment_fraction",
        left.rates.comment_fraction,
        right.rates.comment_fraction,
    );
    push_optional_difference(
        &mut differences,
        "function_cognitive_p50",
        left.distributions.cognitive.p50,
        right.distributions.cognitive.p50,
    );
    push_optional_difference(
        &mut differences,
        "function_cognitive_p90",
        left.distributions.cognitive.p90,
        right.distributions.cognitive.p90,
    );
    push_optional_difference(
        &mut differences,
        "function_cognitive_p99",
        left.distributions.cognitive.p99,
        right.distributions.cognitive.p99,
    );
    push_optional_difference(
        &mut differences,
        "function_cognitive_max",
        left.distributions.cognitive.max,
        right.distributions.cognitive.max,
    );
    push_optional_difference(
        &mut differences,
        "function_cyclomatic_p90",
        left.distributions.cyclomatic.p90,
        right.distributions.cyclomatic.p90,
    );
    push_optional_difference(
        &mut differences,
        "function_sloc_p90",
        left.distributions.sloc.p90,
        right.distributions.sloc.p90,
    );

    let left_files = left
        .files
        .iter()
        .map(|file| ((file.path.as_str(), file.language.as_str()), file))
        .collect::<BTreeMap<_, _>>();
    let right_files = right
        .files
        .iter()
        .map(|file| ((file.path.as_str(), file.language.as_str()), file))
        .collect::<BTreeMap<_, _>>();
    let mut matched_files = Vec::new();
    let mut only_left = Vec::new();
    for (identity, left_file) in &left_files {
        if let Some(right_file) = right_files.get(identity) {
            matched_files.push(matched_file_difference(left_file, right_file));
        } else {
            only_left.push(file_identity(left_file));
        }
    }
    let only_right = right_files
        .iter()
        .filter(|(identity, _)| !left_files.contains_key(identity))
        .map(|(_, file)| file_identity(file))
        .collect();
    matched_files.sort_by(|left, right| {
        descending(
            left.right_minus_left.cognitive.abs(),
            right.right_minus_left.cognitive.abs(),
        )
        .then_with(|| left.path.cmp(&right.path))
    });

    MetricsComparison {
        left: comparison_side(left),
        right: comparison_side(right),
        differences,
        matched_files,
        only_left,
        only_right,
        limitations: vec![
            "Differences are right minus left and have no intrinsic good/bad direction.".to_owned(),
            "Matched-file differences require identical root-relative paths and detected languages.".to_owned(),
            "AST metrics are structural proxies; compare executable behavior and fitness-to-intent separately.".to_owned(),
        ],
    }
}

fn comparison_side(report: &MetricsReport) -> MetricsComparisonSide {
    MetricsComparisonSide {
        root: report.root.clone(),
        analyzer: report.analyzer.clone(),
        coverage: report.coverage.clone(),
        summary: report.summary.clone(),
        rates: report.rates.clone(),
        distributions: report.distributions.clone(),
    }
}

fn push_optional_difference(
    differences: &mut Vec<NumericDifference>,
    metric: &str,
    left: Option<f64>,
    right: Option<f64>,
) {
    if let (Some(left), Some(right)) = (left, right) {
        push_difference(differences, metric, left, right);
    }
}

fn push_difference(differences: &mut Vec<NumericDifference>, metric: &str, left: f64, right: f64) {
    let delta = right - left;
    differences.push(NumericDifference {
        metric: metric.to_owned(),
        left,
        right,
        right_minus_left: delta,
        relative_change_from_left: (left != 0.0).then_some(delta / left.abs()),
    });
}

fn file_identity(file: &FileMetric) -> FileIdentity {
    FileIdentity {
        path: file.path.clone(),
        language: file.language.clone(),
    }
}

fn core_metrics(file: &FileMetric) -> FileCoreMetrics {
    FileCoreMetrics {
        sloc: file.sloc,
        cognitive: file.cognitive,
        cyclomatic: file.cyclomatic,
        arguments: file.arguments,
        exits: file.exits,
    }
}

fn matched_file_difference(left: &FileMetric, right: &FileMetric) -> MatchedFileDifference {
    MatchedFileDifference {
        path: left.path.clone(),
        language: left.language.clone(),
        left: core_metrics(left),
        right: core_metrics(right),
        right_minus_left: FileCoreDelta {
            sloc: signed_usize_delta(right.sloc, left.sloc),
            cognitive: right.cognitive - left.cognitive,
            cyclomatic: right.cyclomatic - left.cyclomatic,
            arguments: signed_usize_delta(right.arguments, left.arguments),
            exits: signed_usize_delta(right.exits, left.exits),
        },
    }
}

fn signed_usize_delta(right: usize, left: usize) -> i128 {
    i128::try_from(right).unwrap_or(i128::MAX) - i128::try_from(left).unwrap_or(i128::MAX)
}

pub fn rank_functions(
    report: &MetricsReport,
    sort: MetricSort,
    top: usize,
) -> Vec<&FunctionMetric> {
    let mut rows: Vec<_> = report.functions.iter().collect();
    rows.sort_by(|a, b| compare_function(a, b, sort));
    rows.truncate(top);
    rows
}

pub fn rank_files(report: &MetricsReport, sort: MetricSort, top: usize) -> Vec<&FileMetric> {
    let mut rows: Vec<_> = report.files.iter().collect();
    rows.sort_by(|a, b| compare_file(a, b, sort).then_with(|| a.path.cmp(&b.path)));
    rows.truncate(top);
    rows
}

fn descending<T: PartialOrd>(a: T, b: T) -> Ordering {
    b.partial_cmp(&a).unwrap_or(Ordering::Equal)
}
fn option_descending(a: Option<f64>, b: Option<f64>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => descending(a, b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}
fn option_ascending(a: Option<f64>, b: Option<f64>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_function(a: &FunctionMetric, b: &FunctionMetric, sort: MetricSort) -> Ordering {
    let metric = match sort {
        MetricSort::Cognitive => descending(a.cognitive, b.cognitive),
        MetricSort::Cyclomatic => descending(a.cyclomatic, b.cyclomatic),
        MetricSort::Sloc => b.sloc.cmp(&a.sloc),
        MetricSort::Arguments => b.arguments.cmp(&a.arguments),
        MetricSort::Exits => b.exits.cmp(&a.exits),
        MetricSort::Maintainability => option_ascending(a.maintainability, b.maintainability),
        MetricSort::HalsteadEffort => option_descending(a.halstead_effort, b.halstead_effort),
    };
    metric.then_with(|| function_tie(a, b))
}
fn function_tie(a: &FunctionMetric, b: &FunctionMetric) -> Ordering {
    a.path
        .cmp(&b.path)
        .then_with(|| a.start_line.cmp(&b.start_line))
        .then_with(|| a.name.cmp(&b.name))
}
fn compare_file(a: &FileMetric, b: &FileMetric, sort: MetricSort) -> Ordering {
    match sort {
        MetricSort::Cognitive => descending(a.cognitive, b.cognitive),
        MetricSort::Cyclomatic => descending(a.cyclomatic, b.cyclomatic),
        MetricSort::Sloc => b.sloc.cmp(&a.sloc),
        MetricSort::Arguments => b.arguments.cmp(&a.arguments),
        MetricSort::Exits => b.exits.cmp(&a.exits),
        MetricSort::Maintainability => option_ascending(a.maintainability, b.maintainability),
        MetricSort::HalsteadEffort => option_descending(a.halstead_effort, b.halstead_effort),
    }
}
