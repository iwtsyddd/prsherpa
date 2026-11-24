use crate::rules::Issue;

/// Reviewability score formula (0..=100):
/// 1. Start at 100 points and subtract penalties, clamped to 0..=100.
/// 2. SIZE: -15 (> 400 weighted lines) or -30 (> 800), excluding docs, generated, pure renames; tests 0.5x.
/// 3. TESTS: -20 if source files changed without test changes.
/// 4. RISKY: -5 per risky file (capped at -20).
/// 5. SPREAD: -10 if > max_files or > max_dirs.
/// 6. MIXED_CONCERNS: -10 if manifests + source changed or >150/+150 across 5+ source files.
/// 7. BINARY_OR_HUGE_FILE: -10 if non-docs binary added/modified or single file > 300 lines.
/// 8. DEBUG_LEFTOVER: -5 if added source lines contain dbg!/todo!/unimplemented!/console.log/pdb.set_trace/binding.pry.
pub const BASE_SCORE: i32 = 100;

pub fn compute_score(issues: &[Issue]) -> u8 {
    let total_penalty: i32 = issues.iter().map(|i| i32::from(i.penalty)).sum();
    (BASE_SCORE - total_penalty).clamp(0, 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Severity;

    fn issue(penalty: u8) -> Issue {
        Issue {
            rule: "TEST".to_string(),
            severity: Severity::Warn,
            penalty,
            message: "msg".to_string(),
            suggestion: None,
        }
    }

    #[test]
    fn computes_and_clamps_score() {
        assert_eq!(compute_score(&[]), 100);
        assert_eq!(compute_score(&[issue(15), issue(20), issue(10)]), 55);
        assert_eq!(
            compute_score(&[issue(30), issue(20), issue(20), issue(40)]),
            0
        );
    }
}
