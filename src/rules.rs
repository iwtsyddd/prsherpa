use crate::config::{matches_ignore_pattern, Config};
use crate::git::{ChangeKind, DiffFile};
use serde::Serialize;
use std::collections::HashSet;

pub const SIZE_WARN_PENALTY: u8 = 15;
pub const SIZE_STRONG_PENALTY: u8 = 30;
pub const TESTS_PENALTY: u8 = 20;
pub const RISKY_PER_FILE_PENALTY: u8 = 5;
pub const RISKY_MAX_PENALTY: u8 = 20;
pub const SPREAD_PENALTY: u8 = 10;
pub const MIXED_CONCERNS_PENALTY: u8 = 10;
pub const MIXED_CONCERNS_MIN_SOURCE_FILES: usize = 5;
pub const MIXED_CONCERNS_LINE_THRESHOLD: u64 = 150;
pub const BINARY_OR_HUGE_PENALTY: u8 = 10;
pub const HUGE_FILE_LINES: u64 = 300;
pub const DEBUG_LEFTOVER_PENALTY: u8 = 5;

const LOCKFILE_NAMES: &[&str] = &[
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "cargo.lock",
];

const DEPENDENCY_MANIFESTS: &[&str] = &["cargo.toml", "package.json", "go.mod", "pyproject.toml"];

const DEBUG_PATTERNS: &[&str] = &[
    "dbg!",
    "todo!",
    "unimplemented!",
    "console.log",
    "pdb.set_trace",
    "binding.pry",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileClass {
    Source,
    Test,
    LockfileOrGenerated,
    Docs,
    Config,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Warn,
    Strong,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub rule: String,
    pub severity: Severity,
    pub penalty: u8,
    pub message: String,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClassifiedFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeKind,
    pub class: FileClass,
    pub risky: bool,
    pub additions: u64,
    pub deletions: u64,
    pub is_binary: bool,
    pub is_pure_rename: bool,
    pub hunk_count: u64,
}

impl ClassifiedFile {
    pub fn changed_lines(&self) -> u64 {
        if self.is_pure_rename {
            0
        } else {
            self.additions + self.deletions
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadingItem {
    pub path: String,
    pub class: FileClass,
    pub status: ChangeKind,
    pub additions: u64,
    pub deletions: u64,
    pub hunk_count: u64,
    pub is_binary: bool,
    pub is_pure_rename: bool,
    pub risky: bool,
    pub skip: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Analysis {
    pub files: Vec<ClassifiedFile>,
    pub issues: Vec<Issue>,
    pub reading_order: Vec<ReadingItem>,
    pub estimated_review_minutes: u64,
    pub non_generated_changed_lines: u64,
    pub total_additions: u64,
    pub total_deletions: u64,
}

/// Classifies a file path into one of the five `FileClass` variants.
/// First match wins: lockfile/generated -> test -> docs -> config -> source.
pub fn classify_file(path: &str) -> FileClass {
    let normalized = path.replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    let segments: Vec<&str> = lower.split('/').filter(|s| !s.is_empty()).collect();
    let filename = segments.last().copied().unwrap_or("");

    if is_lockfile_or_generated(&segments, filename) {
        return FileClass::LockfileOrGenerated;
    }
    if is_test_file(&segments, filename) {
        return FileClass::Test;
    }
    if is_docs_file(&segments, filename) {
        return FileClass::Docs;
    }
    if is_config_file(filename) {
        return FileClass::Config;
    }
    FileClass::Source
}

fn is_lockfile_or_generated(segments: &[&str], filename: &str) -> bool {
    if LOCKFILE_NAMES.contains(&filename) {
        return true;
    }
    if filename.ends_with(".lock")
        || filename.ends_with(".min.js")
        || filename.contains(".generated.")
    {
        return true;
    }
    segments
        .iter()
        .take(segments.len().saturating_sub(1))
        .any(|seg| *seg == "dist" || *seg == "build")
}

fn is_test_file(segments: &[&str], filename: &str) -> bool {
    if segments
        .iter()
        .any(|seg| matches!(*seg, "test" | "tests" | "spec" | "__tests__"))
    {
        return true;
    }
    filename.contains("_test.") || filename.contains(".test.") || filename.contains(".spec.")
}

fn is_docs_file(segments: &[&str], filename: &str) -> bool {
    if filename.ends_with(".md") || filename.ends_with(".rst") {
        return true;
    }
    segments
        .iter()
        .take(segments.len().saturating_sub(1))
        .any(|seg| *seg == "docs")
}

fn is_config_file(filename: &str) -> bool {
    filename.ends_with(".toml")
        || filename.ends_with(".yaml")
        || filename.ends_with(".yml")
        || filename.ends_with(".json")
        || filename.starts_with('.')
}

pub fn is_risky_path(path: &str, keywords: &[String]) -> bool {
    let lower = path.replace('\\', "/").to_ascii_lowercase();
    keywords
        .iter()
        .any(|kw| lower.contains(&kw.to_ascii_lowercase()))
}

fn top_level_dir(path: &str) -> &str {
    let normalized = path.trim_start_matches("./");
    match normalized.split('/').next() {
        Some(first) if normalized.contains('/') => first,
        _ => ".",
    }
}

fn filename_lower(path: &str) -> String {
    path.replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

pub fn analyze(diff_files: &[DiffFile], config: &Config) -> Analysis {
    let active_diffs: Vec<&DiffFile> = diff_files
        .iter()
        .filter(|f| !matches_ignore_pattern(&f.path, &config.ignore_patterns))
        .collect();

    let files: Vec<ClassifiedFile> = active_diffs
        .iter()
        .map(|f| ClassifiedFile {
            path: f.path.clone(),
            old_path: f.old_path.clone(),
            status: f.status,
            class: classify_file(&f.path),
            risky: is_risky_path(&f.path, &config.risky_keywords),
            additions: f.additions,
            deletions: f.deletions,
            is_binary: f.is_binary,
            is_pure_rename: f.is_pure_rename,
            hunk_count: f.hunk_count,
        })
        .collect();

    let total_additions: u64 = files
        .iter()
        .filter(|f| !f.is_pure_rename)
        .map(|f| f.additions)
        .sum();
    let total_deletions: u64 = files
        .iter()
        .filter(|f| !f.is_pure_rename)
        .map(|f| f.deletions)
        .sum();
    let weighted_size_lines = compute_weighted_size_lines(&files);

    let issues = evaluate_rules(&active_diffs, &files, weighted_size_lines, config);
    let reading_order = build_reading_order(&files);
    let estimated_review_minutes = estimate_review_minutes(&files);

    Analysis {
        files,
        issues,
        reading_order,
        estimated_review_minutes,
        non_generated_changed_lines: weighted_size_lines,
        total_additions,
        total_deletions,
    }
}

fn compute_weighted_size_lines(files: &[ClassifiedFile]) -> u64 {
    let mut weighted: f64 = 0.0;
    for f in files {
        if f.is_pure_rename {
            continue;
        }
        match f.class {
            FileClass::LockfileOrGenerated | FileClass::Docs => {}
            FileClass::Test => weighted += (f.changed_lines() as f64) * 0.5,
            FileClass::Source | FileClass::Config => weighted += f.changed_lines() as f64,
        }
    }
    weighted.round() as u64
}

fn evaluate_rules(
    raw_diffs: &[&DiffFile],
    files: &[ClassifiedFile],
    weighted_lines: u64,
    config: &Config,
) -> Vec<Issue> {
    let mut issues = Vec::new();

    if weighted_lines > config.size_strong_warn {
        issues.push(Issue {
            rule: "SIZE".to_string(),
            severity: Severity::Strong,
            penalty: SIZE_STRONG_PENALTY,
            message: format!(
                "{weighted_lines} weighted changed lines exceeds {}.",
                config.size_strong_warn
            ),
            suggestion: Some("Consider splitting into smaller PRs.".to_string()),
        });
    } else if weighted_lines > config.size_warn {
        issues.push(Issue {
            rule: "SIZE".to_string(),
            severity: Severity::Warn,
            penalty: SIZE_WARN_PENALTY,
            message: format!(
                "{weighted_lines} weighted changed lines exceeds {}.",
                config.size_warn
            ),
            suggestion: Some("Consider splitting into smaller PRs.".to_string()),
        });
    }

    let source_files: Vec<&ClassifiedFile> = files
        .iter()
        .filter(|f| f.class == FileClass::Source)
        .collect();
    let has_test = files.iter().any(|f| f.class == FileClass::Test);
    if !source_files.is_empty() && !has_test {
        issues.push(Issue {
            rule: "TESTS".to_string(),
            severity: Severity::Warn,
            penalty: TESTS_PENALTY,
            message: "Source files changed without any test file changes.".to_string(),
            suggestion: Some("Add or update tests covering the new behavior.".to_string()),
        });
    }

    let risky_paths: Vec<&str> = files
        .iter()
        .filter(|f| f.risky)
        .map(|f| f.path.as_str())
        .collect();
    if !risky_paths.is_empty() {
        let penalty = ((risky_paths.len() as u8).saturating_mul(RISKY_PER_FILE_PENALTY))
            .min(RISKY_MAX_PENALTY);
        issues.push(Issue {
            rule: "RISKY".to_string(),
            severity: Severity::Warn,
            penalty,
            message: format!("Needs careful review: {}", risky_paths.join(", ")),
            suggestion: Some("Request a focused review from a domain owner.".to_string()),
        });
    }

    let top_dirs: HashSet<&str> = files.iter().map(|f| top_level_dir(&f.path)).collect();
    if files.len() > config.max_files || top_dirs.len() > config.max_dirs {
        issues.push(Issue {
            rule: "SPREAD".to_string(),
            severity: Severity::Warn,
            penalty: SPREAD_PENALTY,
            message: format!(
                "{} files changed across {} top-level directories.",
                files.len(),
                top_dirs.len()
            ),
            suggestion: Some("Consider splitting the PR by feature or directory.".to_string()),
        });
    }

    if let Some(issue) = check_mixed_concerns(files, &source_files) {
        issues.push(issue);
    }

    if let Some(issue) = check_binary_or_huge_file(files) {
        issues.push(issue);
    }

    if let Some(issue) = check_debug_leftover(raw_diffs, files) {
        issues.push(issue);
    }

    issues
}

fn check_mixed_concerns(
    files: &[ClassifiedFile],
    source_files: &[&ClassifiedFile],
) -> Option<Issue> {
    if source_files.is_empty() {
        return None;
    }

    let manifests: Vec<&str> = files
        .iter()
        .filter(|f| DEPENDENCY_MANIFESTS.contains(&filename_lower(&f.path).as_str()))
        .map(|f| f.path.as_str())
        .collect();

    if !manifests.is_empty() {
        return Some(Issue {
            rule: "MIXED_CONCERNS".to_string(),
            severity: Severity::Warn,
            penalty: MIXED_CONCERNS_PENALTY,
            message: format!(
                "Dependency manifest ({}) changed alongside source files.",
                manifests.join(", ")
            ),
            suggestion: Some(
                "Separate dependency updates from feature/bugfix changes.".to_string(),
            ),
        });
    }

    let src_add: u64 = source_files
        .iter()
        .filter(|f| !f.is_pure_rename)
        .map(|f| f.additions)
        .sum();
    let src_del: u64 = source_files
        .iter()
        .filter(|f| !f.is_pure_rename)
        .map(|f| f.deletions)
        .sum();

    if source_files.len() >= MIXED_CONCERNS_MIN_SOURCE_FILES
        && src_add > MIXED_CONCERNS_LINE_THRESHOLD
        && src_del > MIXED_CONCERNS_LINE_THRESHOLD
    {
        return Some(Issue {
            rule: "MIXED_CONCERNS".to_string(),
            severity: Severity::Warn,
            penalty: MIXED_CONCERNS_PENALTY,
            message: format!(
                "+{src_add}/-{src_del} across {} source files suggests mixed refactor and feature work.",
                source_files.len()
            ),
            suggestion: Some("Split refactoring into a preparatory PR.".to_string()),
        });
    }

    None
}

fn check_binary_or_huge_file(files: &[ClassifiedFile]) -> Option<Issue> {
    let mut flagged = Vec::new();
    for f in files {
        let binary_hit =
            f.is_binary && f.status != ChangeKind::Deleted && f.class != FileClass::Docs;
        let huge_hit = f.class != FileClass::LockfileOrGenerated
            && !f.is_pure_rename
            && f.changed_lines() > HUGE_FILE_LINES;
        if binary_hit {
            flagged.push(format!("{} (binary)", f.path));
        } else if huge_hit {
            flagged.push(format!("{} ({} lines)", f.path, f.changed_lines()));
        }
    }

    if flagged.is_empty() {
        return None;
    }

    Some(Issue {
        rule: "BINARY_OR_HUGE_FILE".to_string(),
        severity: Severity::Warn,
        penalty: BINARY_OR_HUGE_PENALTY,
        message: format!("Binary or oversized file(s): {}", flagged.join(", ")),
        suggestion: Some("Verify binary assets are intentional or split large files.".to_string()),
    })
}

fn check_debug_leftover(raw_diffs: &[&DiffFile], files: &[ClassifiedFile]) -> Option<Issue> {
    let mut hits = Vec::new();
    for (raw, classified) in raw_diffs.iter().zip(files.iter()) {
        if classified.class != FileClass::Source {
            continue;
        }
        let is_rust = classified.path.ends_with(".rs");
        let mut matched = Vec::new();
        for line in &raw.added_lines {
            if is_rust && line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            let code_only = strip_comments_and_quotes(line, is_rust);
            for pat in DEBUG_PATTERNS {
                if code_only.contains(pat) && !matched.contains(pat) {
                    matched.push(*pat);
                }
            }
        }
        if !matched.is_empty() {
            hits.push(format!("{} ({})", classified.path, matched.join(", ")));
        }
    }

    if hits.is_empty() {
        return None;
    }

    Some(Issue {
        rule: "DEBUG_LEFTOVER".to_string(),
        severity: Severity::Warn,
        penalty: DEBUG_LEFTOVER_PENALTY,
        message: format!("Debug/placeholder statements added: {}", hits.join("; ")),
        suggestion: Some(
            "Remove leftover debug prints or todo!() before opening the PR.".to_string(),
        ),
    })
}

fn strip_comments_and_quotes(line: &str, is_rust: bool) -> String {
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut quote: Option<char> = None;

    while i < chars.len() {
        let ch = chars[i];
        if let Some(q) = quote {
            if ch == '\\' {
                i += 2;
                continue;
            }
            if ch == q {
                quote = None;
            }
            i += 1;
            continue;
        }

        if ch == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            break;
        }
        if ch == '"' || (!is_rust && ch == '\'') {
            quote = Some(ch);
            out.push(' ');
            i += 1;
            continue;
        }
        out.push(ch);
        i += 1;
    }

    out
}

fn reading_bucket(file: &ClassifiedFile) -> u8 {
    if file.risky {
        return 0;
    }
    match file.class {
        FileClass::Source => 1,
        FileClass::Test => 2,
        FileClass::Docs | FileClass::Config => 3,
        FileClass::LockfileOrGenerated => 4,
    }
}

pub fn build_reading_order(files: &[ClassifiedFile]) -> Vec<ReadingItem> {
    let mut ordered: Vec<&ClassifiedFile> = files.iter().collect();
    ordered.sort_by(|a, b| {
        let bucket_a = reading_bucket(a);
        let bucket_b = reading_bucket(b);
        bucket_a
            .cmp(&bucket_b)
            .then_with(|| match bucket_a {
                0..=2 => {
                    let score_a = a.hunk_count + a.changed_lines();
                    let score_b = b.hunk_count + b.changed_lines();
                    score_b.cmp(&score_a)
                }
                _ => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.path.cmp(&b.path))
    });

    ordered
        .into_iter()
        .map(|f| {
            let skip = !f.risky && f.class == FileClass::LockfileOrGenerated;
            let note = if f.is_pure_rename {
                Some("moved".to_string())
            } else if f.is_binary {
                Some("binary".to_string())
            } else if f.risky {
                Some("needs careful review".to_string())
            } else if skip {
                Some("skip".to_string())
            } else {
                None
            };
            ReadingItem {
                path: f.path.clone(),
                class: f.class,
                status: f.status,
                additions: if f.is_pure_rename { 0 } else { f.additions },
                deletions: if f.is_pure_rename { 0 } else { f.deletions },
                hunk_count: f.hunk_count,
                is_binary: f.is_binary,
                is_pure_rename: f.is_pure_rename,
                risky: f.risky,
                skip,
                note,
            }
        })
        .collect()
}

/// Estimates review time in minutes: ~10 min per 100 source lines, tests count half,
/// pure renames/generated/docs/config count zero, rounded to the nearest 5 minutes.
pub fn estimate_review_minutes(files: &[ClassifiedFile]) -> u64 {
    let source_lines: u64 = files
        .iter()
        .filter(|f| f.class == FileClass::Source && !f.is_pure_rename)
        .map(ClassifiedFile::changed_lines)
        .sum();
    let test_lines: u64 = files
        .iter()
        .filter(|f| f.class == FileClass::Test && !f.is_pure_rename)
        .map(ClassifiedFile::changed_lines)
        .sum();

    let raw_minutes = (source_lines as f64) * 0.1 + (test_lines as f64) * 0.05;
    ((raw_minutes / 5.0).round() as u64) * 5
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(path: &str, additions: u64, deletions: u64) -> DiffFile {
        DiffFile {
            path: path.to_string(),
            old_path: None,
            additions,
            deletions,
            is_binary: false,
            is_pure_rename: false,
            hunk_count: 1,
            added_lines: Vec::new(),
            status: ChangeKind::Modified,
        }
    }

    #[test]
    fn size_excludes_docs_generated_and_pure_renames_and_halves_tests() {
        let mut moved = diff("src/moved.rs", 200, 200);
        moved.status = ChangeKind::Renamed;
        moved.is_pure_rename = true;

        let files = vec![
            diff("Cargo.lock", 2000, 0),
            diff("README.md", 500, 0),
            moved,
            diff("src/lib.rs", 350, 0),
            diff("tests/lib_test.rs", 200, 0), // weighted 100 -> total 450 > 400
        ];
        let analysis = analyze(&files, &Config::default());
        assert_eq!(analysis.non_generated_changed_lines, 450);
        assert!(analysis
            .issues
            .iter()
            .any(|i| i.rule == "SIZE" && i.severity == Severity::Warn));
    }

    #[test]
    fn mixed_concerns_triggers_on_manifest_or_refactor_plus_feature() {
        let files = vec![
            diff("Cargo.toml", 2, 1),
            diff("src/lib.rs", 10, 0),
            diff("tests/lib_test.rs", 5, 0),
        ];
        let analysis = analyze(&files, &Config::default());
        assert!(analysis.issues.iter().any(|i| i.rule == "MIXED_CONCERNS"));

        let refactor_files = vec![
            diff("src/a.rs", 40, 40),
            diff("src/b.rs", 40, 40),
            diff("src/c.rs", 40, 40),
            diff("src/d.rs", 40, 40),
            diff("src/e.rs", 40, 40),
            diff("tests/e_test.rs", 10, 0),
        ];
        let refactor = analyze(&refactor_files, &Config::default());
        assert!(refactor.issues.iter().any(|i| i.rule == "MIXED_CONCERNS"));
    }

    #[test]
    fn binary_or_huge_file_warns_appropriately() {
        let mut bin = diff("assets/app.exe", 0, 0);
        bin.is_binary = true;
        bin.status = ChangeKind::Added;

        let mut doc_img = diff("docs/diagram.png", 0, 0);
        doc_img.is_binary = true;
        doc_img.status = ChangeKind::Added;

        let files = vec![bin, doc_img, diff("tests/t_test.rs", 1, 0)];
        let analysis = analyze(&files, &Config::default());
        let issue = analysis
            .issues
            .iter()
            .find(|i| i.rule == "BINARY_OR_HUGE_FILE")
            .unwrap();
        assert!(issue.message.contains("assets/app.exe"));
        assert!(!issue.message.contains("docs/diagram.png"));
    }

    #[test]
    fn debug_leftover_scans_only_source_added_lines() {
        let mut src = diff("src/main.rs", 2, 0);
        src.added_lines = vec!["dbg!(user_id);".to_string(), "todo!()".to_string()];

        let mut test_file = diff("tests/main_test.rs", 2, 0);
        test_file.added_lines = vec!["dbg!(test_val);".to_string()];

        let analysis = analyze(&[src, test_file], &Config::default());
        let issue = analysis
            .issues
            .iter()
            .find(|i| i.rule == "DEBUG_LEFTOVER")
            .unwrap();
        assert!(issue.message.contains("src/main.rs (dbg!, todo!)"));
        assert!(!issue.message.contains("tests/main_test.rs"));
    }

    #[test]
    fn debug_leftover_ignores_cfg_test_quotes_comments_and_println() {
        let mut src = diff("src/lib.rs", 6, 0);
        src.added_lines = vec![
            "println!(\"normal cli output\");".to_string(),
            "let marker = \"dbg!\";".to_string(),
            "// dbg!(in_comment);".to_string(),
            "#[cfg(test)]".to_string(),
            "mod tests { fn t() { dbg!(123); } }".to_string(),
        ];

        let analysis = analyze(&[src, diff("tests/t_test.rs", 1, 0)], &Config::default());
        assert!(!analysis.issues.iter().any(|i| i.rule == "DEBUG_LEFTOVER"));
    }

    #[test]
    fn reading_order_ranks_fragmented_edits_by_hunks_plus_lines() {
        let mut single_block = diff("src/big_block.rs", 20, 0);
        single_block.hunk_count = 1; // 21

        let mut fragmented = diff("src/fragmented.rs", 15, 0);
        fragmented.hunk_count = 10; // 25 -> should rank before single_block

        let analysis = analyze(&[single_block, fragmented], &Config::default());
        assert_eq!(analysis.reading_order[0].path, "src/fragmented.rs");
        assert_eq!(analysis.reading_order[1].path, "src/big_block.rs");
    }
}
