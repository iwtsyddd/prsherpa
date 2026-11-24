use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
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

    let entries = parse_numstat(&numstat_out)?;
    let statuses = parse_name_status(&name_status_out);
    let files = merge_diff_info(entries, &statuses);

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

/// Parses `git diff --name-status` output into a map from path to `ChangeKind`.
pub fn parse_name_status(output: &str) -> HashMap<String, ChangeKind> {
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
        let path = match kind {
            ChangeKind::Renamed | ChangeKind::Copied if cols.len() >= 3 => cols[2],
            _ => cols[1],
        };
        map.insert(path.to_string(), kind);
    }
    map
}

pub fn merge_diff_info(
    entries: Vec<NumstatEntry>,
    statuses: &HashMap<String, ChangeKind>,
) -> Vec<DiffFile> {
    entries
        .into_iter()
        .map(|entry| {
            let status = statuses.get(&entry.path).copied().unwrap_or_else(|| {
                if entry.old_path.is_some() {
                    ChangeKind::Renamed
                } else {
                    ChangeKind::Modified
                }
            });
            DiffFile {
                path: entry.path,
                old_path: entry.old_path,
                additions: entry.additions,
                deletions: entry.deletions,
                is_binary: entry.is_binary,
                status,
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
    fn parses_rename_paths() {
        assert_eq!(
            parse_rename_path("src/{old.rs => new.rs}"),
            (Some("src/old.rs".to_string()), "src/new.rs".to_string())
        );
        assert_eq!(
            parse_rename_path("src/{ => nested}/mod.rs"),
            (
                Some("src/mod.rs".to_string()),
                "src/nested/mod.rs".to_string()
            )
        );
        assert_eq!(
            parse_rename_path("a.txt => b.txt"),
            (Some("a.txt".to_string()), "b.txt".to_string())
        );
    }

    #[test]
    fn merges_name_status_with_numstat() {
        let numstat = "10\t0\tsrc/new.rs\n0\t20\tsrc/dead.rs\n5\t5\tsrc/{a.rs => b.rs}\n";
        let status = "A\tsrc/new.rs\nD\tsrc/dead.rs\nR100\tsrc/a.rs\tsrc/b.rs\n";

        let entries = parse_numstat(numstat).unwrap();
        let status_map = parse_name_status(status);
        let merged = merge_diff_info(entries, &status_map);

        assert_eq!(merged[0].status, ChangeKind::Added);
        assert_eq!(merged[1].status, ChangeKind::Deleted);
        assert_eq!(merged[2].status, ChangeKind::Renamed);
        assert_eq!(merged[2].old_path.as_deref(), Some("src/a.rs"));
        assert_eq!(merged[2].path, "src/b.rs");
    }
}
