use std::cmp::Reverse;

use software_evaluation::api_surface::ApiReport;
use software_evaluation::benchmark::{BenchmarkReport, RunReceipt};
use software_evaluation::deps::DependencyReport;
use software_evaluation::duplicates::DuplicateReport;
use software_evaluation::tests_analysis::TestReport;

pub fn print_dependencies(report: &DependencyReport, top: usize) {
    println!("analyzer: {}", report.analyzer);
    println!("root: {}", report.root);
    println!(
        "coverage: {} source files / {} entries; {} declarations; {} manifests; syntax-error-files={}",
        report.coverage.source_files_analyzed,
        report.coverage.filesystem_entries_enumerated,
        report.coverage.declarations_extracted,
        report.coverage.manifests_analyzed,
        report.syntax_error_files,
    );
    println!(
        "graph: {} nodes, {} edges ({} internal, {} external, {} unresolved), {} weak components, {} cycles, condensation-depth={}",
        report.node_count,
        report.edge_count,
        report.internal_edges,
        report.external_edges,
        report.unresolved_edges,
        report.weak_components.len(),
        report.cycles.len(),
        report
            .condensation_maximum_depth
            .map_or_else(|| "n/a".to_owned(), |value| value.to_string()),
    );
    let propagation = &report.propagation;
    let status = match propagation.reachability_status {
        software_evaluation::deps::ReachabilityStatus::Computed => "computed",
        software_evaluation::deps::ReachabilityStatus::NotApplicable => "not_applicable",
        software_evaluation::deps::ReachabilityStatus::SizeLimit => "size_limit",
        software_evaluation::deps::ReachabilityStatus::WorkLimit => "work_limit",
    };
    println!(
        "internal transitive reachability: {}/{} non-self source-file pairs; status={status}; node-limit={}; work-upper-bound={}; work-limit={}",
        propagation
            .reachable_nonself_pairs
            .map_or_else(|| "n/a".to_owned(), |value| value.to_string()),
        propagation
            .possible_nonself_pairs
            .map_or_else(|| "n/a".to_owned(), |value| value.to_string()),
        propagation.reachability_node_limit,
        propagation
            .reachability_work_upper_bound
            .map_or_else(|| "overflow".to_owned(), |value| value.to_string()),
        propagation.reachability_work_limit,
    );
    println!(
        "internal cycles: {} cyclic components, {}/{} cyclic source files, largest={} source files",
        propagation.cyclic_components,
        if propagation.source_files == 0 {
            "n/a".to_owned()
        } else {
            propagation.cyclic_source_files.to_string()
        },
        if propagation.source_files == 0 {
            "n/a".to_owned()
        } else {
            propagation.source_files.to_string()
        },
        if propagation.source_files == 0 {
            "n/a".to_owned()
        } else {
            propagation.largest_cyclic_component_files.to_string()
        },
    );
    println!(
        "manifest dependencies: {} total, {} non-registry, {} risky literal sources",
        report.manifest_dependency_count,
        report.non_registry_manifest_dependency_count,
        report.risky_manifest_dependency_count,
    );
    if !report.manifest_source_kind_counts.is_empty() {
        let counts = report
            .manifest_source_kind_counts
            .iter()
            .map(|(kind, count)| format!("{kind}={count}"))
            .collect::<Vec<_>>()
            .join(" ");
        println!("manifest source kinds: {counts}");
    }

    let mut nodes = report.nodes.iter().collect::<Vec<_>>();
    nodes.sort_by_key(|node| {
        (
            Reverse(node.fan_out),
            Reverse(node.fan_in),
            node.id.as_str(),
        )
    });
    println!(
        "highest fan-out nodes: {} / {} shown",
        nodes.len().min(top),
        nodes.len()
    );
    println!(
        "  {:>7} {:>7} {:>14} {:>15} {:>18} {:>19} {:<22} NODE",
        "FAN-OUT",
        "FAN-IN",
        "INTERNAL-OUT",
        "INTERNAL-IN",
        "TRANSITIVE-OUT",
        "TRANSITIVE-IN",
        "KIND"
    );
    for node in nodes.into_iter().take(top) {
        let shown =
            |value: Option<usize>| value.map_or_else(|| "n/a".to_owned(), |n| n.to_string());
        println!(
            "  {:>7} {:>7} {:>14} {:>15} {:>18} {:>19} {:<22?} {}",
            node.fan_out,
            node.fan_in,
            shown(node.direct_internal_out_degree),
            shown(node.direct_internal_in_degree),
            shown(node.transitive_internal_out_count),
            shown(node.transitive_internal_in_count),
            node.kind,
            node.id
        );
    }

    if !report.cycles.is_empty() {
        println!(
            "cycles: {} / {} shown",
            report.cycles.len().min(top),
            report.cycles.len()
        );
        for cycle in report.cycles.iter().take(top) {
            println!("  {}", cycle.join(" -> "));
        }
    }
    if !report.manifest_dependencies.is_empty() {
        println!(
            "manifest rows: {} / {} shown",
            report.manifest_dependencies.len().min(top),
            report.manifest_dependencies.len()
        );
        println!(
            "  {:<12} {:<16} {:<24} {:<12} REQUIREMENT",
            "ECOSYSTEM", "SCOPE", "NAME", "SOURCE"
        );
        for dependency in report.manifest_dependencies.iter().take(top) {
            println!(
                "  {:<12} {:<16} {:<24} {:<12?} {}",
                dependency.ecosystem,
                dependency.scope,
                dependency.name,
                dependency.source_kind,
                dependency.requirement,
            );
        }
    }
    print_limitations(&report.limitations);
}

pub fn print_duplicates(report: &DuplicateReport) {
    println!("analyzer: {}", report.analyzer);
    println!("root: {}", report.root);
    println!(
        "coverage: {} considered / {} enumerated files; {} skipped; {} tokens; syntax-error-files={}",
        report.coverage.considered_files,
        report.coverage.enumerated_files,
        report.coverage.skipped_files,
        report.coverage.considered_tokens,
        report.coverage.syntax_error_files,
    );
    println!(
        "thresholds: min-tokens={} min-lines={} max-groups={}",
        report.config.min_tokens, report.config.min_lines, report.config.max_groups,
    );
    println!(
        "clones: {} groups, {} occurrences, {} duplicated tokens, {} duplicated lines",
        report.totals.clone_groups,
        report.totals.clone_occurrences,
        report.totals.duplicated_tokens,
        report.totals.duplicated_lines,
    );
    for (index, group) in report.groups.iter().enumerate() {
        println!(
            "group {}: {} tokens × {} occurrences; {} lines/occurrence; mass={} tokens / {} lines; digest={}",
            index + 1,
            group.tokens_per_occurrence,
            group.occurrences.len(),
            group.lines_per_occurrence,
            group.duplicated_token_mass,
            group.duplicated_line_mass,
            group.digest,
        );
        for occurrence in &group.occurrences {
            println!(
                "  {}:{}-{}",
                occurrence.path, occurrence.start_line, occurrence.end_line
            );
        }
    }
    print_limitations(&report.limitations);
}

pub fn print_api(report: &ApiReport, top: usize) {
    println!("analyzer: {}", report.analyzer);
    println!("root: {}", report.root);
    println!(
        "coverage: {} parsed / {} source files / {} enumerated; {} skipped; {} source lines; syntax-error-files={}",
        report.coverage.parsed_files,
        report.coverage.source_files,
        report.coverage.enumerated_paths,
        report.coverage.skipped_non_source_paths,
        report.coverage.source_lines,
        report.coverage.syntax_error_files,
    );
    println!(
        "surface: {} symbols ({} functions, {} methods, {} types, {} constants, {} fields, {} other); documented={}; parameters={}; type-parameters={}; symbols/kSLOC={:.3}",
        report.counts.public_symbols,
        report.counts.functions,
        report.counts.methods,
        report.counts.types,
        report.counts.constants,
        report.counts.fields,
        report.counts.other,
        report.counts.documented_symbols,
        report.counts.total_parameters,
        report.counts.total_generic_or_type_parameters,
        report.counts.public_symbols_per_ksloc,
    );
    println!(
        "symbols: {} / {} shown",
        report.symbols.len().min(top),
        report.symbols.len()
    );
    println!(
        "  {:<40} {:>6} {:<12} {:<12} {:>6} {:>6} {:>5} SYMBOL",
        "PATH", "LINE", "LANGUAGE", "KIND", "PARAM", "GENERIC", "DOC"
    );
    for symbol in report.symbols.iter().take(top) {
        println!(
            "  {:<40} {:>6} {:<12} {:<12?} {:>6} {:>6} {:>5} {}",
            symbol.path,
            symbol.line,
            symbol.language.name(),
            symbol.kind,
            symbol.parameter_count,
            symbol.generic_or_type_parameter_count,
            if symbol.documentation_immediately_precedes {
                "yes"
            } else {
                "no"
            },
            symbol.symbol,
        );
        println!("    basis: {}", symbol.visibility_or_proxy_basis);
    }
    print_limitations(&report.limitations);
}

pub fn print_tests(report: &TestReport, top: usize) {
    let coverage = &report.coverage;
    println!("analyzer: {}", report.analyzer);
    println!("root: {}", report.root);
    println!(
        "coverage: {} supported / {} enumerated files; {} skipped; source={} files/{} lines; tests={} files/{} lines; syntax-error-files={}",
        coverage.supported_files,
        coverage.enumerated_files,
        coverage.skipped_unsupported_files,
        coverage.analyzed_source_files,
        coverage.analyzed_source_lines,
        coverage.test_files,
        coverage.test_lines,
        coverage.syntax_error_files,
    );
    println!(
        "test observations: {} cases, {} ignored, {} non-ignored, {} assertion-like calls; test-lines/source-line={}; cases/kSLOC={}",
        coverage.discovered_test_cases,
        coverage.ignored_test_cases,
        coverage.non_ignored_test_cases,
        coverage.assertion_like_calls,
        optional(coverage.test_lines_per_source_line),
        optional(coverage.test_cases_per_ksloc),
    );
    println!(
        "same-stem matching: {} / {} source modules matched; {} unmatched source modules; {} unmatched test files",
        coverage.source_modules_with_same_stem_test,
        coverage.source_modules_considered,
        report.unmatched_source_modules.len(),
        report.unmatched_test_files.len(),
    );
    let mut files = report.files.iter().collect::<Vec<_>>();
    files.sort_by_key(|file| {
        (
            Reverse(file.discovered_test_cases),
            Reverse(file.assertion_like_calls),
            file.path.as_str(),
        )
    });
    println!(
        "test machinery files: {} / {} shown",
        files.len().min(top),
        files.len()
    );
    println!(
        "  {:<12} {:<8} {:>8} {:>8} {:>8} {:>8} PATH",
        "LANG", "ROLE", "LINES", "CASES", "IGNORED", "ASSERTS"
    );
    for file in files.into_iter().take(top) {
        println!(
            "  {:<12} {:<8} {:>8} {:>8} {:>8} {:>8} {}",
            file.language.name(),
            format!("{:?}", file.role),
            file.lines,
            file.discovered_test_cases,
            file.ignored_test_cases,
            file.assertion_like_calls,
            file.path,
        );
    }
    print_paths(
        "unmatched source modules",
        &report.unmatched_source_modules,
        top,
    );
    print_paths("unmatched test files", &report.unmatched_test_files, top);
    print_limitations(&report.limitations);
}

pub fn print_benchmark(report: &BenchmarkReport) {
    println!("analyzer: {}", report.analyzer);
    println!("root: {}", report.root);
    println!(
        "command: {:?} {:?}",
        report.command.program, report.command.args
    );
    println!("command-sha256: {}", report.command.identity_sha256);
    println!("executable: {}", report.command.executable_locator);
    println!(
        "environment: os={} arch={} timer={}",
        report.environment.os, report.environment.architecture, report.environment.timer
    );
    println!(
        "coverage: warmups={}/{} measured={}/{} warmed-denominator={}",
        report.coverage.observed_warmup_runs,
        report.coverage.requested_warmup_runs,
        report.coverage.observed_measured_runs,
        report.coverage.requested_measured_runs,
        report.coverage.warmed_distribution_denominator,
    );
    println!("successful: {}", report.successful);
    print_run("first measured", &report.first_measured_run);
    let latency = &report.warmed_latency_ns;
    println!(
        "warmed latency: n={} min={} p50={} p95={} p99={} max={} mean={}",
        latency.sample_count,
        optional_ns(latency.min_ns),
        optional_ns(latency.p50_ns),
        optional_ns(latency.p95_ns),
        optional_ns(latency.p99_ns),
        optional_ns(latency.max_ns),
        optional_ns(latency.mean_ns),
    );
    if let Some(rate) = &report.warmed_units_per_second {
        println!(
            "warmed units/s: n={} p50={} p95={} p99={} mean={}",
            rate.sample_count,
            optional(rate.p50_per_second),
            optional(rate.p95_per_second),
            optional(rate.p99_per_second),
            optional(rate.mean_per_second),
        );
    }
    if let Some(rate) = &report.warmed_bytes_per_second {
        println!(
            "warmed bytes/s: n={} p50={} p95={} p99={} mean={}",
            rate.sample_count,
            optional(rate.p50_per_second),
            optional(rate.p95_per_second),
            optional(rate.p99_per_second),
            optional(rate.mean_per_second),
        );
    }
    for sample in &report.warmed_samples {
        print_run("sample", sample);
    }
    print_limitations(&report.limitations);
}

fn print_run(label: &str, run: &RunReceipt) {
    println!(
        "{label} #{}: success={} exit={:?} signal={:?} timeout={} elapsed={} stdout={}B stderr={}B peak-rss={}",
        run.ordinal,
        run.success,
        run.exit_code,
        run.termination_signal,
        run.timed_out,
        format_ns(run.elapsed_ns),
        run.stdout.len(),
        run.stderr.len(),
        run.peak_rss_bytes
            .map_or_else(|| "n/a".to_owned(), |value| value.to_string()),
    );
    if let Some(error) = &run.spawn_error {
        println!("  spawn-error: {error}");
    }
}

fn print_paths(label: &str, paths: &[String], top: usize) {
    if paths.is_empty() {
        return;
    }
    println!("{label}: {} / {} shown", paths.len().min(top), paths.len());
    for path in paths.iter().take(top) {
        println!("  {path}");
    }
}

fn optional(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_owned(), |value| format!("{value:.3}"))
}

fn optional_ns(value: Option<u128>) -> String {
    value.map_or_else(|| "n/a".to_owned(), format_ns)
}

fn format_ns(value: u128) -> String {
    if value >= 1_000_000_000 {
        format!("{:.3}s", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.3}ms", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.3}µs", value as f64 / 1_000.0)
    } else {
        format!("{value}ns")
    }
}

fn print_limitations(limitations: &[String]) {
    if limitations.is_empty() {
        return;
    }
    println!("limitations:");
    for limitation in limitations {
        println!("  - {limitation}");
    }
}
