use std::cmp::Reverse;

use software_evaluation::api_surface::ApiReport;
use software_evaluation::cochange::CochangeLayoutReport;
use software_evaluation::cochange_support::{CochangeSupportReport, ExactRatio, SupportMassBin};
use software_evaluation::benchmark::{BenchmarkReport, RunReceipt};
use software_evaluation::deps::DependencyReport;
use software_evaluation::discipline::{
    DisciplineReport, DisciplineSort, Tail, rank_files as rank_discipline_files,
    rank_functions as rank_discipline_functions,
};
use software_evaluation::typespace::TypeSpaceReport;
use software_evaluation::duplicates::DuplicateReport;
use software_evaluation::shape::{
    IntegerDistribution, ShapeReport, rank_functions as rank_shape_functions,
};
use software_evaluation::symbols::{SymbolEdgeKind, SymbolReport};
use software_evaluation::tests_analysis::TestReport;

pub fn print_symbols(report: &SymbolReport, top: usize) {
    println!("analyzer: {}", report.analyzer);
    println!("root: {}", report.root);
    println!("epistemic-class: {}", report.epistemic_class);
    let coverage = &report.coverage;
    println!(
        "coverage: {} Rust files / {} entries; {} non-Rust-or-unsupported skipped; {} declarations -> {} identity nodes; {} references ({} call, {} type-use); syntax-error-files={}",
        coverage.rust_files_analyzed,
        coverage.filesystem_entries_enumerated,
        coverage.non_rust_or_unsupported_entries_skipped,
        coverage.declarations_extracted,
        coverage.symbols_extracted,
        report.resolution.references_total,
        coverage.call_references,
        coverage.type_use_references,
        coverage.syntax_error_files,
    );
    let resolution = &report.resolution;
    println!(
        "resolution: {}/{} resolved (same-file={}, unique-crate={}), ambiguous={}, external-or-unresolved={}, fraction={}",
        resolution.resolved_total,
        resolution.references_total,
        resolution.resolved_same_file,
        resolution.resolved_unique_crate,
        resolution.ambiguous,
        resolution.external_or_unresolved,
        resolution
            .resolution_fraction
            .map_or_else(|| "n/a".to_owned(), |fraction| format!("{fraction:.3}")),
    );
    let graph = &report.graph;
    let mut scc_size_counts = std::collections::BTreeMap::new();
    for size in &graph.strongly_connected_component_sizes {
        *scc_size_counts.entry(*size).or_insert(0usize) += 1;
    }
    let scc_sizes = scc_size_counts
        .iter()
        .rev()
        .map(|(size, count)| format!("{size}x{count}"))
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "graph: {} nodes, {} edges, {} strongly connected components; size-histogram={}",
        graph.node_count,
        graph.edge_count,
        graph.strongly_connected_component_count,
        if scc_sizes.is_empty() { "none" } else { &scc_sizes },
    );
    println!(
        "mutual reachability: {}/{} ordered same-component pairs (fraction={})",
        graph.mutually_reachable_pairs,
        graph
            .possible_nonself_pairs
            .map_or_else(|| "n/a".to_owned(), |count| count.to_string()),
        graph
            .mutual_reachability_fraction
            .map_or_else(|| "n/a".to_owned(), |fraction| format!("{fraction:.3}")),
    );
    let reachability = &report.working_set_reachability;
    println!(
        "working-set reachability: status={}; {}/{} nodes; min={} p50={} p90={} max={}; node-limit={}; work-upper-bound={}; work-limit={}",
        reachability_status(reachability.status),
        reachability.nodes_in_distribution,
        graph.node_count,
        shown_count(reachability.min),
        shown_count(reachability.p50),
        shown_count(reachability.p90),
        shown_count(reachability.max),
        reachability.node_limit,
        reachability
            .work_upper_bound
            .map_or_else(|| "overflow".to_owned(), |bound| bound.to_string()),
        reachability.work_limit,
    );
    print_ranked_symbols(
        "highest forward-reachable working sets",
        &reachability.top,
        top,
    );
    print_ranked_symbols(
        "highest transitive fan-in (load-bearing symbols)",
        &report.transitive_fan_in_tail,
        top,
    );
    println!(
        "per-file symbol counts: {} / {} shown",
        report.per_file_symbol_counts.len().min(top),
        report.per_file_symbol_counts.len(),
    );
    println!("  {:>7} PATH", "SYMBOLS");
    for row in report.per_file_symbol_counts.iter().take(top) {
        println!("  {:>7} {}", row.symbols, row.path);
    }
    let mut kind_counts = std::collections::BTreeMap::new();
    for edge in &report.edges {
        for kind in &edge.kinds {
            *kind_counts.entry(*kind).or_insert(0usize) += 1;
        }
    }
    println!(
        "edge relations: call={} type-use={} over {} unique directed pairs",
        kind_counts.get(&SymbolEdgeKind::Call).copied().unwrap_or(0),
        kind_counts
            .get(&SymbolEdgeKind::TypeUse)
            .copied()
            .unwrap_or(0),
        graph.edge_count,
    );
    print_limitations(&report.limitations);
}

fn reachability_status(status: software_evaluation::deps::ReachabilityStatus) -> &'static str {
    match status {
        software_evaluation::deps::ReachabilityStatus::Computed => "computed",
        software_evaluation::deps::ReachabilityStatus::NotApplicable => "not_applicable",
        software_evaluation::deps::ReachabilityStatus::SizeLimit => "size_limit",
        software_evaluation::deps::ReachabilityStatus::WorkLimit => "work_limit",
    }
}

fn shown_count(value: Option<usize>) -> String {
    value.map_or_else(|| "n/a".to_owned(), |count| count.to_string())
}

fn print_ranked_symbols(
    label: &str,
    rows: &[software_evaluation::symbols::RankedSymbol],
    top: usize,
) {
    println!("{label}: {} / {} shown", rows.len().min(top), rows.len());
    println!("  {:>8} SYMBOL", "COUNT");
    for row in rows.iter().take(top) {
        println!("  {:>8} {}", row.count, row.id);
    }
}

pub fn print_dependencies(report: &DependencyReport, top: usize) {
    println!("analyzer: {}", report.analyzer);
    println!("root: {}", report.root);
    println!(
        "coverage: {} source files / {} entries; {} declarations; {} manifests ({} unreadable, skipped); syntax-error-files={}",
        report.coverage.source_files_analyzed,
        report.coverage.filesystem_entries_enumerated,
        report.coverage.declarations_extracted,
        report.coverage.manifests_analyzed,
        report.coverage.manifests_unreadable,
        report.syntax_error_files,
    );
    for manifest in &report.unreadable_manifests {
        let reason = manifest.reason.split_whitespace().collect::<Vec<_>>().join(" ");
        println!("  unreadable manifest (skipped): {} — {reason}", manifest.path);
    }
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
        "mutual reachability: {}/{} ordered same-component pairs (fraction={})",
        propagation.mutually_reachable_pairs,
        propagation
            .mutual_possible_pairs
            .map_or_else(|| "n/a".to_owned(), |value| value.to_string()),
        propagation
            .mutual_reachability_fraction
            .map_or_else(|| "n/a".to_owned(), |value| format!("{value:.3}")),
    );
    println!(
        "  within worst weak component: {}/{} pairs over {} files (fraction={}); {} weak components, largest {} files",
        propagation.worst_weak_component_mutually_reachable_pairs,
        if propagation.worst_weak_component_files >= 2 {
            ((propagation.worst_weak_component_files as u128)
                * (propagation.worst_weak_component_files as u128 - 1))
                .to_string()
        } else {
            "n/a".to_owned()
        },
        propagation.worst_weak_component_files,
        propagation
            .worst_weak_component_mutual_reachability_fraction
            .map_or_else(|| "n/a".to_owned(), |value| format!("{value:.3}")),
        propagation.weak_components,
        propagation.largest_weak_component_files,
    );
    if let Some(depth) = &report.condensation_depth {
        println!(
            "condensation depth: {} SCC nodes, {} edges over {} files; depth_in p50={} p90={} max={}, depth_out p50={} p90={} max={}",
            depth.condensation_nodes,
            depth.condensation_edges,
            depth.source_files,
            depth.depth_in_p50,
            depth.depth_in_p90,
            depth.depth_in_max,
            depth.depth_out_p50,
            depth.depth_out_p90,
            depth.depth_out_max,
        );
        let witness = depth
            .longest_path
            .iter()
            .map(|step| {
                if step.scc_files > 1 {
                    format!("{} (SCC of {})", step.file, step.scc_files)
                } else {
                    step.file.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" -> ");
        println!("  longest path: {witness}");
    }
    let layout = &report.layout;
    println!(
        "layout: {} analyzed files, {} internal undirected edges",
        layout.analyzed_files, layout.internal_undirected_edges,
    );
    for partition in &layout.partitions {
        println!(
            "  {}: communities={}, intra={}, cross={} (fraction={}), modularity Q={}",
            partition.granularity,
            partition.communities,
            partition.intra_community_edges,
            partition.cross_community_edges,
            optional(partition.cross_community_edge_fraction),
            optional(partition.modularity),
        );
        if !partition.rows.is_empty() {
            println!(
                "    {:<40} {:>6} {:>6} {:>6} {:>6}",
                "PATH", "FILES", "INTRA", "OUT", "IN"
            );
            for row in partition.rows.iter().take(top) {
                println!(
                    "    {:<40} {:>6} {:>6} {:>6} {:>6}",
                    row.path, row.files, row.intra_edges, row.out_edges, row.in_edges,
                );
            }
        }
        if !partition.rows.is_empty() {
            println!("    boundary endpoint dispersion (file denominator = FILES):");
            println!(
                "    {:<40} {:>8} {:>9} {:>8}  COVER-90 WITNESS",
                "PATH", "IN-FILES", "OUT-FILES", "COVER-90"
            );
            for row in partition.rows.iter().take(top) {
                println!(
                    "    {:<40} {:>8} {:>9} {:>8}  {}",
                    row.path,
                    row.boundary_in_files,
                    row.boundary_out_files,
                    row.boundary_cover_90_files,
                    row.boundary_cover_90_file_paths.join(", "),
                );
            }
        }
        println!(
            "    direction inconsistency: {}/{} (fraction={})",
            partition.direction_inconsistency_numerator,
            partition.direction_inconsistency_denominator,
            optional(partition.direction_inconsistency),
        );
        if !partition.direction_pairs.is_empty() {
            println!(
                "    {:<28} {:<28} {:>6} {:>6}",
                "PATH A", "PATH B", "E_AB", "E_BA"
            );
            for pair in partition.direction_pairs.iter().take(top) {
                println!(
                    "    {:<28} {:<28} {:>6} {:>6}",
                    pair.path_a, pair.path_b, pair.e_ab, pair.e_ba,
                );
                for edge in &pair.edge_witnesses {
                    println!("      witness: {} -> {}", edge.source, edge.target);
                }
            }
        }
    }

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

pub fn print_discipline(report: &DisciplineReport, sort: DisciplineSort, top: usize) {
    let coverage = &report.coverage;
    println!("analyzer: {}", report.analyzer);
    println!("root: {}", report.root);
    println!("sort: {}", discipline_sort_name(sort));
    println!(
        "coverage: {} supported / {} enumerated files; {} skipped; {} functions; syntax-error-files={}",
        coverage.supported_files,
        coverage.enumerated_files,
        coverage.skipped_unsupported_files,
        coverage.functions_total,
        coverage.syntax_error_files,
    );
    let languages = coverage
        .functions_per_language
        .iter()
        .map(|row| format!("{}={}", row.language.name(), row.functions))
        .collect::<Vec<_>>()
        .join(" ");
    println!(
        "functions per language: {}",
        if languages.is_empty() { "none".to_owned() } else { languages }
    );
    println!(
        "purity: {} / {} syntactically pure (fraction={})",
        coverage.pure_functions,
        coverage.functions_total,
        optional(coverage.pure_fraction),
    );
    let totals = &coverage.totals;
    println!(
        "effect totals: {} nonlocal-writes, {} mut-params, {} unsafe-blocks, {} effect-calls",
        totals.nonlocal_writes, totals.mut_params, totals.unsafe_blocks, totals.effect_calls,
    );
    println!(
        "mutation totals: {} bindings, {} mutable, {} reassignments, {} shadowings",
        totals.bindings, totals.mutable_bindings, totals.reassignments, totals.shadowings,
    );
    println!(
        "error totals: {} try-propagations, {} unwrap/expect, {} panic-like, {} broad-catches, {} empty-catches, {} ignored-results",
        totals.try_propagations,
        totals.unwrap_expect,
        totals.panic_like,
        totals.broad_catches,
        totals.empty_catches,
        totals.ignored_results,
    );
    println!(
        "type totals: {} string-literal-conditions, {} any-annotations, {} unannotated-params, {} type-ignores",
        totals.string_literal_conditions,
        totals.any_annotations,
        totals.unannotated_params,
        totals.type_ignores,
    );
    println!(
        "file totals: {} magic-numbers, {} magic-strings, {} global-mutable-state",
        totals.magic_numbers, totals.magic_strings, totals.global_mutable_state,
    );
    println!("repo tails (nearest-rank p50/p90/p99):");
    println!("  {:<28} {:>6} {:>6} {:>6}", "METRIC", "P50", "P90", "P99");
    for (name, tail) in [
        ("mutable_bindings", &coverage.tails.mutable_bindings),
        (
            "max_mutable_live_range_lines",
            &coverage.tails.max_mutable_live_range_lines,
        ),
        ("max_call_chain_len", &coverage.tails.max_call_chain_len),
        ("params", &coverage.tails.params),
    ] {
        print_tail(name, tail);
    }

    let files = rank_discipline_files(report, sort, top);
    println!("files: {} / {} shown", files.len(), report.files.len());
    println!(
        "  {:<12} {:>6} {:>6} {:>7} {:>6} {:>6} {:>6} {:>6} PATH",
        "LANG", "FUNCS", "EFFECT", "NONLOC", "MUT", "CHAIN", "ERRS", "PARAMS"
    );
    for file in files {
        let sums = &file.sums;
        println!(
            "  {:<12} {:>6} {:>6} {:>7} {:>6} {:>6} {:>6} {:>6} {}",
            file.language.name(),
            file.functions,
            sums.effect_calls,
            sums.nonlocal_writes,
            sums.mutable_bindings,
            sums.max_call_chain_len,
            sums.unwrap_expect + sums.panic_like + sums.broad_catches + sums.empty_catches + sums.ignored_results,
            sums.params,
            file.path,
        );
    }

    let functions = rank_discipline_functions(report, sort, top);
    println!(
        "function hotspots: {} / {} shown",
        functions.len(),
        report.functions.len()
    );
    println!(
        "  {:<12} {:>5} {:<7} {:>4} {:>4} {:>5} {:>4} {:>5} {:>4} LOCATION / NAME",
        "LANG", "PURE", "EFFECT", "NLW", "MUT", "RANGE", "CHN", "ERRS", "PRM"
    );
    for function in functions {
        println!(
            "  {:<12} {:>5} {:>7} {:>4} {:>4} {:>5} {:>4} {:>5} {:>4} {}:{} {}",
            function.language.name(),
            if function.syntactically_pure { "yes" } else { "no" },
            function.effect_calls,
            function.nonlocal_writes,
            function.mutable_bindings,
            function.max_mutable_live_range_lines,
            function.max_call_chain_len,
            function.unwrap_expect
                + function.panic_like
                + function.broad_catches
                + function.empty_catches
                + function.ignored_results,
            function.params,
            function.path,
            function.start_line,
            function.name,
        );
    }
    print_limitations(&report.limitations);
}

pub fn print_typespace(report: &TypeSpaceReport, top: usize) {
    println!("analyzer: {}", report.analyzer);
    println!("root: {}", report.root);
    println!("epistemic class: {}", report.epistemic_class);
    println!(
        "coverage: {} supported / {} enumerated files; {} skipped; syntax-error-files={}",
        report.coverage.supported_files,
        report.coverage.enumerated_files,
        report.coverage.skipped_unsupported_files,
        report.coverage.syntax_error_files,
    );
    let languages = report.coverage.files_per_language.iter()
        .map(|(language, files)| format!("{language}={files}"))
        .collect::<Vec<_>>().join(" ");
    println!("files per language: {}", if languages.is_empty() { "none" } else { &languages });
    println!("determinant coverage by language:");
    for (language, determinants) in &report.coverage.determinants_per_language {
        println!("  {language}: {determinants}");
    }

    let t1 = &report.t1;
    println!("T1 algebraic type shape [proxy over declared type syntax]:");
    println!(
        "  data-bearing enums: {} / {} all type definitions; denominator partition: structs={} + data-bearing-enums={} + field-less-tag-enums={} + other={}",
        t1.data_bearing_enums, t1.all_type_definitions, t1.structs, t1.data_bearing_enums,
        t1.fieldless_tag_enums, t1.other_type_definitions,
    );
    println!(
        "  Option+bool tail: {} / {} structs with >=2 fields have >=2 such fields; {} mentions over {} structs",
        t1.structs_with_at_least_two_option_bool_fields,
        t1.structs_with_at_least_two_fields,
        t1.option_bool_fields,
        t1.structs,
    );

    let t2 = &report.t2;
    println!("T2 dynamic-state surface [proxy over declared type syntax; aliases defeat it]:");
    println!("  {} / {} type-constructor leaf mentions in struct-field and function-signature positions", t2.dynamic_state_mentions, t2.type_constructor_leaf_mentions);
    for (language, count) in &t2.by_language {
        println!("  {language}: {} / {}", count.numerator, count.denominator);
    }
    println!("  dedupe: overlaps discipline's bare-any count; this census is signature/field-scoped and includes container forms");

    let t4 = &report.t4;
    println!("T4 endomorphic closure [proxy over declared return syntax; closure != lawfulness]:");
    println!("  endomorphic methods: {} / {} public methods; owned={}; borrowed={}; mutant-builder={}", t4.endomorphic_methods, t4.public_methods, t4.owned_endomorphic_methods, t4.borrowed_endomorphic_methods, t4.mutant_endomorphic_methods);
    println!("  binary closures: {} / {} Rust functions have (T,T)->T or (&T,&T)->T shape", t4.binary_closures, t4.functions_censused_for_binary_closure);

    let t5 = &report.t5;
    println!("T5 ownership-evasion density [proxy over declared/call syntax]:");
    println!("  shared-mutable: {} type mentions + {} borrow/lock calls / {} type leaves + {} call expressions", t5.shared_mutable_type_mentions, t5.borrow_lock_calls, t5.type_constructor_leaf_mentions, t5.call_expressions);
    println!("  shared-ownership: {} Rc/Arc type mentions / {} type leaves", t5.shared_ownership_type_mentions, t5.type_constructor_leaf_mentions);
    println!("  clone density: {} / {} call expressions; cross-check discipline's unsafe count", t5.clone_calls, t5.call_expressions);

    let t6 = &report.t6;
    println!("T6 newtype adoption [proxy over declared type syntax; within-language distribution only]:");
    println!("  wide primitives: {} / {} type mentions in public fn-param + pub-struct-field positions; non-primitive={}", t6.wide_primitive_mentions, t6.public_boundary_type_mentions, t6.non_primitive_mentions);
    println!("  newtype supply: {}; costume newtypes with public inner field: {} / {}", t6.newtype_supply, t6.costume_newtypes, t6.newtype_supply);

    print_typespace_tables(report, top);
    print_limitations(&report.limitations);
}

fn print_typespace_tables(report: &TypeSpaceReport, top: usize) {
    let mut structs = report.t1.structs_detail.iter().collect::<Vec<_>>();
    structs.sort_by(|a, b| b.option_bool_fields.cmp(&a.option_bool_fields).then_with(|| b.fields.cmp(&a.fields)).then_with(|| (&a.path, a.line, &a.name).cmp(&(&b.path, b.line, &b.name))));
    println!("T1 struct offenders: {} / {} shown", structs.len().min(top), structs.len());
    for row in structs.into_iter().take(top) { println!("  {:>4} option+bool / {:>4} fields  {}:{} {}", row.option_bool_fields, row.fields, row.path, row.line, row.name); }

    let mut dynamic = report.t2.items.iter().collect::<Vec<_>>();
    dynamic.sort_by(|a, b| b.dynamic_mentions.cmp(&a.dynamic_mentions).then_with(|| (&a.path, a.line, &a.name).cmp(&(&b.path, b.line, &b.name))));
    println!("T2 dynamic-state offenders: {} / {} shown", dynamic.len().min(top), dynamic.len());
    for row in dynamic.into_iter().take(top) { println!("  {:<10} {:>4} / {:>4} leaves  {}:{} {}", row.language, row.dynamic_mentions, row.type_leaf_mentions, row.path, row.line, row.name); }

    let mut types = report.t4.types.iter().collect::<Vec<_>>();
    types.sort_by(|a, b| {
        let left = a.endomorphic_methods.saturating_mul(b.public_methods);
        let right = b.endomorphic_methods.saturating_mul(a.public_methods);
        right.cmp(&left).then_with(|| b.endomorphic_methods.cmp(&a.endomorphic_methods)).then_with(|| (&a.path, a.line, &a.name).cmp(&(&b.path, b.line, &b.name)))
    });
    println!("T4 most-endomorphic types: {} / {} shown", types.len().min(top), types.len());
    for row in types.into_iter().take(top) { println!("  {:>4} / {:>4} methods (owned={} borrowed={} mutant={})  {}:{} {}", row.endomorphic_methods, row.public_methods, row.owned_endomorphic_methods, row.borrowed_endomorphic_methods, row.mutant_endomorphic_methods, row.path, row.line, row.name); }
    println!("T4 binary closures: {} / {} shown", report.t4.binary_items.len().min(top), report.t4.binary_items.len());
    for row in report.t4.binary_items.iter().take(top) { println!("  {}:{} {}", row.path, row.line, row.name); }

    let mut files = report.t5.files.iter().collect::<Vec<_>>();
    files.sort_by(|a, b| (b.shared_mutable_type_mentions + b.borrow_lock_calls + b.shared_ownership_type_mentions + b.clone_calls).cmp(&(a.shared_mutable_type_mentions + a.borrow_lock_calls + a.shared_ownership_type_mentions + a.clone_calls)).then_with(|| a.path.cmp(&b.path)));
    println!("T5 file offenders: {} / {} shown", files.len().min(top), files.len());
    for row in files.into_iter().take(top) { println!("  smut={:>3}+{:>3} shared={:>3} clone={:>4}/{:<4} leaves={:<5} {}", row.shared_mutable_type_mentions, row.borrow_lock_calls, row.shared_ownership_type_mentions, row.clone_calls, row.call_expressions, row.type_constructor_leaf_mentions, row.path); }

    let mut primitive = report.t6.primitive_items.iter().collect::<Vec<_>>();
    primitive.sort_by(|a, b| b.primitive_mentions.cmp(&a.primitive_mentions).then_with(|| (&a.path, a.line, &a.name).cmp(&(&b.path, b.line, &b.name))));
    println!("T6 primitive-boundary offenders: {} / {} shown", primitive.len().min(top), primitive.len());
    for row in primitive.into_iter().take(top) { println!("  {:>4} / {:>4} mentions  {}:{} {}", row.primitive_mentions, row.type_mentions, row.path, row.line, row.name); }
    println!("T6 newtypes: {} / {} shown", report.t6.newtypes.len().min(top), report.t6.newtypes.len());
    for row in report.t6.newtypes.iter().take(top) { println!("  {:<7} {:<16} {}:{} {}", if row.costume { "costume" } else { "private" }, row.wrapped_type, row.path, row.line, row.name); }
}

fn print_tail(name: &str, tail: &Tail) {
    println!(
        "  {:<28} {:>6} {:>6} {:>6}",
        name, tail.p50, tail.p90, tail.p99
    );
}

fn discipline_sort_name(sort: DisciplineSort) -> &'static str {
    match sort {
        DisciplineSort::Pure => "pure",
        DisciplineSort::Mutable => "mutable",
        DisciplineSort::LiveRange => "live-range",
        DisciplineSort::Chain => "chain",
        DisciplineSort::Errors => "errors",
        DisciplineSort::Params => "params",
    }
}

pub fn print_shape(report: &ShapeReport, top: usize) {
    let coverage = &report.coverage;
    println!("analyzer: {}", report.analyzer);
    println!("root: {}", report.root);
    println!("epistemic class: {}", report.epistemic_class);
    println!(
        "coverage: {} supported / {} enumerated files; {} skipped; {} functions; syntax-error-files={}",
        coverage.supported_files,
        coverage.enumerated_files,
        coverage.skipped_unsupported_files,
        coverage.functions_analyzed,
        coverage.syntax_error_files,
    );
    let languages = coverage
        .functions_per_language
        .iter()
        .map(|row| {
            format!(
                "{}={}/{}-files",
                row.language.name(), row.functions_analyzed, row.files_analyzed
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    println!("functions per language: {}", if languages.is_empty() { "none" } else { &languages });
    println!(
        "shallow corner: {} / {} functions (interface-width >= interior-volume, volume > 0)",
        coverage.shallow_functions, coverage.shallow_denominator,
    );
    println!(
        "no-else ifs with then-arm >= 8 statements: {}",
        coverage.no_else_large_then_arms,
    );
    println!("repo distributions (nearest-rank min/p50/p90/max):");
    println!("  {:<24} {:>8} {:>8} {:>8} {:>8}", "METRIC", "MIN", "P50", "P90", "MAX");
    for (name, distribution) in [
        ("interface_width", &report.distributions.interface_width),
        ("interior_volume", &report.distributions.interior_volume),
        ("cyclomatic", &report.distributions.cyclomatic),
        ("cognitive", &report.distributions.cognitive),
        ("cognitive_gap", &report.distributions.cognitive_gap),
        ("max_nesting_depth", &report.distributions.max_nesting_depth),
    ] {
        print_shape_distribution(name, distribution);
    }
    let ratios = &report.distributions.max_arm_size_ratio;
    println!(
        "  {:<24} {:>8} {:>8} {:>8} {:>8}  (n={})",
        "max_arm_size_ratio",
        optional(ratios.min),
        optional(ratios.p50),
        optional(ratios.p90),
        optional(ratios.max),
        ratios.observations,
    );

    println!("file distributions (nearest-rank min/p50/p90/max): {} files", report.files.len());
    for file in &report.files {
        println!(
            "  {} [{}; functions={}; shallow={}; no-else-large={}]",
            file.path,
            file.language.name(),
            file.functions_analyzed,
            file.shallow_functions,
            file.no_else_large_then_arms,
        );
        println!(
            "    width={} volume={} cyclomatic={} cognitive={}",
            compact_distribution(&file.distributions.interface_width),
            compact_distribution(&file.distributions.interior_volume),
            compact_distribution(&file.distributions.cyclomatic),
            compact_distribution(&file.distributions.cognitive),
        );
        let ratio = &file.distributions.max_arm_size_ratio;
        println!(
            "    gap={} nesting={} arm-ratio={} (n={})",
            compact_distribution(&file.distributions.cognitive_gap),
            compact_distribution(&file.distributions.max_nesting_depth),
            compact_float_distribution(ratio.min, ratio.p50, ratio.p90, ratio.max),
            ratio.observations,
        );
    }

    let functions = rank_shape_functions(report, top);
    println!("function shapes: {} / {} shown", functions.len(), report.functions.len());
    println!(
        "  {:<12} {:>3} {:>3} {:>4} {:>4} {:>4} {:>4} {:>4} {:>6} {:>6} LOCATION / NAME",
        "LANG", "IFW", "VOL", "CYC", "COG", "GAP", "NEST", "SHAL", "RATIO", "NOELSE"
    );
    for function in functions {
        println!(
            "  {:<12} {:>3} {:>3} {:>4} {:>4} {:>4} {:>4} {:>4} {:>6} {:>6} {}:{} {}",
            function.language.name(),
            function.interface_width,
            function.interior_volume,
            function.cyclomatic,
            function.cognitive,
            function.cognitive_gap,
            function.max_nesting_depth,
            if function.shallow { "yes" } else { "no" },
            function.max_arm_size_ratio.map_or_else(|| "n/a".to_owned(), |value| format!("{value:.2}")),
            function.no_else_large_then_arms,
            function.path,
            function.start_line,
            function.name,
        );
    }
    print_limitations(&report.limitations);
}

fn print_shape_distribution(name: &str, distribution: &IntegerDistribution) {
    println!(
        "  {:<24} {:>8} {:>8} {:>8} {:>8}  (n={})",
        name,
        optional_integer(distribution.min),
        optional_integer(distribution.p50),
        optional_integer(distribution.p90),
        optional_integer(distribution.max),
        distribution.observations,
    );
}

fn compact_distribution(distribution: &IntegerDistribution) -> String {
    [
        distribution.min,
        distribution.p50,
        distribution.p90,
        distribution.max,
    ]
    .into_iter()
    .map(optional_integer)
    .collect::<Vec<_>>()
    .join("/")
}

fn optional_integer(value: Option<i64>) -> String {
    value.map_or_else(|| "n/a".to_owned(), |number| number.to_string())
}

fn compact_float_distribution(
    min: Option<f64>,
    p50: Option<f64>,
    p90: Option<f64>,
    max: Option<f64>,
) -> String {
    [min, p50, p90, max]
        .into_iter()
        .map(|value| value.map_or_else(|| "n/a".to_owned(), |number| format!("{number:.3}")))
        .collect::<Vec<_>>()
        .join("/")
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

pub fn print_cochange_layout(report: &CochangeLayoutReport, top: usize) {
    let artifact = serde_json::to_string(&report.artifact).unwrap_or_else(|_| "null".to_owned());
    let h = &report.history_coverage;
    let u = &report.universe_coverage;
    let p = &report.source_provenance;
    println!("cochange-layout analyzer={}", report.analyzer);
    println!("artifact {artifact}");
    println!(
        "history requested_commits={} commits_streamed={} truncated={} eligible_commits={} broad_commits_excluded={} (cap={}) below_pair_threshold_commits={} earliest_committer_unix_seconds={} latest_committer_unix_seconds={}",
        h.requested_commits,
        h.commits_streamed,
        h.truncated,
        h.eligible_commits,
        h.broad_commits_excluded,
        h.broad_commit_cap,
        h.below_pair_threshold_commits,
        h.earliest_committer_unix_seconds
            .map_or_else(|| "n/a".to_owned(), |v| v.to_string()),
        h.latest_committer_unix_seconds
            .map_or_else(|| "n/a".to_owned(), |v| v.to_string()),
    );
    println!(
        "history_receipt git_version={} command={} stdout_sha256={} stdout_bytes={}",
        h.git_version, h.command, h.stdout_sha256, h.stdout_bytes
    );
    println!(
        "universe tracked_regular_files={} utf8_path_regular_files={} source_classified_files={} files_touched_in_history={} files_never_touched={}",
        u.tracked_regular_files,
        u.utf8_path_regular_files,
        u.source_classified_files,
        u.files_touched_in_history,
        u.files_never_touched,
    );
    println!(
        "source_tree_receipt git_version={} command={} stdout_sha256={} stdout_bytes={}",
        p.git_version, p.ls_tree_command, p.ls_tree_stdout_sha256, p.ls_tree_stdout_bytes
    );
    println!(
        "mass total_pair_weight={:.3} ideal={:.3} quantization_bound={:.3e} weight_scale={}",
        report.total_pair_weight,
        report.total_pair_weight_ideal,
        report.total_pair_weight_quantization_bound,
        report.weight_scale,
    );
    for partition in &report.partitions {
        println!(
            "  {}: communities={}, intra={:.3}, cross={:.3} (fraction={}), modularity Q={}",
            partition.granularity,
            partition.communities,
            partition.intra_weight,
            partition.cross_weight,
            optional(partition.cross_weight_fraction),
            optional(partition.modularity),
        );
        if !partition.rows.is_empty() {
            println!("    {:<40} {:>6} {:>10} {:>10}", "PATH", "FILES", "INTRA", "CROSS");
            for row in partition.rows.iter().take(top) {
                println!(
                    "    {:<40} {:>6} {:>10.3} {:>10.3}",
                    row.path, row.files, row.intra_weight, row.cross_weight,
                );
            }
        }
    }
    print_limitations(&report.limitations);
}

pub fn print_cochange_support(report: &CochangeSupportReport) {
    println!("artifact: {}", report.artifact.id);
    println!("revision: {}", report.artifact.revision);
    println!("tree: {}", report.artifact.tree_digest);
    println!("cochange-support analyzer={}", report.analyzer);
    println!("static analyzer={}", report.static_analyzer);
    let history = &report.history_coverage;
    println!(
        "history: requested={} streamed={} truncated={} eligible={} broad_excluded={} below_pair_threshold={} cap={}",
        history.requested_commits,
        history.commits_streamed,
        history.truncated,
        history.eligible_commits,
        history.broad_commits_excluded,
        history.below_pair_threshold_commits,
        history.broad_commit_cap,
    );
    println!(
        "history range: earliest={:?} latest={:?}",
        history.earliest_committer_unix_seconds, history.latest_committer_unix_seconds
    );
    let universe = &report.universe_coverage;
    println!(
        "universe: tracked_regular={} utf8_regular={} cochange={} static={} static_only={} cochange_only={} intersection={} union={} intersection_touched={} intersection_never_touched={}",
        universe.tracked_regular_files,
        universe.utf8_path_regular_files,
        universe.source_classified_tracked_blobs,
        universe.static_analyzed_files,
        universe.static_only_files,
        universe.cochange_only_files,
        universe.intersection_files,
        universe.union_files,
        universe.intersection_files_touched_in_history,
        universe.intersection_files_never_touched,
    );
    println!(
        "history evidence: git_version={} command={} stdout_sha256={} stdout_bytes={}",
        history.git_version, history.command, history.stdout_sha256, history.stdout_bytes
    );
    let source = &report.source_provenance;
    println!(
        "tree evidence: git_version={} command={} stdout_sha256={} stdout_bytes={}",
        source.git_version,
        source.ls_tree_command,
        source.ls_tree_stdout_sha256,
        source.ls_tree_stdout_bytes,
    );
    let static_snapshot = &report.static_snapshot_provenance;
    println!(
        "blob evidence: git_version={} command={} request_sha256={} stdout_sha256={} stdout_bytes={}",
        static_snapshot.git_version,
        static_snapshot.cat_file_command,
        static_snapshot.cat_file_request_sha256,
        static_snapshot.cat_file_stdout_sha256,
        static_snapshot.cat_file_stdout_bytes,
    );
    println!(
        "intersected co-change mass: actual={:.9} ideal={:.9} quantization_bound={:.3e} scaled={}/{} ideal_scaled={} bound_scaled={}",
        report.total_intersected_pair_mass,
        report.total_intersected_pair_mass_ideal,
        report.total_intersected_pair_mass_quantization_bound,
        report.total_intersected_pair_mass_scaled,
        report.weight_scale,
        report.total_intersected_pair_mass_ideal_scaled,
        report.total_intersected_pair_mass_quantization_bound_scaled,
    );
    let cross_tab = &report.support_cross_tab;
    println!(
        "static reachability: status={:?} node_limit={} work_upper_bound={:?} work_limit={}",
        cross_tab.reachability_status,
        cross_tab.reachability_node_limit,
        cross_tab.reachability_work_upper_bound,
        cross_tab.reachability_work_limit,
    );
    print_support_bin("direct", Some(&cross_tab.direct));
    print_support_bin("transitive_only", cross_tab.transitive_only.as_ref());
    print_support_bin("unrelated", cross_tab.unrelated.as_ref());
    if let Some(pending) = &cross_tab.non_direct_uncomputed_mass {
        print_support_bin("non_direct_uncomputed", Some(pending));
    }
    let reverse = &report.reverse_static_edge_support;
    println!(
        "reverse static support: {}/{} cross-directory edges carry co-change mass (fraction={}) granularity={}",
        reverse.supported_cross_directory_edges,
        reverse.cross_directory_edges,
        shown_fraction(reverse.fraction),
        reverse.directory_granularity,
    );
    let jaccard = &report.commit_jaccard;
    println!(
        "commit Jaccard: cooccurring_pairs={} distribution_pairs={} distribution_minimum_cooccurrence={} top_minimum_cooccurrence={}",
        jaccard.cooccurring_pairs,
        jaccard.pairs_in_distribution,
        jaccard.distribution_minimum_cooccurrence,
        jaccard.top_pairs_minimum_cooccurrence,
    );
    println!(
        "Jaccard distribution: p50={} p90={} max={}",
        shown_ratio(jaccard.distribution.p50.as_ref()),
        shown_ratio(jaccard.distribution.p90.as_ref()),
        shown_ratio(jaccard.distribution.max.as_ref()),
    );
    println!("top Jaccard pairs:");
    for pair in &jaccard.top_pairs {
        println!(
            "  {:.6} {}/{} cooccurrence={} union={} left_commits={} right_commits={} {} <-> {}",
            pair.jaccard.value,
            pair.jaccard.numerator,
            pair.jaccard.denominator,
            pair.cooccurrence_commits,
            pair.union_commits,
            pair.left_commits,
            pair.right_commits,
            pair.left,
            pair.right,
        );
    }
    println!("interpretation: {}", report.interpretation);
    print_limitations(&report.limitations);
}

fn print_support_bin(name: &str, bin: Option<&SupportMassBin>) {
    match bin {
        Some(bin) => println!(
            "support {name}: pairs={} mass={:.9} scaled={} fraction={}",
            bin.pairs,
            bin.mass,
            bin.mass_scaled,
            shown_fraction(bin.fraction_of_total),
        ),
        None => println!("support {name}: uncomputed"),
    }
}

fn shown_fraction(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_owned(), |value| format!("{value:.6}"))
}

fn shown_ratio(ratio: Option<&ExactRatio>) -> String {
    ratio.map_or_else(
        || "n/a".to_owned(),
        |ratio| {
            format!(
                "{}/{} ({:.6})",
                ratio.numerator, ratio.denominator, ratio.value
            )
        },
    )
}
