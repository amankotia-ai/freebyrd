//! Takes secrets out of text before Freebyrd stores it or sends it to
//! Claude: private keys, tokens, API keys, passwords written as settings,
//! and card numbers. Names, addresses and the like are not secrets in this
//! sense and stay.

use std::sync::LazyLock;

use regex::{Captures, Regex};

static SECRETS: LazyLock<Vec<(Regex, &'static str)>> =
    LazyLock::new(|| {
        [
        (
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|\z)",
            "[private key]",
        ),
        // JSON Web Tokens: three base64url parts, the first starting `eyJ`.
        (
            r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
            "[token]",
        ),
        (r"\bsk-ant-[A-Za-z0-9_-]{20,}", "[API key]"),
        (r"\bsk-(?:proj-)?[A-Za-z0-9_-]{20,}", "[API key]"),
        (r"\b(?:sk|rk|pk)_(?:live|test)_[A-Za-z0-9]{16,}", "[API key]"),
        (r"\bAIza[0-9A-Za-z_-]{35}", "[API key]"),
        // TypeSafe (Jev): `apikey_` and two hex parts.
        (r"\bapikey_[0-9A-Za-z]{16,}_[0-9A-Za-z]{32,}", "[API key]"),
        (r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", "[API key]"),
        (r"\bgh[pousr]_[A-Za-z0-9]{30,}", "[token]"),
        (r"\bgithub_pat_[A-Za-z0-9_]{40,}", "[token]"),
        (r"\bglpat-[A-Za-z0-9_-]{20,}", "[token]"),
        (r"\bxox[abposr]-[A-Za-z0-9-]{10,}", "[token]"),
        (r"\bnpm_[A-Za-z0-9]{36}\b", "[token]"),
    ]
    .into_iter()
    .map(|(pattern, label)| (Regex::new(pattern).expect("valid pattern"), label))
    .collect()
    });

/// `password: hunter2!x`, `API_KEY=...`: keeps the name, drops the value.
static SETTINGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(password|passwd|pwd|secret|api[_-]?key|access[_-]?token|auth[_-]?token|client[_-]?secret)(\s*[:=]\s*)["']?[^\s"']{8,}["']?"#,
    )
    .expect("valid pattern")
});

/// 13 to 19 digits, spaces or dashes allowed between them, starting like a
/// card number does (3 to 6). Kept only if the Luhn check passes.
static CARD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[3-6](?:[ -]?\d){12,18}\b").expect("valid pattern"));

pub fn redact(text: &str) -> String {
    let mut out = text.to_owned();
    for (pattern, label) in SECRETS.iter() {
        if pattern.is_match(&out) {
            out = pattern.replace_all(&out, *label).into_owned();
        }
    }
    if SETTINGS.is_match(&out) {
        out = SETTINGS
            .replace_all(&out, |c: &Captures| format!("{}{}[redacted]", &c[1], &c[2]))
            .into_owned();
    }
    if CARD.is_match(&out) {
        out = CARD
            .replace_all(&out, |c: &Captures| {
                if luhn(&c[0]) {
                    "[card number]".to_owned()
                } else {
                    c[0].to_owned()
                }
            })
            .into_owned();
    }
    out
}

fn luhn(number: &str) -> bool {
    let digits: Vec<u32> = number.chars().filter_map(|c| c.to_digit(10)).collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| match (i % 2 == 1, d * 2) {
            (true, doubled) if doubled > 9 => doubled - 9,
            (true, doubled) => doubled,
            (false, _) => d,
        })
        .sum();
    sum.is_multiple_of(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_keys_and_tokens() {
        let text = "key sk-ant-api03-abcdefghijklmnopqrstuvwxyz and ghp_abcdefghijklmnopqrstuvwxyz0123456789 \
                    and eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        assert_eq!(redact(text), "key [API key] and [token] and [token]");
        assert_eq!(
            redact("apikey_0123456789abcdef0123_0123456789abcdef0123456789abcdef0123"),
            "[API key]"
        );
    }

    #[test]
    fn removes_private_keys() {
        let text = "before\n-----BEGIN OPENSSH PRIVATE KEY-----\nabc\ndef\n-----END OPENSSH PRIVATE KEY-----\nafter";
        assert_eq!(redact(text), "before\n[private key]\nafter");
    }

    #[test]
    fn keeps_setting_names_but_not_values() {
        assert_eq!(
            redact("set PASSWORD=hunter2hunter2 then API_KEY: \"abcd1234efgh\""),
            "set PASSWORD=[redacted] then API_KEY: [redacted]"
        );
        // Ordinary words after "password" are not values.
        assert_eq!(redact("the password is short"), "the password is short");
    }

    #[test]
    fn removes_card_numbers_but_not_other_numbers() {
        assert_eq!(
            redact("card 4242 4242 4242 4242 ok"),
            "card [card number] ok"
        );
        // A millisecond timestamp and a phone number stay.
        assert_eq!(redact("at 1760300000000"), "at 1760300000000");
        assert_eq!(redact("call +91 98765 43210"), "call +91 98765 43210");
        // Fails the Luhn check.
        assert_eq!(redact("4242 4242 4242 4241"), "4242 4242 4242 4241");
    }
}
