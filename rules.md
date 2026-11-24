# prsherpa rules

Single source of truth for how prsherpa classifies files, scores a branch, orders files, and estimates review time. Linked from [AGENTS.md](../AGENTS.md). If you change behavior, change this file in the same commit.

Thresholds and penalties below are defaults. In code they must be named constants, not magic numbers.

## File classification

Every changed file gets exactly one class. First match wins, in this order:

1. `lockfile_or_generated`: `*.lock`, `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `Cargo.lock`, `dist/`, `build/`, `*.min.js`, `*.generated.*`
2. `test`: path contains `test`, `tests`, `spec`, or `__tests__`; or filename matches `*_test.*`, `*.test.*`, `*.spec.*`
3. `docs`: `*.md`, `*.rst`, `docs/`
4. `config`: `*.toml`, `*.yaml`, `*.yml`, `*.json`, dotfiles
5. `source`: everything else

`lockfile_or_generated` files are excluded from all size and time calculations.

## Rules

| Rule   | Trigger | Penalty |
|--------|---------|---------|
| SIZE   | changed lines (excluding generated) greater than 400 | -15 |
| SIZE (strong) | changed lines greater than 800; replaces the plain SIZE penalty, never stacks | -30 |
| TESTS  | at least one `source` file changed and no `test` file changed | -20 |
| RISKY  | file path contains `auth`, `login`, `session`, `payment`, `billing`, `migration`, `security`, `secret`, `.env`, or `permission` | -5 per file, capped at -20 |
| SPREAD | more than 15 files changed, or more than 5 distinct top-level directories touched | -10 |

Each issue carries a severity (`warn` or `strong`), a short message, and, where it helps, a suggestion (for example "consider splitting into 3 PRs").

## Score

Start at 100, subtract all penalties, clamp to 0-100. The formula lives in `score.rs` with a comment per penalty.

## Reading order

1. Risky files
2. `source` files, by changed lines, descending
3. `test` files
4. `docs` and `config` files
5. `lockfile_or_generated` files last, marked "skip"

A file that is both risky and a test is listed under risky.

## Review time estimate

About 10 minutes per 100 changed `source` lines. `test` lines count half. Generated lines count zero. Round to the nearest 5 minutes and present it as approximate (for example "~45 min").

## Output

- Terminal: colored report that fits one screen for a typical PR: score, issues, reading order, time estimate.
- `--json`: the same data, stable field names, no colors. Treat field names as public API.
