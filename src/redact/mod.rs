//! Redaction helpers for hiding API keys in dashboard payloads.
//!
//! Port of `internal/redact/redact.go`. The `UsageSnapshot` helper that walks a
//! `StatisticsSnapshot` will be added in Phase 4 once the repository DTOs land.

use sha2::{Digest, Sha256};

const API_ALIAS_PREFIX: &str = "redacted_api_";

/// Returns a stable, opaque alias for an API key. Matches Go's `APIAlias`.
pub fn api_alias(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "unknown".to_string();
    }
    if trimmed == "unknown" || trimmed.starts_with(API_ALIAS_PREFIX) {
        return trimmed.to_string();
    }
    let mut hasher = Sha256::new();
    hasher.update(trimmed.as_bytes());
    let digest = hasher.finalize();
    let hex = hex::encode(digest);
    format!("{API_ALIAS_PREFIX}{}", &hex[..12])
}

/// Masked display name for an API key. Matches Go's `APIKeyDisplayName`.
pub fn api_key_display_name(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "unknown" {
        return "unknown".to_string();
    }

    let chars: Vec<char> = trimmed.chars().collect();
    let n = chars.len();

    if n <= 4 {
        return "*".repeat(n);
    }
    if n <= 8 {
        let mut out = String::new();
        out.push(chars[0]);
        out.push_str(&"*".repeat(n - 2));
        out.push(chars[n - 1]);
        return out;
    }
    let mut out = String::new();
    out.extend(&chars[..4]);
    out.push_str(&"*".repeat(n - 8));
    out.extend(&chars[n - 4..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_unknown_for_blank() {
        assert_eq!(api_alias(""), "unknown");
        assert_eq!(api_alias("   "), "unknown");
    }

    #[test]
    fn alias_passes_through_already_redacted() {
        assert_eq!(api_alias("unknown"), "unknown");
        assert_eq!(api_alias("redacted_api_abc"), "redacted_api_abc");
    }

    #[test]
    fn alias_is_stable_and_prefixed() {
        let a = api_alias("sk-very-secret-key");
        let b = api_alias("sk-very-secret-key");
        assert_eq!(a, b);
        assert!(a.starts_with(API_ALIAS_PREFIX));
        assert_eq!(a.len(), API_ALIAS_PREFIX.len() + 12);
    }

    #[test]
    fn display_name_short_keys() {
        assert_eq!(api_key_display_name(""), "unknown");
        assert_eq!(api_key_display_name("ab"), "**");
        assert_eq!(api_key_display_name("abcd"), "****");
        assert_eq!(api_key_display_name("abcdef"), "a****f");
    }

    #[test]
    fn display_name_long_key() {
        assert_eq!(api_key_display_name("abcdefghij"), "abcd**ghij");
        assert_eq!(
            api_key_display_name("sk-proj-abcdefghij"),
            "sk-p**********ghij"
        );
    }
}
