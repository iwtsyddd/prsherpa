use crate::git::{ChangeKind, DiffFile};
use serde::Serialize;
use std::collections::HashSet;

pub const SIZE_WARN_LINES: u64 = 400;
pub const SIZE_STRONG_LINES: u64 = 800;
pub const SIZE_WARN_PENALTY: u8 = 15;
pub const SIZE_STRONG_PENALTY: u8 = 30;

pub const TESTS_PENALTY: u8 = 20;

pub const RISKY_PER_FILE_PENALTY: u8 = 5;
pub const RISKY_MAX_PENALTY: u8 = 20;

pub const SPREAD_MAX_FILES: usize = 15;
pub const SPREAD_MAX_DIRS: usize = 5;
pub const SPREAD_PENALTY: u8 = 10;

const LOCKFILE_NAMES: &[&str] = &[
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "cargo.lock",
];

const RISKY_KEYWORDS: &[&str] = &[
    "auth",
    "login",
    "session",
    "payment",
    "billing",
    "migration",
    "security",
    "secret",
    ".env",
    "permission",
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
}

impl ClassifiedFile {
    pub fn changed_lines(&self) -> u64 {
        self.additions + self.deletions
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadingItem {
    pub path: String,
    pub class: FileClass,
    pub status: ChangeKind,
    pub additions: u64,
    pub deletions: u64,
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
    has_test_filename_pattern(filename)
}

fn has_test_filename_pattern(filename: &str) -> bool {
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

pub fn is_risky_path(path: &str) -> bool {
    let lower = path.replace('\\', "/").to_ascii_lowercase();
    RISKY_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

fn top_level_dir(path: &str) -> &str {
    let normalized = path.trim_start_matches("./");
    match normalized.split('/').next() {
        Some(first) if normalized.contains('/') => first,
        _ => ".",
    }
}

pub fn analyze(diff_files: &[DiffFile]) -> Analysis {
    let files: Vec<ClassifiedFile> = diff_files
        .iter()
        .map(|f| ClassifiedFile {
            path: f.path.clone(),
            old_path: f.old_path.clone(),
            status: f.status,
            class: classify_file(&f.path),
            risky: is_risky_path(&f.path),
            additions: f.additions,
            deletions: f.deletions,
            is_binary: f.is_binary,
        })
        .collect();

    let total_additions: u64 = files.iter().map(|f| f.additions).sum();
    let total_deletions: u64 = files.iter().map(|f| f.deletions).sum();
    let non_generated_changed_lines: u64 = files
        .iter()
        .filter(|f| f.class != FileClass::LockfileOrGenerated)
        .map(ClassifiedFile::changed_lines)
        .sum();

    let issues = evaluate_rules(&files, non_generated_changed_lines);
    let reading_order = build_reading_order(&files);
    let estimated_review_minutes = estimate_review_minutes(&files);

    Analysis {
        files,
        issues,
        reading_order,
        estimated_review_minutes,
        non_generated_changed_lines,
        total_additions,
        total_deletions,
    }
}

fn evaluate_rules(files: &[ClassifiedFile], non_generated_lines: u64) -> Vec<Issue> {
    let mut issues = Vec::new();

    if non_generated_lines > SIZE_STRONG_LINES {
        issues.push(Issue {
            rule: "SIZE".to_string(),
            severity: Severity::Strong,
            penalty: SIZE_STRONG_PENALTY,
            message: format!(
                "{non_generated_lines} changed lines (excluding generated) exceeds {SIZE_STRONG_LINES}."
            ),
            suggestion: Some("Consider splitting into smaller PRs.".to_string()),
        });
    } else if non_generated_lines > SIZE_WARN_LINES {
        issues.push(Issue {
            rule: "SIZE".to_string(),
            severity: Severity::Warn,
            penalty: SIZE_WARN_PENALTY,
            message: format!(
                "{non_generated_lines} changed lines (excluding generated) exceeds {SIZE_WARN_LINES}."
            ),
            suggestion: Some("Consider splitting into smaller PRs.".to_string()),
        });
    }

    let has_source = files.iter().any(|f| f.class == FileClass::Source);
    let has_test = files.iter().any(|f| f.class == FileClass::Test);
    if has_source && !has_test {
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
    if files.len() > SPREAD_MAX_FILES || top_dirs.len() > SPREAD_MAX_DIRS {
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

    issues
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
                0..=2 => b.changed_lines().cmp(&a.changed_lines()),
                _ => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.path.cmp(&b.path))
    });

    ordered
        .into_iter()
        .map(|f| {
            let skip = !f.risky && f.class == FileClass::LockfileOrGenerated;
            let note = if f.risky {
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
                additions: f.additions,
                deletions: f.deletions,
                risky: f.risky,
                skip,
                note,
            }
        })
        .collect()
}

/// Estimates review time in minutes: ~10 min per 100 source lines, tests count half,
/// generated/docs/config count zero, rounded to the nearest 5 minutes.
pub fn estimate_review_minutes(files: &[ClassifiedFile]) -> u64 {
    let source_lines: u64 = files
        .iter()
        .filter(|f| f.class == FileClass::Source)
        .map(ClassifiedFile::changed_lines)
        .sum();
    let test_lines: u64 = files
        .iter()
        .filter(|f| f.class == FileClass::Test)
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
            status: ChangeKind::Modified,
        }
    }

    #[test]
    fn classifies_all_file_types() {
        assert_eq!(classify_file("Cargo.lock"), FileClass::LockfileOrGenerated);
        assert_eq!(
            classify_file("web/dist/app.min.js"),
            FileClass::LockfileOrGenerated
        );
        assert_eq!(
            classify_file("src/api.generated.ts"),
            FileClass::LockfileOrGenerated
        );
        assert_eq!(classify_file("tests/integration.rs"), FileClass::Test);
        assert_eq!(classify_file("src/parser_test.go"), FileClass::Test);
        assert_eq!(classify_file("src/button.spec.tsx"), FileClass::Test);
        assert_eq!(classify_file("README.md"), FileClass::Docs);
        assert_eq!(classify_file("docs/architecture.rst"), FileClass::Docs);
        assert_eq!(classify_file("Cargo.toml"), FileClass::Config);
        assert_eq!(classify_file(".github/workflows/ci.yml"), FileClass::Config);
        assert_eq!(classify_file(".gitignore"), FileClass::Config);
        assert_eq!(classify_file("src/main.rs"), FileClass::Source);
    }

    #[test]
    fn size_rule_does_not_stack_and_excludes_generated() {
        let files = vec![
            diff("Cargo.lock", 2000, 500),
            diff("src/lib.rs", 450, 0),
            diff("tests/lib_test.rs", 10, 0),
        ];
        let analysis = analyze(&files);
        assert_eq!(analysis.non_generated_changed_lines, 460);
        assert_eq!(analysis.issues.len(), 1);
        assert_eq!(analysis.issues[0].rule, "SIZE");
        assert_eq!(analysis.issues[0].severity, Severity::Warn);
        assert_eq!(analysis.issues[0].penalty, 15);

        let large = vec![diff("src/lib.rs", 900, 0), diff("tests/lib_test.rs", 10, 0)];
        let large_analysis = analyze(&large);
        assert_eq!(large_analysis.issues.len(), 1);
        assert_eq!(large_analysis.issues[0].severity, Severity::Strong);
        assert_eq!(large_analysis.issues[0].penalty, 30);
    }

    #[test]
    fn reading_order_puts_risky_first_and_lockfiles_last() {
        let files = vec![
            diff("Cargo.lock", 500, 0),
            diff("README.md", 20, 0),
            diff("src/small.rs", 10, 0),
            diff("src/big.rs", 100, 0),
            diff("tests/auth_test.rs", 15, 0),
            diff("tests/unit_test.rs", 30, 0),
        ];
        let analysis = analyze(&files);
        let paths: Vec<&str> = analysis
            .reading_order
            .iter()
            .map(|r| r.path.as_str())
            .collect();
        assert_eq!(
            paths,
            vec![
                "tests/auth_test.rs",
                "src/big.rs",
                "src/small.rs",
                "tests/unit_test.rs",
                "README.md",
                "Cargo.lock",
            ]
        );
        assert!(analysis.reading_order[0].risky);
        assert!(analysis.reading_order[5].skip);
    }

    #[test]
    fn estimates_review_time_rounded_to_five_minutes() {
        let files = vec![
            diff("src/a.rs", 120, 30),       // 150 source -> 15 min
            diff("tests/a_test.rs", 100, 0), // 100 test -> 5 min
            diff("Cargo.lock", 5000, 0),     // 0 min
        ];
        let analysis = analyze(&files);
        assert_eq!(analysis.estimated_review_minutes, 20);
    }
}
