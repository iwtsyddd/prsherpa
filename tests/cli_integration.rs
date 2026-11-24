use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "prsherpa_it_{}_{}_{}",
            std::process::id(),
            label,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        Self { path: dir }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("failed to run git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo(dir: &Path) {
    git(dir, &["init", "-b", "main"]);
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    fs::write(dir.join("README.md"), "# demo\n").unwrap();
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/lib.rs"), "pub fn hello() {}\n").unwrap();
    fs::write(dir.join("src/old.rs"), "pub fn old() {}\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "initial"]);
    git(dir, &["checkout", "-b", "feature/test"]);
}

fn run_prsherpa(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_prsherpa"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("failed to execute prsherpa")
}

#[test]
fn fails_gracefully_outside_git_repo() {
    let tmp = TempDir::new("not_git");
    let out = run_prsherpa(&tmp.path, &[]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Not a git repository"));
}

#[test]
fn fails_gracefully_on_empty_diff() {
    let tmp = TempDir::new("empty_diff");
    init_repo(&tmp.path);
    let out = run_prsherpa(&tmp.path, &["--base", "main"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("No changes found"));
}

#[test]
fn fails_gracefully_when_base_branch_missing() {
    let tmp = TempDir::new("missing_base");
    init_repo(&tmp.path);
    let out = run_prsherpa(&tmp.path, &["--base", "does-not-exist"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not found"));
}

#[test]
fn handles_binary_renames_deletions_and_uncommitted_changes() {
    let tmp = TempDir::new("complex_diff");
    init_repo(&tmp.path);

    // Rename src/old.rs -> src/renamed.rs and delete README.md in a commit
    git(&tmp.path, &["mv", "src/old.rs", "src/renamed.rs"]);
    git(&tmp.path, &["rm", "README.md"]);
    fs::write(tmp.path.join("logo.bin"), [0_u8, 159, 146, 150, 0, 1, 2]).unwrap();
    git(&tmp.path, &["add", "."]);
    git(&tmp.path, &["commit", "-m", "rename delete binary"]);

    // Uncommitted change to auth + test
    fs::write(
        tmp.path.join("src/auth.rs"),
        "pub fn login() -> bool { true }\n",
    )
    .unwrap();
    git(&tmp.path, &["add", "src/auth.rs"]);
    fs::create_dir_all(tmp.path.join("tests")).unwrap();
    fs::write(tmp.path.join("tests/auth_test.rs"), "#[test]\nfn ok() {}\n").unwrap();
    git(&tmp.path, &["add", "tests/auth_test.rs"]);

    let out = run_prsherpa(&tmp.path, &["--base", "main", "--json"]);
    assert_eq!(out.status.code(), Some(0));

    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["base_branch"], "main");
    let reading = json["reading_order"].as_array().unwrap();
    assert!(reading.iter().any(|r| r["path"] == "src/renamed.rs"));
    assert!(reading.iter().any(|r| r["path"] == "README.md"));
    assert!(reading.iter().any(|r| r["path"] == "logo.bin"));
    assert!(reading
        .iter()
        .any(|r| r["path"] == "src/auth.rs" && r["risky"] == true));
}

#[test]
fn lockfile_only_changes_score_100_and_fail_under_enforces_exit_1() {
    let tmp = TempDir::new("lockfile_and_fail_under");
    init_repo(&tmp.path);

    // Lockfile-only change
    fs::write(
        tmp.path.join("Cargo.lock"),
        "# huge lockfile\n".repeat(1000),
    )
    .unwrap();
    git(&tmp.path, &["add", "Cargo.lock"]);

    let out = run_prsherpa(&tmp.path, &["--base", "main", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["score"], 100);
    assert_eq!(json["summary"]["non_generated_changed_lines"], 0);
    assert_eq!(json["estimated_review_minutes"], 0);

    // Add source file without tests -> score drops to 80 (-20 TESTS)
    fs::write(
        tmp.path.join("src/lib.rs"),
        "pub fn hello() { println!(\"hi\"); }\n",
    )
    .unwrap();
    let fail_out = run_prsherpa(&tmp.path, &["--base", "main", "--fail-under", "90"]);
    assert_eq!(fail_out.status.code(), Some(1));
}
