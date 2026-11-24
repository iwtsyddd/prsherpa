# prsherpa

`prsherpa` checks a git branch **before** you open a pull request. It prints a reviewability score (0–100), flags common reviewability issues, suggests a reading order for the reviewer, and estimates review time.

It is offline, deterministic, and calls the `git` CLI directly.

## Install

```bash
cargo install --path .
```

Or build from source:

```bash
cargo build --release
```

## Usage

Run inside a git repository on your feature branch:

```bash
prsherpa
```

### Options

```text
prsherpa [--base <branch>] [--json] [--fail-under <N>]
```

- `--base <branch>`: Base branch to compare against (defaults to `origin/main`, `origin/master`, `main`, or `master`).
- `--json`: Output the report as JSON.
- `--fail-under <N>`: Exit with code `1` if the score is below `N` (0–100).

### Exit codes

- `0`: Success
- `1`: Score is below `--fail-under`
- `2`: Operational error (not a git repo, no changes, base branch not found)
