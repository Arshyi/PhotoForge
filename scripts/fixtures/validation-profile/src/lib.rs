#[cfg(test)]
mod tests {
    #[test]
    fn compact_validation_keeps_debug_assertions() {
        // This fixture is intentionally tested in the dev/test profile, never release.
        assert!(cfg!(debug_assertions));
        assert!(std::panic::catch_unwind(|| debug_assert!(false, "probe")).is_err());
    }

    #[test]
    fn compact_validation_keeps_overflow_checks() {
        let value = std::hint::black_box(u8::MAX);
        assert!(std::panic::catch_unwind(|| value + 1).is_err());
    }
}
