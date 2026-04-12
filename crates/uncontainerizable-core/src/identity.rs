//! Identity validation and prefix combination.
//!
//! Identity strings become cgroup dirnames on Linux, Windows Job Object names,
//! and argv[0] tags on macOS. The character set is restrictive by design so
//! the same string is safe in all three contexts.

use crate::error::Error;

const MAX_LENGTH: usize = 200;

/// Characters allowed in identities.
#[inline]
fn is_valid_identity_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':')
}

pub fn validate(raw: &str) -> Result<(), Error> {
    if raw.is_empty() || raw.len() > MAX_LENGTH {
        return Err(Error::InvalidIdentity(format!(
            "must be 1..={} chars, got {}",
            MAX_LENGTH,
            raw.len()
        )));
    }
    if !raw.chars().all(is_valid_identity_char) {
        return Err(Error::InvalidIdentity(
            "allowed chars: a-z A-Z 0-9 . _ - :".into(),
        ));
    }
    Ok(())
}

pub fn combine(prefix: &str, identity: &str) -> String {
    format!("{prefix}:{identity}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_typical_reverse_dns_prefix() {
        assert!(validate("com.example.my-supervisor").is_ok());
    }

    #[test]
    fn accepts_alnum_dot_underscore_dash_colon() {
        assert!(validate("abc.DEF_123-xyz:sub").is_ok());
    }

    #[test]
    fn rejects_empty() {
        let err = validate("").unwrap_err();
        assert!(matches!(err, Error::InvalidIdentity(_)));
    }

    #[test]
    fn rejects_too_long() {
        let s = "a".repeat(MAX_LENGTH + 1);
        let err = validate(&s).unwrap_err();
        assert!(matches!(err, Error::InvalidIdentity(_)));
    }

    #[test]
    fn accepts_max_length() {
        let s = "a".repeat(MAX_LENGTH);
        assert!(validate(&s).is_ok());
    }

    #[test]
    fn rejects_whitespace() {
        assert!(validate("bad space").is_err());
        assert!(validate("bad\ttab").is_err());
    }

    #[test]
    fn rejects_slashes() {
        assert!(validate("bad/slash").is_err());
        assert!(validate("bad\\backslash").is_err());
    }

    #[test]
    fn accepts_dotdot_intentionally() {
        // `..` passes identity validation; path-traversal-style strings get
        // rejected at the platform sanitization layer (e.g. `sanitize_for_cgroup`)
        // once combined with a prefix. Keeping the identity layer liberal lets
        // callers use identity strings like "v1..v2" if they want.
        assert!(validate("..").is_ok());
    }

    #[test]
    fn rejects_unicode() {
        assert!(validate("unicode–dash").is_err()); // en dash, not ASCII dash
        assert!(validate("ñ").is_err());
    }

    #[test]
    fn combine_uses_colon_separator() {
        assert_eq!(
            combine("com.example.my-supervisor", "browser-main"),
            "com.example.my-supervisor:browser-main"
        );
    }
}
