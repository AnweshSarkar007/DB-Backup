//! Secret redaction for logs and error detail (Section 18 / Section 28).
//!
//! The golden rule: no password, connection URI with password, or token ever reaches a log sink
//! or an error surfaced to the UI. This module provides the sanitizers used at every boundary.

/// The placeholder we substitute for any redacted secret.
pub const MASK: &str = "****";

/// Redact the password portion of a PostgreSQL connection URI.
///
/// `postgresql://user:secret@host:5432/db?x=y` -> `postgresql://user:****@host:5432/db?x=y`
///
/// Handles the `scheme://user:pass@host` shape. If there is no userinfo or no password, the input
/// is returned unchanged. Non-URI strings are returned unchanged (nothing to redact here).
pub fn redact_connection_uri(input: &str) -> String {
    // Find scheme separator.
    let Some(scheme_end) = input.find("://") else {
        return input.to_string();
    };
    let (scheme, rest) = input.split_at(scheme_end + 3); // include "://"

    // userinfo is everything up to the first '@' that precedes the host.
    let Some(at) = rest.find('@') else {
        return input.to_string();
    };
    let (userinfo, host_part) = rest.split_at(at); // host_part starts with '@'

    // userinfo may be "user:pass" or just "user".
    match userinfo.split_once(':') {
        Some((user, _pass)) => format!("{scheme}{user}:{MASK}{host_part}"),
        None => input.to_string(),
    }
}

/// Redact any occurrences of a known secret substring within an arbitrary string.
///
/// Used to scrub tool output (pg_dump stderr, docker stderr) that may echo a password. Empty or
/// very short secrets are ignored to avoid masking unrelated text.
pub fn redact_secret(haystack: &str, secret: &str) -> String {
    if secret.len() < 4 {
        return haystack.to_string();
    }
    haystack.replace(secret, MASK)
}

/// Redact common `KEY=value` secret assignments (PGPASSWORD, tokens) in free text.
pub fn redact_env_assignments(input: &str) -> String {
    const KEYS: [&str; 6] = [
        "PGPASSWORD",
        "password",
        "PASSWORD",
        "access_token",
        "refresh_token",
        "authorization",
    ];
    let mut out = input.to_string();
    for key in KEYS {
        out = redact_kv(&out, key);
    }
    out
}

/// Replace `key=<value>` (value up to whitespace, quote, `&` or `;`) with `key=****`.
fn redact_kv(input: &str, key: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let lower_input = input.to_ascii_lowercase();
    let lower_key = key.to_ascii_lowercase();
    let needle = format!("{lower_key}=");
    let mut cursor = 0usize;

    while let Some(rel) = lower_input[cursor..].find(&needle) {
        let start = cursor + rel;
        let value_start = start + needle.len();
        // copy up to and including "key="
        result.push_str(&input[cursor..value_start]);
        result.push_str(MASK);
        // skip the original value
        let bytes = input.as_bytes();
        let mut i = value_start;
        while i < bytes.len() {
            let c = bytes[i] as char;
            if c.is_whitespace() || c == '"' || c == '\'' || c == '&' || c == ';' {
                break;
            }
            i += 1;
        }
        cursor = i;
    }
    result.push_str(&input[cursor..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_uri_password() {
        assert_eq!(
            redact_connection_uri("postgresql://postgres:sup3rs3cret@db.example.com:5432/postgres"),
            "postgresql://postgres:****@db.example.com:5432/postgres"
        );
    }

    #[test]
    fn redacts_uri_with_query() {
        assert_eq!(
            redact_connection_uri("postgres://u:p@h:5432/d?sslmode=require"),
            "postgres://u:****@h:5432/d?sslmode=require"
        );
    }

    #[test]
    fn leaves_uri_without_password() {
        assert_eq!(
            redact_connection_uri("postgresql://postgres@host/db"),
            "postgresql://postgres@host/db"
        );
    }

    #[test]
    fn leaves_non_uri_unchanged() {
        assert_eq!(redact_connection_uri("just some text"), "just some text");
    }

    #[test]
    fn redacts_known_secret_in_output() {
        let out = "connection to server failed: password 'hunter2xyz' rejected";
        assert_eq!(
            redact_secret(out, "hunter2xyz"),
            "connection to server failed: password '****' rejected"
        );
    }

    #[test]
    fn ignores_short_secret() {
        assert_eq!(redact_secret("abc def", "ab"), "abc def");
    }

    #[test]
    fn redacts_env_assignments() {
        let s = "docker run -e PGPASSWORD=topsecret -e OTHER=1";
        assert_eq!(
            redact_env_assignments(s),
            "docker run -e PGPASSWORD=**** -e OTHER=1"
        );
    }

    #[test]
    fn redacts_token_kv() {
        let s = "refresh_token=1//abcXYZ&scope=drive";
        assert_eq!(redact_env_assignments(s), "refresh_token=****&scope=drive");
    }
}
