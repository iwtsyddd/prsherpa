# AGENTS.md

`prsherpa` is a Rust CLI that checks a git branch **before** a pull request is opened. It prints a reviewability score, a list of issues, a reading order for the reviewer, and an estimated review time. It is offline and deterministic; the core has no LLM.

Current phase: **MVP** (parse diff -> rules -> score -> report). Do not start `--explain` (opt-in LLM) or the GitHub Action unless asked.

## Commands

```bash
cargo build
cargo run -- --base main                      # try it on a real repo
cargo test                                    # must pass
cargo clippy --all-targets -- -D warnings     # must be clean
cargo fmt                                     # run before finishing
```

Run fmt, clippy, and test before saying a task is done. Never claim tests pass without running them. Style is enforced by rustfmt and clippy; do not add style rules by hand.

## Stack constraints

- Call the `git` binary via `std::process::Command`. **Do not add `git2`.**
- Allowed crates: `clap` (derive), `anyhow`, `owo-colors`, `serde`, `serde_json`. Ask before adding any other.
- No async, no network, no telemetry.
- Prefer simple code: `String` and `.clone()` over lifetimes. This is a CLI, not a hot path.
- No `unwrap()` / `expect()` outside tests.

## Architecture rule

Keep pure logic (diff parsing, file classification, rules, scoring) separate from I/O (running git, printing). Pure logic must be unit-testable without a git repo.

## Behavior contract

- Flags and exit codes are public API: `0` ok, `1` score below `--fail-under`, `2` operational error.
- Same diff in, same report out. No randomness, no timestamps in output.
- Rules, penalties, thresholds, reading order, and the time estimate are defined in [docs/rules.md](docs/rules.md). Read it before touching rules or scoring, and update it in the same change if behavior changes.

## Testing

- Unit tests for parsing, classification, each rule, scoring. Integration tests build a temporary git repo.
- Always cover: empty diff, binary files, renames, deletions, lockfile-only changes, not inside a git repo.
- A bug fix comes with a test that failed before the fix.

## Gotchas

- `git diff --numstat` prints `-` instead of numbers for binary files.
- Renames show up as `old => new` in numstat; parse them explicitly.
- Measure against `git merge-base` and include uncommitted changes, not just commits.
- Respect `NO_COLOR`; no colors when stdout is not a terminal.
- User-facing errors must say what to do next (e.g. "Not a git repository. Run prsherpa inside a repo.").

## Boundaries

- **Always:** keep it fast and offline; update docs/rules.md when rules change.
- **Ask first:** new dependency; changing the score formula or thresholds; changing flags or exit codes.
- **Never:** send code, diffs, or file names over the network; commit secrets, tokens, or keys; add features outside the current phase.

## Git workflow

Small, focused commits with imperative messages (e.g. "Add SPREAD rule"). Keep PRs small: this tool preaches reviewability, so run `cargo run` on your own branch before finishing.
