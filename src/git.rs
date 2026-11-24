use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_BASE_CANDIDATES: &[&str] = &["origin/main", "origin/master", "main", "master"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffFile {
    pub path: String,
    pub old_path: Option<String>,
    pub additions: u64,
    pub deletions: u64,
    pub is_binary: bool,
    pub is_pure_rename: bool,
    pub hunk_count: u64,
    #[serde(skip)]
    pub added_lines: Vec<String>,
    pub status: ChangeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumstatEntry {
    pub path: String,
    pub old_path: Option<String>,
    pub additions: u64,
    pub deletions: u64,
    pub is_binary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusInfo {
    pub kind: ChangeKind,
    pub is_pure_rename: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnifiedFileDiff {
    pub hunk_count: u64,
    pub added_lines: Vec<String>,
}

fn run_git(args: &[&str], dir: Option<&Path>) -> Result<std::process::Output> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    cmd.output()
        .context("Failed to execute `git`. Ensure git is installed and on PATH.")
}

fn run_git_stdout(args: &[&str], dir: Option<&Path>) -> Result<String> {
    let output = run_git(args, dir)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn ensure_git_repo(dir: Option<&Path>) -> Result<()> {
    let output = run_git(&["rev-parse", "--is-inside-work-tree"], dir)?;
    if !output.status.success() {
        bail!("Not a git repository. Run prsherpa inside a git repository.");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim() != "true" {
        bail!("Not a git repository work tree. Run prsherpa inside a git repository.");
    }
    Ok(())
}

pub fn repo_root(dir: Option<&Path>) -> Result<PathBuf> {
    ensure_git_repo(dir)?;
    let out = run_git_stdout(&["rev-parse", "--show-toplevel"], dir)?;
    Ok(PathBuf::from(out.trim()))
}

fn ref_exists(reference: &str, dir: Option<&Path>) -> Result<bool> {
    let target = format!("{reference}^{{commit}}");
    let output = run_git(&["rev-parse", "--verify", "--quiet", &target], dir)?;
    Ok(output.status.success())
}

pub fn detect_base_branch(explicit: Option<&str>, dir: Option<&Path>) -> Result<String> {
    if let Some(branch) = explicit {
        if ref_exists(branch, dir)? {
            return Ok(branch.to_string());
        }
        bail!(
            "Base branch '{branch}' not found. Pass an existing branch or commit with --base <branch>."
        );
    }

    for candidate in DEFAULT_BASE_CANDIDATES {
        if ref_exists(candidate, dir)? {
            return Ok((*candidate).to_string());
        }
    }

    bail!(
        "Could not detect a base branch (tried origin/main, origin/master, main, master). Specify one with --base <branch>."
    );
}

pub fn merge_base(base: &str, dir: Option<&Path>) -> Result<String> {
    if !ref_exists("HEAD", dir)? {
        bail!("No commits found on current branch (HEAD is unborn). Commit at least once before running prsherpa.");
    }
    let out = run_git_stdout(&["merge-base", "HEAD", base], dir)
        .with_context(|| format!("Could not find merge-base between HEAD and '{base}'."))?;
    Ok(out.trim().to_string())
}

pub fn collect_diff(base: Option<&str>, dir: Option<&Path>) -> Result<(String, Vec<DiffFile>)> {
    ensure_git_repo(dir)?;
    let base_branch = detect_base_branch(base, dir)?;
    let mb = merge_base(&base_branch, dir)?;

    let numstat_out = run_git_stdout(&["diff", "--numstat", "-M", &mb], dir)?;
    let name_status_out = run_git_stdout(&["diff", "--name-status", "-M", &mb], dir)?;
    let unified_out = run_git_stdout(&["diff", "-U0", "-M", &mb], dir)?;

    let entries = parse_numstat(&numstat_out)?;
    let statuses = parse_name_status(&name_status_out);
    let unified = parse_unified_diff(&unified_out);
    let files = merge_diff_info(entries, &statuses, &unified);

    Ok((base_branch, files))
}

/// Parses `git diff --numstat` output into structured entries.
pub fn parse_numstat(output: &str) -> Result<Vec<NumstatEntry>> {
    let mut entries = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim_end_matches('\r');
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.splitn(3, '\t');
        let add_str = parts
            .next()
            .context("Malformed numstat line: missing additions column")?;
        let del_str = parts
            .next()
            .context("Malformed numstat line: missing deletions column")?;
        let raw_path = parts
            .next()
            .context("Malformed numstat line: missing path column")?;

        let (is_binary, additions, deletions) = if add_str == "-" || del_str == "-" {
            (true, 0, 0)
        } else {
            let add: u64 = add_str
                .parse()
                .with_context(|| format!("Invalid additions count in numstat: '{add_str}'"))?;
            let del: u64 = del_str
                .parse()
                .with_context(|| format!("Invalid deletions count in numstat: '{del_str}'"))?;
            (false, add, del)
        };

        let (old_path, path) = parse_rename_path(raw_path);
        entries.push(NumstatEntry {
            path,
            old_path,
            additions,
            deletions,
            is_binary,
        });
    }
    Ok(entries)
}

/// Parses a path from `git diff --numstat`, handling rename syntax such as
/// `src/{old.rs => new.rs}` or `old.rs => new.rs`.
pub fn parse_rename_path(raw: &str) -> (Option<String>, String) {
    if let (Some(open), Some(close)) = (raw.find('{'), raw.rfind('}')) {
        if open < close {
            let inner = &raw[open + 1..close];
            if let Some(arrow) = inner.find(" => ") {
                let prefix = &raw[..open];
                let suffix = &raw[close + 1..];
                let old_mid = &inner[..arrow];
                let new_mid = &inner[arrow + 4..];

                let old_joined = clean_joined_path(prefix, old_mid, suffix);
                let new_joined = clean_joined_path(prefix, new_mid, suffix);
                return (Some(old_joined), new_joined);
            }
        }
    }

    if let Some(arrow) = raw.find(" => ") {
        let old = raw[..arrow].trim().to_string();
        let new = raw[arrow + 4..].trim().to_string();
        return (Some(old), new);
    }

    (None, raw.to_string())
}

fn clean_joined_path(prefix: &str, middle: &str, suffix: &str) -> String {
    let joined = format!("{prefix}{middle}{suffix}");
    let mut cleaned = joined.replace("//", "/");
    if cleaned.starts_with('/') && !prefix.starts_with('/') {
        cleaned.remove(0);
    }
    cleaned
}

/// Parses `git diff --name-status -M` output into a map from path to `StatusInfo`.
pub fn parse_name_status(output: &str) -> HashMap<String, StatusInfo> {
    let mut map = HashMap::new();
    for line in output.lines() {
        let trimmed = line.trim_end_matches('\r');
        if trimmed.is_empty() {
            continue;
        }
        let cols: Vec<&str> = trimmed.split('\t').collect();
        if cols.len() < 2 {
            continue;
        }
        let code = cols[0];
        let kind = match code.chars().next() {
            Some('A') => ChangeKind::Added,
            Some('M') => ChangeKind::Modified,
            Some('D') => ChangeKind::Deleted,
            Some('R') => ChangeKind::Renamed,
            Some('C') => ChangeKind::Copied,
            _ => ChangeKind::Unknown,
        };
        let is_pure_rename = kind == ChangeKind::Renamed && code == "R100";
        let path = match kind {
            ChangeKind::Renamed | ChangeKind::Copied if cols.len() >= 3 => cols[2],
            _ => cols[1],
        };
        map.insert(
            path.to_string(),
            StatusInfo {
                kind,
                is_pure_rename,
            },
        );
    }
    map
}

/// Parses `git diff -U0 -M` output to count `@@` hunks and collect `+` added lines per file.
pub fn parse_unified_diff(output: &str) -> HashMap<String, UnifiedFileDiff> {
    let mut map: HashMap<String, UnifiedFileDiff> = HashMap::new();
    let mut current_path: Option<String> = None;

    for raw_line in output.lines() {
        let line = raw_line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("diff --git ") {
            current_path = extract_b_path(rest);
            if let Some(path) = &current_path {
                map.entry(path.clone()).or_default();
            }
            continue;
        }
        if let Some(path) = line.strip_prefix("+++ b/") {
            current_path = Some(path.to_string());
            map.entry(path.to_string()).or_default();
            continue;
        }
        if line == "+++ /dev/null" {
            continue;
        }
        if line.starts_with("--- ") {
            continue;
        }
        if line.starts_with("@@ ") {
            if let Some(path) = &current_path {
                map.entry(path.clone()).or_default().hunk_count += 1;
            }
            continue;
        }
        if let Some(added) = line.strip_prefix('+') {
            if let Some(path) = &current_path {
                map.entry(path.clone())
                    .or_default()
                    .added_lines
                    .push(added.to_string());
            }
        }
    }

    map
}

fn extract_b_path(diff_git_rest: &str) -> Option<String> {
    let idx = diff_git_rest.rfind(" b/")?;
    Some(diff_git_rest[idx + 3..].to_string())
}

pub fn merge_diff_info(
    entries: Vec<NumstatEntry>,
    statuses: &HashMap<String, StatusInfo>,
    unified: &HashMap<String, UnifiedFileDiff>,
) -> Vec<DiffFile> {
    entries
        .into_iter()
        .map(|entry| {
            let status_info = statuses.get(&entry.path).copied().unwrap_or(StatusInfo {
                kind: if entry.old_path.is_some() {
                    ChangeKind::Renamed
                } else {
                    ChangeKind::Modified
                },
                is_pure_rename: entry.old_path.is_some()
                    && entry.additions == 0
                    && entry.deletions == 0,
            });
            let u = unified.get(&entry.path).cloned().unwrap_or_default();
            DiffFile {
                path: entry.path,
                old_path: entry.old_path,
                additions: entry.additions,
                deletions: entry.deletions,
                is_binary: entry.is_binary,
                is_pure_rename: status_info.is_pure_rename,
                hunk_count: u.hunk_count,
                added_lines: u.added_lines,
                status: status_info.kind,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_and_binary_numstat() {
        let raw = "12\t3\tsrc/main.rs\n-\t-\tassets/logo.png\n0\t45\told.txt\n";
        let parsed = parse_numstat(raw).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].path, "src/main.rs");
        assert_eq!(parsed[0].additions, 12);
        assert_eq!(parsed[0].deletions, 3);
        assert!(!parsed[0].is_binary);

        assert_eq!(parsed[1].path, "assets/logo.png");
        assert!(parsed[1].is_binary);
        assert_eq!(parsed[1].additions, 0);
        assert_eq!(parsed[1].deletions, 0);

        assert_eq!(parsed[2].path, "old.txt");
        assert_eq!(parsed[2].deletions, 45);
    }

    #[test]
    fn parses_rename_paths_and_pure_rename_status() {
        assert_eq!(
            parse_rename_path("src/{old.rs => new.rs}"),
            (Some("src/old.rs".to_string()), "src/new.rs".to_string())
        );
        let numstat = "0\t0\tsrc/{old.rs => moved.rs}\n3\t2\tsrc/{a.rs => b.rs}\n";
        let status = "R100\tsrc/old.rs\tsrc/moved.rs\nR085\tsrc/a.rs\tsrc/b.rs\n";
        let merged = merge_diff_info(
            parse_numstat(numstat).unwrap(),
            &parse_name_status(status),
            &HashMap::new(),
        );
        assert_eq!(merged[0].status, ChangeKind::Renamed);
        assert!(merged[0].is_pure_rename);
        assert_eq!(merged[1].status, ChangeKind::Renamed);
        assert!(!merged[1].is_pure_rename);
    }

    #[test]
    fn parses_unified_diff_hunks_and_added_lines() {
        let diff = concat!(
            "diff --git a/src/lib.rs b/src/lib.rs\n",
            "index 111..222 100644\n",
            "--- a/src/lib.rs\n",
            "+++ b/src/lib.rs\n",
            "@@ -1,0 +2,2 @@\n",
            "+let x = 1;\n",
            "+dbg!(x);\n",
            "@@ -10,1 +12,1 @@\n",
            "-old();\n",
            "+new();\n",
        );
        let parsed = parse_unified_diff(diff);
        let file = parsed.get("src/lib.rs").unwrap();
        assert_eq!(file.hunk_count, 2);
        assert_eq!(file.added_lines, vec!["let x = 1;", "dbg!(x);", "new();"]);
    }
}
