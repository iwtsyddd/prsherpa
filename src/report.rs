use crate::rules::{Analysis, FileClass, Issue, ReadingItem, Severity};
use anyhow::Result;
use owo_colors::OwoColorize;
use serde::Serialize;
use std::fmt::Write;

const COLLAPSE_GENERATED_THRESHOLD: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub total_files: usize,
    pub total_additions: u64,
    pub total_deletions: u64,
    pub non_generated_changed_lines: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub base_branch: String,
    pub score: u8,
    pub estimated_review_minutes: u64,
    pub summary: Summary,
    pub issues: Vec<Issue>,
    pub reading_order: Vec<ReadingItem>,
}

impl Report {
    pub fn from_analysis(base_branch: String, score: u8, analysis: Analysis) -> Self {
        Self {
            base_branch,
            score,
            estimated_review_minutes: analysis.estimated_review_minutes,
            summary: Summary {
                total_files: analysis.files.len(),
                total_additions: analysis.total_additions,
                total_deletions: analysis.total_deletions,
                non_generated_changed_lines: analysis.non_generated_changed_lines,
            },
            issues: analysis.issues,
            reading_order: analysis.reading_order,
        }
    }
}

pub fn render_json(report: &Report) -> Result<String> {
    Ok(serde_json::to_string_pretty(report)?)
}

pub fn render_markdown(report: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "## prsherpa — Reviewability Score: {}/100",
        report.score
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "**Base:** `{}` | **Est. review:** ~{} min | **Files:** {} | **Lines:** +{}/-{} ({} weighted)",
        report.base_branch,
        report.estimated_review_minutes,
        report.summary.total_files,
        report.summary.total_additions,
        report.summary.total_deletions,
        report.summary.non_generated_changed_lines
    );
    let _ = writeln!(out);

    if report.issues.is_empty() {
        let _ = writeln!(out, "No reviewability issues found.");
    } else {
        let _ = writeln!(out, "### Issues");
        for issue in &report.issues {
            let badge = match issue.severity {
                Severity::Warn => "WARN",
                Severity::Strong => "STRONG",
            };
            let _ = writeln!(
                out,
                "- **[{badge}] `{}` (-{})**: {}",
                issue.rule, issue.penalty, issue.message
            );
            if let Some(suggestion) = &issue.suggestion {
                let _ = writeln!(out, "  - *Suggestion:* {suggestion}");
            }
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "<details>");
    let _ = writeln!(
        out,
        "<summary>Reading guide ({} files)</summary>",
        report.reading_order.len()
    );
    let _ = writeln!(out);

    let (visible, collapsed_count, collapsed_add, collapsed_del) =
        partition_reading_order(&report.reading_order, report.summary.total_files);

    for (idx, item) in visible.iter().enumerate() {
        let class_label = format_class(item.class);
        let note = item
            .note
            .as_deref()
            .map(|n| format!(" `[{n}]`"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{}. `{}` ({}, +{}/-{}){}",
            idx + 1,
            item.path,
            class_label,
            item.additions,
            item.deletions,
            note
        );
    }

    if collapsed_count > 0 {
        let _ = writeln!(
            out,
            "{}. *+{} lockfile/generated file(s) (+{}/-{})* `[skip]`",
            visible.len() + 1,
            collapsed_count,
            collapsed_add,
            collapsed_del
        );
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "</details>");
    out
}

pub fn render_terminal(report: &Report, use_color: bool) -> String {
    let mut out = String::new();

    let header = format!(
        "prsherpa (base: {}) — Score: {}/100 | Est. review: ~{} min",
        report.base_branch, report.score, report.estimated_review_minutes
    );
    if use_color {
        let score_str = format!("{}/100", report.score);
        let colored_score = if report.score >= 80 {
            score_str.green().bold().to_string()
        } else if report.score >= 50 {
            score_str.yellow().bold().to_string()
        } else {
            score_str.red().bold().to_string()
        };
        let _ = writeln!(
            out,
            "{} (base: {}) — Score: {} | Est. review: ~{} min",
            "prsherpa".bold(),
            report.base_branch.cyan(),
            colored_score,
            report.estimated_review_minutes
        );
    } else {
        let _ = writeln!(out, "{header}");
    }

    let _ = writeln!(
        out,
        "Files: {} | Lines: +{} / -{} ({} weighted)",
        report.summary.total_files,
        report.summary.total_additions,
        report.summary.total_deletions,
        report.summary.non_generated_changed_lines
    );

    let _ = writeln!(out);
    if report.issues.is_empty() {
        let msg = "Issues: none — looks easy to review.";
        if use_color {
            let _ = writeln!(out, "{}", msg.green());
        } else {
            let _ = writeln!(out, "{msg}");
        }
    } else {
        let _ = writeln!(out, "Issues:");
        for issue in &report.issues {
            let tag = match issue.severity {
                Severity::Warn => "WARN",
                Severity::Strong => "STRONG",
            };
            let line = format!(
                "  [{tag}] {} (-{}): {}",
                issue.rule, issue.penalty, issue.message
            );
            if use_color {
                match issue.severity {
                    Severity::Warn => {
                        let _ = writeln!(out, "{}", line.yellow());
                    }
                    Severity::Strong => {
                        let _ = writeln!(out, "{}", line.red().bold());
                    }
                }
            } else {
                let _ = writeln!(out, "{line}");
            }
            if let Some(suggestion) = &issue.suggestion {
                let _ = writeln!(out, "         -> {suggestion}");
            }
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "Reading order:");
    let (visible, collapsed_count, collapsed_add, collapsed_del) =
        partition_reading_order(&report.reading_order, report.summary.total_files);

    for (idx, item) in visible.iter().enumerate() {
        let class_label = format_class(item.class);
        let stats = format!("+{}/-{}", item.additions, item.deletions);
        let note = item
            .note
            .as_deref()
            .map(|n| format!(" [{n}]"))
            .unwrap_or_default();
        let line = format!(
            "  {:>2}. {} ({}, {}){}",
            idx + 1,
            item.path,
            class_label,
            stats,
            note
        );
        if use_color {
            if item.risky {
                let _ = writeln!(out, "{}", line.red());
            } else if item.skip {
                let _ = writeln!(out, "{}", line.dimmed());
            } else {
                let _ = writeln!(out, "{line}");
            }
        } else {
            let _ = writeln!(out, "{line}");
        }
    }

    if collapsed_count > 0 {
        let summary_line = format!(
            "  ... +{collapsed_count} lockfile/generated file(s) (+{collapsed_add}/-{collapsed_del}) [skip]"
        );
        if use_color {
            let _ = writeln!(out, "{}", summary_line.dimmed());
        } else {
            let _ = writeln!(out, "{summary_line}");
        }
    }

    out
}

fn partition_reading_order(
    items: &[ReadingItem],
    total_files: usize,
) -> (Vec<&ReadingItem>, usize, u64, u64) {
    if total_files <= COLLAPSE_GENERATED_THRESHOLD {
        return (items.iter().collect(), 0, 0, 0);
    }

    let mut visible = Vec::new();
    let mut collapsed_count = 0usize;
    let mut collapsed_add = 0u64;
    let mut collapsed_del = 0u64;

    for item in items {
        if item.skip && item.class == FileClass::LockfileOrGenerated {
            collapsed_count += 1;
            collapsed_add += item.additions;
            collapsed_del += item.deletions;
        } else {
            visible.push(item);
        }
    }

    (visible, collapsed_count, collapsed_add, collapsed_del)
}

fn format_class(class: FileClass) -> &'static str {
    match class {
        FileClass::Source => "source",
        FileClass::Test => "test",
        FileClass::LockfileOrGenerated => "generated",
        FileClass::Docs => "docs",
        FileClass::Config => "config",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::ChangeKind;

    fn sample_report(total_files: usize) -> Report {
        let mut items = Vec::new();
        for i in 0..total_files.saturating_sub(2) {
            items.push(ReadingItem {
                path: format!("src/f{i}.rs"),
                class: FileClass::Source,
                status: ChangeKind::Modified,
                additions: 5,
                deletions: 1,
                hunk_count: 1,
                is_binary: false,
                is_pure_rename: false,
                risky: false,
                skip: false,
                note: None,
            });
        }
        items.push(ReadingItem {
            path: "Cargo.lock".to_string(),
            class: FileClass::LockfileOrGenerated,
            status: ChangeKind::Modified,
            additions: 100,
            deletions: 20,
            hunk_count: 1,
            is_binary: false,
            is_pure_rename: false,
            risky: false,
            skip: true,
            note: Some("skip".to_string()),
        });
        items.push(ReadingItem {
            path: "package-lock.json".to_string(),
            class: FileClass::LockfileOrGenerated,
            status: ChangeKind::Modified,
            additions: 50,
            deletions: 10,
            hunk_count: 1,
            is_binary: false,
            is_pure_rename: false,
            risky: false,
            skip: true,
            note: Some("skip".to_string()),
        });

        Report {
            base_branch: "main".to_string(),
            score: 85,
            estimated_review_minutes: 15,
            summary: Summary {
                total_files: items.len(),
                total_additions: 200,
                total_deletions: 40,
                non_generated_changed_lines: 120,
            },
            issues: vec![Issue {
                rule: "DEBUG_LEFTOVER".to_string(),
                severity: Severity::Warn,
                penalty: 5,
                message: "Debug statement in src/f0.rs".to_string(),
                suggestion: Some("Remove dbg!()".to_string()),
            }],
            reading_order: items,
        }
    }

    #[test]
    fn markdown_includes_score_issues_and_details() {
        let md = render_markdown(&sample_report(4));
        assert!(md.contains("Reviewability Score: 85/100"));
        assert!(md.contains("**[WARN] `DEBUG_LEFTOVER` (-5)**"));
        assert!(md.contains("<details>"));
        assert!(md.contains("<summary>Reading guide (4 files)</summary>"));
        assert!(md.contains("</details>"));
    }

    #[test]
    fn collapses_lockfiles_when_total_files_exceeds_20() {
        let report = sample_report(22);
        let term = render_terminal(&report, false);
        assert!(term.contains("+2 lockfile/generated file(s) (+150/-30) [skip]"));
        assert!(!term.contains("Cargo.lock"));
    }
}
