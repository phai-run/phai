//! Backward-compatibility helpers for the `finance-os` → `phai` rename.
//!
//! The product was renamed from `finance-os` to `phai`. Its on-disk identity
//! (config/data directories, the local DB filename) and its environment
//! variables must keep working for users who installed under the old name.
//!
//! The rule is uniform and non-destructive: resolve the new `phai` identity
//! first, and fall back to the legacy `finance-os` identity only when the new
//! one is absent. Nothing is moved or rewritten. See ADR-0021.

use std::ffi::OsString;

/// Read an environment variable by its new `PHAI_*` name, falling back to the
/// legacy `FINANCE_OS_*` name. Returns the first that is set (new name wins).
pub fn env_var_os(new_name: &str, legacy_name: &str) -> Option<OsString> {
    std::env::var_os(new_name).or_else(|| std::env::var_os(legacy_name))
}

/// String variant of [`env_var_os`]. A value that is set but not valid UTF-8
/// is treated as absent for that name (matching `std::env::var` semantics)
/// before the fallback is tried.
pub fn env_var(new_name: &str, legacy_name: &str) -> Option<String> {
    std::env::var(new_name)
        .ok()
        .or_else(|| std::env::var(legacy_name).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn clear(new: &str, legacy: &str) {
        unsafe {
            std::env::remove_var(new);
            std::env::remove_var(legacy);
        }
    }

    #[test]
    #[serial]
    fn new_name_takes_precedence_over_legacy() {
        let (new, legacy) = ("PHAI_COMPAT_TEST", "FINANCE_OS_COMPAT_TEST");
        clear(new, legacy);
        unsafe {
            std::env::set_var(new, "new");
            std::env::set_var(legacy, "legacy");
        }
        assert_eq!(env_var(new, legacy).as_deref(), Some("new"));
        clear(new, legacy);
    }

    #[test]
    #[serial]
    fn falls_back_to_legacy_when_new_absent() {
        let (new, legacy) = ("PHAI_COMPAT_TEST", "FINANCE_OS_COMPAT_TEST");
        clear(new, legacy);
        unsafe {
            std::env::set_var(legacy, "legacy");
        }
        assert_eq!(env_var(new, legacy).as_deref(), Some("legacy"));
        assert_eq!(env_var_os(new, legacy), Some(OsString::from("legacy")));
        clear(new, legacy);
    }

    #[test]
    #[serial]
    fn none_when_neither_set() {
        let (new, legacy) = ("PHAI_COMPAT_TEST", "FINANCE_OS_COMPAT_TEST");
        clear(new, legacy);
        assert_eq!(env_var(new, legacy), None);
        assert_eq!(env_var_os(new, legacy), None);
    }
}
