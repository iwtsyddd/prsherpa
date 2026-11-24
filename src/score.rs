use crate::rules::Issue;

/// Reviewability score formula (0..=100):
/// 1. Start at 100 points.
/// 2. Subtract SIZE penalty: -15 if non-generated changed lines > 400, or -30 if > 800 (never stacks).
/// 3. Subtract TESTS penalty: -20 if source files changed without any test file changes.
/// 4. Subtract RISKY penalty: -5 per risky file (auth, login, session, payment, billing,
///    migration, security, secret, .env, permission), capped at -20.
/// 5. Subtract SPREAD penalty: -10 if > 15 files changed or > 5 top-level directories touched.
/// 6. Clamp the final score to 0..=100.
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
        assert_eq!(compute_score(&[issue(15), issue(20)]), 65);
        assert_eq!(
            compute_score(&[issue(30), issue(20), issue(20), issue(40)]),
            0
        );
    }
}
