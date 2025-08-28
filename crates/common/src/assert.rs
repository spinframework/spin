//! Assertion macros.

/// Asserts that the expression matches the pattern.
///
/// This is equivalent to `assert!(matches!(...))` except that it produces nicer
/// errors.
#[macro_export]
macro_rules! assert_matches {
    ($expr:expr, $pat:pat $(,)?) => {{
        let val = $expr;
        assert!(
            matches!(val, $pat),
            "expected {val:?} to match {}",
            stringify!($pat),
        )
    }};
}

/// Asserts that the expression does not match the pattern.
///
/// This is equivalent to `assert!(!matches!(...))` except that it produces
/// nicer errors.
#[macro_export]
macro_rules! assert_not_matches {
    ($expr:expr, $pat:pat $(,)?) => {{
        let val = $expr;
        assert!(
            !matches!(val, $pat),
            "expected {val:?} to NOT match {}",
            stringify!($pat),
        )
    }};
}
