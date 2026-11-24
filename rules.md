# prsherpa rules

Single source of truth for how prsherpa classifies files, scores a branch, orders files, and estimates review time. Linked from [AGENTS.md](AGENTS.md). If you change behavior, change this file in the same commit.

Thresholds and penalties below are defaults (overridable via `.prsherpa.toml`). In code they must be named constants, not magic numbers.

## File classification

Every changed file gets exactly one class. First match wins, in this order:

1. `lockfile_or_generated`: `*.lock`, `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `Cargo.lock`, `dist/`, `build/`, `*.min.js`, `*.generated.*`
2. `test`: path contains `test`, `tests`, `spec`, or `__tests__`; or filename matches `*_test.*`, `*.test.*`, `*.spec.*`
3. `docs`: `*.md`, `*.rst`, `docs/`
4. `config`: `*.toml`, `*.yaml`, `*.yml`, `*.json`, dotfiles
5. `source`: everything else

`lockfile_or_generated`, `docs`, and pure renames (`R100`) are excluded from `SIZE` and review time calculations.

## Rules

| Rule | Trigger | Penalty |
|------|---------|---------|
| SIZE | weighted changed lines (`source`/`config` 1x, `test` 0.5x; excluding `docs`, `lockfile_or_generated`, pure renames) > 400 | -15 |
| SIZE (strong) | weighted changed lines > 800; replaces plain SIZE, never stacks | -30 |
| TESTS | at least one `source` file changed and no `test` file changed | -20 |
| RISKY | file path contains a risky keyword (`auth`, `login`, `session`, `payment`, `billing`, `migration`, `security`, `secret`, `.env`, `permission`) | -5 per file, capped at -20 |
| SPREAD | > 15 files changed or > 5 distinct top-level directories touched | -10 |
| MIXED_CONCERNS | dependency manifest (`Cargo.toml`, `package.json`, `go.mod`, `pyproject.toml`) changed alongside `source`, OR >150 additions and >150 deletions across 5+ `source` files | -10 |
| BINARY_OR_HUGE_FILE | non-`docs` binary file added/modified, OR single non-generated file > 300 changed lines | -10 |
| DEBUG_LEFTOVER | added lines (`+`) in non-test `source` code contain `dbg!`, `todo!`, `unimplemented!`, `console.log`, `pdb.set_trace`, `binding.pry` | -5 |

## Score

Start at 100, subtract all penalties, clamp to 0–100. The formula lives in `src/score.rs`.

## Reading order

1. Risky files
2. `source` files
3. `test` files
4. `docs` and `config` files
5. `lockfile_or_generated` files last, marked `[skip]` (collapsed into one summary line if total files > 20)

Within buckets 1–3, files are sorted by `hunk_count + changed_lines` descending. Pure renames are labeled `[moved]`, binary files `[binary]`.

## Review time estimate

About 10 minutes per 100 changed `source` lines. `test` lines count half. Pure renames, generated, docs, and config lines count zero. Round to the nearest 5 minutes.

## Output

- `--format terminal` (default): colored report for humans.
- `--format json` (or `--json`): machine-readable JSON.
- `--format markdown`: concise GitHub PR comment with `<details>` reading guide.
