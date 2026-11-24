use crate::rules::{Analysis, FileClass, Issue, ReadingItem, Severity};
use anyhow::Result;
use owo_colors::OwoColorize;
use serde::Serialize;
use std::fmt::Write;

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
        "Files: {} | Lines: +{} / -{} ({} non-generated)",
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
    for (idx, item) in report.reading_order.iter().enumerate() {
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

    out
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
