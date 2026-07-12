use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use software_evaluation::change_profile::{ChangeProfileReport, CurrentFileRow, HistoryStatus};

const LEFT: f64 = 190.0;
const PLANE_WIDTH: f64 = 400.0;
const PLANE_GAP: f64 = 90.0;
const FACET_HEIGHT: f64 = 280.0;
const PLOT_HEIGHT: f64 = 150.0;

pub fn render_change_profile_text(report: &ChangeProfileReport, top: usize) -> String {
    let mut out = String::new();
    let artifact = serde_json::to_string(&report.artifact).unwrap_or_else(|_| "null".to_owned());
    let h = &report.history_coverage;
    let s = &report.source_coverage;
    let j = &report.join_coverage;
    let _ = writeln!(out, "change-profile analyzer={}", report.analyzer);
    let _ = writeln!(out, "artifact {artifact}");
    let _ = writeln!(
        out,
        "history requested_commits={} commits_analyzed={} truncated={} earliest_committer_unix_seconds={} latest_committer_unix_seconds={}",
        h.requested_commits,
        h.commits_analyzed,
        h.truncated,
        opt_i64(h.earliest_committer_unix_seconds),
        opt_i64(h.latest_committer_unix_seconds)
    );
    let p = &report.source_provenance;
    let _ = writeln!(
        out,
        "history_receipt git_version={} command={} stdout_sha256={} stdout_bytes={}",
        h.git_version, h.command, h.stdout_sha256, h.stdout_bytes
    );
    let _ = writeln!(
        out,
        "source tracked_regular_files={} utf8_path_regular_files={} non_utf8_path_regular_files={} supported_source_files={} analyzed_source_files={} unsupported_regular_files={} syntax_error_files={}",
        s.tracked_regular_files,
        s.utf8_path_regular_files,
        s.non_utf8_path_regular_files,
        s.supported_source_files,
        s.analyzed_source_files,
        s.unsupported_regular_files,
        s.syntax_error_files
    );
    let _ = writeln!(
        out,
        "source_tree_receipt git_version={} command={} stdout_sha256={} stdout_bytes={}",
        p.git_version, p.ls_tree_command, p.ls_tree_stdout_sha256, p.ls_tree_stdout_bytes
    );
    let _ = writeln!(
        out,
        "source_blob_receipt command={} protocol={} request_sha256={} stdout_sha256={} stdout_bytes={}",
        p.cat_file_command,
        p.cat_file_protocol,
        p.cat_file_request_sha256,
        p.cat_file_stdout_sha256,
        p.cat_file_stdout_bytes
    );
    let _ = writeln!(
        out,
        "join current_analyzed_paths={} sampled_history_paths={} matched_paths={} current_without_history_paths={} historical_without_current_paths={} binary_touched_current_paths={}",
        j.current_analyzed_paths,
        j.sampled_history_paths,
        j.matched_paths,
        j.current_without_history_paths,
        j.historical_without_current_paths,
        j.binary_touched_current_paths
    );
    let _ = writeln!(out, "limitations:");
    for limitation in &report.limitations {
        let _ = writeln!(out, "- {limitation}");
    }

    let mut rows: Vec<_> = report.current_rows.iter().collect();
    rows.sort_by(|a, b| {
        b.line_change_mass
            .cmp(&a.line_change_mass)
            .then_with(|| b.current_cognitive.total_cmp(&a.current_cognitive))
            .then_with(|| a.path_bytes_hex.cmp(&b.path_bytes_hex))
    });
    rows.truncate(top);
    let _ = writeln!(
        out,
        "rows shown={} total={} (sorted by line_change_mass desc, current_cognitive desc, raw path identity asc; no combined rank)",
        rows.len(),
        report.current_rows.len()
    );
    let _ = writeln!(
        out,
        "line_change_mass\tcurrent_cognitive\tcurrent_sloc\tcommits_touched\thistory_status\tlanguage\tpath\tpath_bytes_hex"
    );
    for row in rows {
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            row.line_change_mass,
            finite(row.current_cognitive),
            row.current_sloc,
            row.commits_touched,
            history_status(row.history_status),
            row.language,
            row.path,
            row.path_bytes_hex
        );
    }
    out
}

pub fn render_change_profile_svg(report: &ChangeProfileReport) -> Result<String, String> {
    if report.current_rows.len() > 5_000 {
        return Err(format!(
            "SVG output refuses {} current rows; maximum is 5,000",
            report.current_rows.len()
        ));
    }
    let mut facets: BTreeMap<&str, Vec<&CurrentFileRow>> = BTreeMap::new();
    for row in &report.current_rows {
        facets.entry(&row.language).or_default().push(row);
    }
    if facets.len() > 5 {
        return Err(format!(
            "SVG output supports at most five language facets; report contains {}",
            facets.len()
        ));
    }
    for rows in facets.values_mut() {
        rows.sort_by(|a, b| a.path_bytes_hex.cmp(&b.path_bytes_hex));
    }

    let absolute_x_raw_max = report
        .current_rows
        .iter()
        .map(|r| r.line_change_mass as f64)
        .fold(0.0, f64::max);
    let absolute_y_raw_max = report
        .current_rows
        .iter()
        .map(|r| r.current_cognitive)
        .filter(|v| v.is_finite())
        .fold(0.0, f64::max);
    let normalized_x_raw_max = report
        .current_rows
        .iter()
        .filter_map(|r| r.line_change_mass_per_current_sloc)
        .filter(|v| v.is_finite())
        .fold(0.0, f64::max);
    let normalized_y_raw_max = report
        .current_rows
        .iter()
        .filter_map(|r| r.cognitive_per_ksloc)
        .filter(|v| v.is_finite())
        .fold(0.0, f64::max);
    let absolute_x_max = absolute_x_raw_max.ln_1p().max(1.0);
    let absolute_y_max = absolute_y_raw_max.ln_1p().max(1.0);
    let normalized_x_max = normalized_x_raw_max.ln_1p().max(1.0);
    let normalized_y_max = normalized_y_raw_max.ln_1p().max(1.0);
    let sloc_max = report
        .current_rows
        .iter()
        .map(|r| r.current_sloc)
        .max()
        .unwrap_or(0)
        .max(1) as f64;
    let facet_count = facets.len().max(1);
    let history_only_shown = report.history_only_rows.len().min(20);
    let overflow_lines = usize::from(report.history_only_rows.len() > history_only_shown);
    let ledger_height = 72.0 + 18.0 * (history_only_shown + overflow_lines) as f64;
    let height = 400.0 + FACET_HEIGHT * facet_count as f64 + ledger_height;
    let metadata = canonical_report_json(report)?;

    let mut svg = String::new();
    let _ = write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" role=\"img\" aria-label=\"Change × structure profile\" viewBox=\"0 0 1200 {}\" width=\"1200\" height=\"{}\">",
        fmt(height),
        fmt(height)
    );
    svg.push_str("<title>Change × structure profile</title><desc>Faceted paired planes relate bounded textual change mass to current cognitive complexity without a combined score. Axes use log1p positions with raw-unit ticks. Missing normalized values are explicitly counted. No-history and history-only paths are summarized without sampling the embedded data.</desc>");
    let _ = write!(
        svg,
        "<metadata id=\"change-profile-json\">{}</metadata>",
        xml_text(&metadata)
    );
    svg.push_str("<style>text{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:11px;fill:#17202a}.heading{font-size:18px;font-weight:700}.subheading{font-size:13px;font-weight:700}.axis{stroke:#566573;stroke-width:1;fill:none}.grid{stroke:#d5d8dc;stroke-width:1}.point{fill:#21618c;stroke:#12344d;stroke-width:1.5}.history-binary-only{fill:#fff;stroke:#4a235a;stroke-width:2}.history-text-and-binary{fill:#2471a3;stroke:#4a235a;stroke-width:2}.no-history{fill:#fff;stroke:#566573;stroke-width:1.5}.panel{fill:#f8f9f9;stroke:#aeb6bf}.muted{fill:#566573}.ledger{fill:#f4f6f7}.background{fill:#fff}.extrema{font-size:11px;font-weight:700}</style>");
    let _ = write!(
        svg,
        "<rect class=\"background\" width=\"1200\" height=\"{}\"/>",
        fmt(height)
    );
    svg.push_str("<text class=\"heading\" x=\"32\" y=\"34\">Change × structure profile</text>");
    let h = &report.history_coverage;
    let s = &report.source_coverage;
    let p = &report.source_provenance;
    let j = &report.join_coverage;
    let _ = write!(
        svg,
        "<text x=\"32\" y=\"58\">analyzer: {} · commits: {}/{} · truncated: {} · current analyzed: {} · matched: {} · no history: {} · history only: {}</text>",
        xml_text(&report.analyzer),
        h.commits_analyzed,
        h.requested_commits,
        h.truncated,
        j.current_analyzed_paths,
        j.matched_paths,
        j.current_without_history_paths,
        j.historical_without_current_paths
    );
    let _ = write!(
        svg,
        "<text x=\"32\" y=\"78\">source: tracked {} · UTF-8 paths {} · non-UTF-8 paths {} · supported {} · analyzed {} · unsupported {} · syntax errors {}</text>",
        s.tracked_regular_files,
        s.utf8_path_regular_files,
        s.non_utf8_path_regular_files,
        s.supported_source_files,
        s.analyzed_source_files,
        s.unsupported_regular_files,
        s.syntax_error_files
    );
    let _ = write!(
        svg,
        "<text x=\"32\" y=\"98\">artifact revision: {} · tree: {}</text>",
        xml_text(&report.artifact.revision),
        xml_text(&report.artifact.tree_digest)
    );
    let _ = write!(
        svg,
        "<text x=\"32\" y=\"118\">source tree receipt: {} · {} bytes · SHA-256 {}</text>",
        xml_text(&p.git_version),
        p.ls_tree_stdout_bytes,
        xml_text(&p.ls_tree_stdout_sha256)
    );
    bounded_text(
        &mut svg,
        32.0,
        138.0,
        &format!("source tree command: {}", p.ls_tree_command),
        150,
    );
    bounded_text(
        &mut svg,
        32.0,
        158.0,
        &format!(
            "source blob receipt: request SHA-256 {} · {} bytes · stdout SHA-256 {}",
            p.cat_file_request_sha256, p.cat_file_stdout_bytes, p.cat_file_stdout_sha256
        ),
        150,
    );
    bounded_text(
        &mut svg,
        32.0,
        178.0,
        &format!("source blob command: {}", p.cat_file_command),
        150,
    );
    bounded_text(
        &mut svg,
        32.0,
        198.0,
        &format!("source blob protocol: {}", p.cat_file_protocol),
        150,
    );
    let _ = write!(
        svg,
        "<text x=\"32\" y=\"218\">history receipt: {} · {} bytes · SHA-256 {}</text>",
        xml_text(&h.git_version),
        h.stdout_bytes,
        xml_text(&h.stdout_sha256)
    );
    bounded_text(
        &mut svg,
        32.0,
        238.0,
        &format!("history command: {}", h.command),
        150,
    );
    let _ = write!(
        svg,
        "<text x=\"32\" y=\"258\">committer timestamps: earliest {} · latest {}</text>",
        opt_i64(h.earliest_committer_unix_seconds),
        opt_i64(h.latest_committer_unix_seconds)
    );
    let limitations = report
        .limitations
        .iter()
        .map(|v| v.as_str())
        .collect::<Vec<_>>()
        .join(" · ");
    bounded_text(
        &mut svg,
        32.0,
        278.0,
        &format!("limitations: {limitations}"),
        160,
    );
    bounded_text(
        &mut svg,
        32.0,
        300.0,
        "encoding: circle=text · split circle=text+binary · hollow diamond=binary-only · area=current SLOC (absolute only) · normalized=fixed area · encodings are not rank or quality",
        160,
    );

    if facets.is_empty() {
        svg.push_str("<text x=\"32\" y=\"345\">No supported current source rows.</text>");
    }
    for (facet_index, (language, rows)) in facets.iter().enumerate() {
        let top = 345.0 + facet_index as f64 * FACET_HEIGHT;
        render_plane_frame(
            &mut svg,
            top,
            language,
            "absolute",
            "textual line mass",
            "cognitive total",
            LEFT,
            absolute_x_raw_max,
            absolute_y_raw_max,
        );
        render_plane_frame(
            &mut svg,
            top,
            language,
            "normalized",
            "textual mass / current SLOC",
            "cognitive / kSLOC",
            LEFT + PLANE_WIDTH + PLANE_GAP,
            normalized_x_raw_max,
            normalized_y_raw_max,
        );
        for row in rows {
            let ax = LEFT + scale(row.line_change_mass as f64, absolute_x_max, PLANE_WIDTH);
            let ay = top + PLOT_HEIGHT - scale(row.current_cognitive, absolute_y_max, PLOT_HEIGHT);
            let radius = 3.0 + 9.0 * ((row.current_sloc as f64 / sloc_max).sqrt());
            render_mark(&mut svg, row, ax, ay, radius, "absolute");
            if let (Some(x), Some(y)) = (
                row.line_change_mass_per_current_sloc,
                row.cognitive_per_ksloc,
            ) {
                let nx = LEFT + PLANE_WIDTH + PLANE_GAP + scale(x, normalized_x_max, PLANE_WIDTH);
                let ny = top + PLOT_HEIGHT - scale(y, normalized_y_max, PLOT_HEIGHT);
                render_mark(&mut svg, row, nx, ny, 5.0, "normalized");
            }
        }
        let absolute_plotted = rows
            .iter()
            .filter(|r| !matches!(r.history_status, HistoryStatus::None))
            .count();
        let normalized_plotted = rows
            .iter()
            .filter(|r| {
                !matches!(r.history_status, HistoryStatus::None)
                    && r.line_change_mass_per_current_sloc.is_some()
                    && r.cognitive_per_ksloc.is_some()
            })
            .count();
        let no_history: Vec<_> = rows
            .iter()
            .filter(|r| matches!(r.history_status, HistoryStatus::None))
            .collect();
        let normalization_unavailable = absolute_plotted.saturating_sub(normalized_plotted);
        if absolute_plotted == 0 {
            bounded_text(
                &mut svg,
                LEFT + 20.0,
                top + 78.0,
                &format!(
                    "{} current files; sampled change history unavailable",
                    rows.len()
                ),
                54,
            );
        }
        if normalized_plotted == 0 {
            bounded_text(
                &mut svg,
                LEFT + PLANE_WIDTH + PLANE_GAP + 20.0,
                top + 78.0,
                &format!(
                    "{} current files; no complete normalized change rows",
                    rows.len()
                ),
                54,
            );
        }
        let summary_y = top + PLOT_HEIGHT + 48.0;
        let _ = write!(
            svg,
            "<text x=\"32\" y=\"{}\">plotted: absolute {}/{} current · normalized {}/{} history-matched · no history {} · normalization unavailable {}</text>",
            fmt(summary_y),
            absolute_plotted,
            rows.len(),
            normalized_plotted,
            absolute_plotted,
            no_history.len(),
            normalization_unavailable
        );
        if !no_history.is_empty() {
            let examples = no_history
                .iter()
                .take(3)
                .map(|r| bounded(&r.path, 30))
                .collect::<Vec<_>>()
                .join(" · ");
            bounded_text(
                &mut svg,
                32.0,
                summary_y + 18.0,
                &format!("no-history examples: {examples}; full rows in metadata/JSON"),
                150,
            );
        }
        label_extrema(&mut svg, rows, top);
    }

    let ledger_top = 370.0 + facet_count as f64 * FACET_HEIGHT;
    let mut by_text: Vec<_> = report.history_only_rows.iter().collect();
    by_text.sort_by(|a, b| {
        b.line_change_mass
            .cmp(&a.line_change_mass)
            .then_with(|| a.path_bytes_hex.cmp(&b.path_bytes_hex))
    });
    let mut by_binary = by_text.clone();
    by_binary.sort_by(|a, b| {
        b.binary_change_count
            .cmp(&a.binary_change_count)
            .then_with(|| a.path_bytes_hex.cmp(&b.path_bytes_hex))
    });
    let mut by_raw = by_text.clone();
    by_raw.sort_by(|a, b| a.path_bytes_hex.cmp(&b.path_bytes_hex));
    let mut history_only = Vec::with_capacity(history_only_shown);
    let mut seen = BTreeSet::new();
    for row in by_text
        .iter()
        .take(12)
        .chain(by_binary.iter().take(5))
        .chain(by_raw.iter())
    {
        if seen.insert(row.path_bytes_hex.as_str()) {
            history_only.push(*row);
            if history_only.len() == history_only_shown {
                break;
            }
        }
    }
    let all_shown = report.history_only_rows.len() <= history_only.len();
    let qualifier = if all_shown {
        "all shown"
    } else {
        "12 highest text mass + 5 highest binary-touch candidates + raw-path fill; deduplicated"
    };
    let _ = write!(
        svg,
        "<rect class=\"ledger\" x=\"24\" y=\"{}\" width=\"1152\" height=\"{}\"/><text class=\"subheading\" x=\"32\" y=\"{}\">History-only ledger — {} paths ({})</text>",
        fmt(ledger_top),
        fmt(ledger_height),
        fmt(ledger_top + 22.0),
        report.history_only_rows.len(),
        qualifier
    );
    for (index, row) in history_only.iter().enumerate() {
        let y = ledger_top + 44.0 + index as f64 * 18.0;
        let full = format!(
            "{} · commits={} · days={} · text mass={} · binary changes={} · status={} · raw={}",
            row.path,
            row.commits_touched,
            row.active_change_days,
            row.line_change_mass,
            row.binary_change_count,
            history_status(row.history_status),
            row.path_bytes_hex
        );
        let shown = format!(
            "{} · commits={} · days={} · text mass={} · binary changes={} · status={} · raw={}",
            bounded(&row.path, 48),
            row.commits_touched,
            row.active_change_days,
            row.line_change_mass,
            row.binary_change_count,
            history_status(row.history_status),
            bounded(&row.path_bytes_hex, 24)
        );
        titled_text(&mut svg, 32.0, y, &shown, &full);
    }
    if !all_shown {
        let remaining = report.history_only_rows.len() - history_only.len();
        let y = ledger_top + 44.0 + history_only.len() as f64 * 18.0;
        let _ = write!(
            svg,
            "<text x=\"32\" y=\"{}\">+{} more; full rows in metadata/JSON</text>",
            fmt(y),
            remaining
        );
    }
    svg.push_str("</svg>");
    Ok(svg)
}

fn render_plane_frame(
    out: &mut String,
    top: f64,
    language: &str,
    plane: &str,
    x_label: &str,
    y_label: &str,
    left: f64,
    x_raw_max: f64,
    y_raw_max: f64,
) {
    let _ = write!(
        out,
        "<rect class=\"panel\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/><path class=\"axis\" d=\"M {} {}V {}H {}\"/><text class=\"subheading\" x=\"{}\" y=\"{}\">{} — {}</text>",
        fmt(left),
        fmt(top),
        fmt(PLANE_WIDTH),
        fmt(PLOT_HEIGHT),
        fmt(left),
        fmt(top),
        fmt(top + PLOT_HEIGHT),
        fmt(left + PLANE_WIDTH),
        fmt(left),
        fmt(top - 8.0),
        xml_text(language),
        xml_text(plane)
    );
    let x_domain = x_raw_max.ln_1p().max(1.0);
    let y_domain = y_raw_max.ln_1p().max(1.0);
    for (index, tick) in log_ticks(x_raw_max).into_iter().enumerate() {
        let x = left + scale(tick, x_domain, PLANE_WIDTH);
        if index > 0 && tick < x_raw_max {
            let _ = write!(
                out,
                "<path class=\"grid\" d=\"M {} {}V {}\"/>",
                fmt(x),
                fmt(top),
                fmt(top + PLOT_HEIGHT)
            );
        }
        let _ = write!(
            out,
            "<text class=\"muted\" text-anchor=\"middle\" x=\"{}\" y=\"{}\">{}</text>",
            fmt(x),
            fmt(top + PLOT_HEIGHT + 14.0),
            raw(tick)
        );
    }
    for (index, tick) in log_ticks(y_raw_max).into_iter().enumerate() {
        let y = top + PLOT_HEIGHT - scale(tick, y_domain, PLOT_HEIGHT);
        if index > 0 && tick < y_raw_max {
            let _ = write!(
                out,
                "<path class=\"grid\" d=\"M {} {}H {}\"/>",
                fmt(left),
                fmt(y),
                fmt(left + PLANE_WIDTH)
            );
        }
        let _ = write!(
            out,
            "<text class=\"muted\" text-anchor=\"end\" x=\"{}\" y=\"{}\">{}</text>",
            fmt(left - 6.0),
            fmt(y + 4.0),
            raw(tick)
        );
    }
    let _ = write!(
        out,
        "<text text-anchor=\"middle\" x=\"{}\" y=\"{}\">{} (log1p scale; raw ticks)</text><text text-anchor=\"middle\" transform=\"translate({} {}) rotate(-90)\">{} (log1p scale; raw ticks)</text>",
        fmt(left + PLANE_WIDTH / 2.0),
        fmt(top + PLOT_HEIGHT + 31.0),
        xml_text(x_label),
        fmt(left - 62.0),
        fmt(top + PLOT_HEIGHT / 2.0),
        xml_text(y_label)
    );
}

fn render_mark(out: &mut String, row: &CurrentFileRow, x: f64, y: f64, radius: f64, plane: &str) {
    let label = mark_label(row, plane);
    let class = match row.history_status {
        HistoryStatus::Text => "point history-text",
        HistoryStatus::TextAndBinary => "point history-text-and-binary",
        HistoryStatus::BinaryOnly => "point history-binary-only",
        HistoryStatus::None => return,
    };
    match row.history_status {
        HistoryStatus::BinaryOnly => {
            let _ = write!(
                out,
                "<path class=\"{}\" role=\"img\" aria-label=\"{}\" d=\"M {} {} L {} {} L {} {} L {} {} Z\"><title>{}</title></path>",
                class,
                xml_attr(&label),
                fmt(x),
                fmt(y - radius),
                fmt(x + radius),
                fmt(y),
                fmt(x),
                fmt(y + radius),
                fmt(x - radius),
                fmt(y),
                xml_text(&label)
            );
        }
        HistoryStatus::TextAndBinary => {
            let _ = write!(
                out,
                "<path class=\"{}\" role=\"img\" aria-label=\"{}\" d=\"M {} {} A {} {} 0 1 1 {} {} L {} {} Z\"><title>{}</title></path>",
                class,
                xml_attr(&label),
                fmt(x),
                fmt(y - radius),
                fmt(radius),
                fmt(radius),
                fmt(x - radius),
                fmt(y),
                fmt(x),
                fmt(y),
                xml_text(&label)
            );
        }
        _ => {
            let _ = write!(
                out,
                "<circle class=\"{}\" role=\"img\" aria-label=\"{}\" cx=\"{}\" cy=\"{}\" r=\"{}\"><title>{}</title></circle>",
                class,
                xml_attr(&label),
                fmt(x),
                fmt(y),
                fmt(radius),
                xml_text(&label)
            );
        }
    }
}

fn mark_label(row: &CurrentFileRow, plane: &str) -> String {
    format!(
        "{}; raw path {}; language {}; {} plane; line change mass {}; current SLOC {}; cognitive {}; history {}",
        row.path,
        row.path_bytes_hex,
        row.language,
        plane,
        row.line_change_mass,
        row.current_sloc,
        finite(row.current_cognitive),
        history_status(row.history_status)
    )
}

fn scale(value: f64, domain_log_max: f64, extent: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        value.ln_1p() / domain_log_max * extent
    } else {
        0.0
    }
}

fn history_status(status: HistoryStatus) -> &'static str {
    match status {
        HistoryStatus::Text => "text",
        HistoryStatus::TextAndBinary => "text_and_binary",
        HistoryStatus::BinaryOnly => "binary_only",
        HistoryStatus::None => "none",
    }
}

fn bounded_text(out: &mut String, x: f64, y: f64, full: &str, max_chars: usize) {
    titled_text(out, x, y, &bounded(full, max_chars), full);
}

fn titled_text(out: &mut String, x: f64, y: f64, shown: &str, full: &str) {
    let _ = write!(
        out,
        "<g><title>{}</title><text x=\"{}\" y=\"{}\">{}</text></g>",
        xml_text(full),
        fmt(x),
        fmt(y),
        xml_text(shown)
    );
}

fn bounded(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let mut shown: String = value.chars().take(max_chars.saturating_sub(1)).collect();
    shown.push('…');
    shown
}

fn log_ticks(max: f64) -> Vec<f64> {
    if !max.is_finite() || max <= 0.0 {
        return vec![0.0];
    }
    let domain = max.ln_1p();
    (0..=4)
        .map(|index| (domain * index as f64 / 4.0).exp_m1())
        .collect()
}

fn raw(value: f64) -> String {
    let absolute = value.abs();
    if absolute == 0.0 {
        "0".to_owned()
    } else if absolute >= 100.0 {
        format!("{value:.0}")
    } else if absolute >= 10.0 {
        format!("{value:.1}")
    } else if absolute >= 1.0 {
        format!("{value:.2}")
    } else if absolute >= 0.01 {
        format!("{value:.3}")
    } else {
        format!("{value:.1e}")
    }
}

fn label_extrema(out: &mut String, rows: &[&CurrentFileRow], top: f64) {
    let by_mass = rows
        .iter()
        .copied()
        .filter(|row| !matches!(row.history_status, HistoryStatus::None))
        .max_by(|a, b| {
            a.line_change_mass
                .cmp(&b.line_change_mass)
                .then_with(|| b.path_bytes_hex.cmp(&a.path_bytes_hex))
        });
    let by_cognitive = rows
        .iter()
        .copied()
        .filter(|row| !matches!(row.history_status, HistoryStatus::None))
        .max_by(|a, b| {
            a.current_cognitive
                .total_cmp(&b.current_cognitive)
                .then_with(|| b.path_bytes_hex.cmp(&a.path_bytes_hex))
        });
    if let (Some(mass), Some(cognitive)) = (by_mass, by_cognitive) {
        let full = if mass.path_bytes_hex == cognitive.path_bytes_hex {
            format!(
                "coordinate extrema: textual mass and cognitive total = {}",
                mass.path
            )
        } else {
            format!(
                "coordinate extrema: textual mass = {} · cognitive total = {}",
                mass.path, cognitive.path
            )
        };
        bounded_text(out, 32.0, top + PLOT_HEIGHT + 84.0, &full, 150);
    }
}

fn canonical_report_json(report: &ChangeProfileReport) -> Result<String, String> {
    let mut value = serde_json::to_value(report)
        .map_err(|e| format!("failed to serialize SVG metadata: {e}"))?;
    for field in ["current_rows", "history_only_rows"] {
        if let Some(rows) = value
            .get_mut(field)
            .and_then(serde_json::Value::as_array_mut)
        {
            rows.sort_by(|a, b| {
                a.get("path_bytes_hex")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .cmp(
                        b.get("path_bytes_hex")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or(""),
                    )
            });
        }
    }
    serde_json::to_string(&value).map_err(|e| format!("failed to serialize SVG metadata: {e}"))
}

fn opt_i64(value: Option<i64>) -> String {
    value.map_or_else(|| "null".to_owned(), |v| v.to_string())
}
fn finite(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.6}")
    } else {
        "null".to_owned()
    }
}
fn fmt(value: f64) -> String {
    format!("{value:.3}")
}
fn xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn xml_attr(value: &str) -> String {
    xml_text(value)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
