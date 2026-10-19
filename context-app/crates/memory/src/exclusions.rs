//! Web pages that are never kept. Mirrors `extension/exclusions.js`, which
//! applies the same rules in the browser; keep the two in step. Here they
//! filter browser history, which records every page, excluded or not.

const EXCLUDED_DOMAINS: &[&str] = &[
    "1password.com",
    "bitwarden.com",
    "lastpass.com",
    "dashlane.com",
    "keepersecurity.com",
    "nordpass.com",
    "pass.proton.me",
    "onetimesecret.com",
    "privnote.com",
    "pwpush.com",
    "accounts.google.com",
    "appleid.apple.com",
    "icloud.com",
];

/// Matched anywhere in the hostname, so "hdfcbank.com" and "mylogin.example"
/// are excluded. Over-excluding a few unrelated sites is the safer mistake.
const EXCLUDED_HOST_SUBSTRINGS: &[&str] = &["bank", "login", "signin", "password"];

/// Too short to match inside words, so matched only as a whole label:
/// "auth.example.com" is excluded, "author.example.com" is not.
const EXCLUDED_HOST_LABELS: &[&str] = &["auth", "sso", "accounts", "secure"];

/// Private parts of sites that are otherwise read: X's direct messages.
const EXCLUDED_PATHS: &[(&[&str], &[&str])] =
    &[(&["x.com", "twitter.com"], &["/messages", "/i/chat"])];

/// The reason a URL is excluded, or `None` when it may be kept.
pub fn reason(url: &str) -> Option<String> {
    let Some((scheme, _)) = url.split_once("://") else {
        return Some("unparseable-url".to_owned());
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Some("unsupported-scheme".to_owned());
    }
    let Some(host) = hostname(url) else {
        return Some("unparseable-url".to_owned());
    };

    if let Some(domain) = EXCLUDED_DOMAINS
        .iter()
        .find(|d| host == **d || host.ends_with(&format!(".{d}")))
    {
        return Some(format!("domain:{domain}"));
    }
    if let Some(keyword) = EXCLUDED_HOST_SUBSTRINGS.iter().find(|k| host.contains(*k)) {
        return Some(format!("keyword:{keyword}"));
    }
    let labels: Vec<&str> = host.split(['.', '-']).collect();
    if let Some(label) = EXCLUDED_HOST_LABELS.iter().find(|k| labels.contains(k)) {
        return Some(format!("keyword:{label}"));
    }
    // A post Freebyrd opened by itself, not the person. It no longer opens
    // any (the X suggestions were removed, 2026-10-16); this keeps the
    // visits it made before out of the memory if history is read again.
    if url
        .split_once('?')
        .is_some_and(|(_, query)| query.split(['&', '#']).any(|pair| pair == "freebyrd=1"))
    {
        return Some("freebyrd".to_owned());
    }
    let path = path(url).to_ascii_lowercase();
    for (domains, prefixes) in EXCLUDED_PATHS {
        if !domains
            .iter()
            .any(|d| host == *d || host.ends_with(&format!(".{d}")))
        {
            continue;
        }
        if let Some(prefix) = prefixes
            .iter()
            .find(|p| path == **p || path.starts_with(&format!("{p}/")))
        {
            return Some(format!("path:{prefix}"));
        }
    }
    None
}

/// The path of an http(s) URL, "/" when it has none.
pub(crate) fn path(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    rest.find('/').map_or("/", |at| &rest[at..])
}

/// The lowercase hostname of an http(s) URL.
fn hostname(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next()?;
    let host = match host_port.strip_prefix('[') {
        Some(ipv6) => ipv6.split(']').next()?,
        None => host_port.split(':').next()?,
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The site a URL is on, without a leading "www.".
pub fn site(url: &str) -> Option<String> {
    let host = hostname(url)?;
    Some(host.strip_prefix("www.").unwrap_or(&host).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_extension_rules() {
        assert_eq!(reason("https://example.com/a"), None);
        assert_eq!(
            reason("https://my.1password.com/"),
            Some("domain:1password.com".into())
        );
        assert_eq!(
            reason("https://www.hdfcbank.com/"),
            Some("keyword:bank".into())
        );
        assert_eq!(
            reason("https://auth.example.com/x"),
            Some("keyword:auth".into())
        );
        assert_eq!(reason("https://author.example.com/x"), None);
        assert_eq!(
            reason("https://accounts.google.com/signin"),
            Some("domain:accounts.google.com".into())
        );
        assert_eq!(
            reason("chrome://extensions"),
            Some("unsupported-scheme".into())
        );
        assert_eq!(reason("not a url"), Some("unparseable-url".into()));
        assert_eq!(
            reason("https://user:pw@LOGIN.example.com:8080/"),
            Some("keyword:login".into())
        );
        assert_eq!(
            reason("https://x.com/messages/123-456"),
            Some("path:/messages".into())
        );
        assert_eq!(reason("https://x.com/i/chat"), Some("path:/i/chat".into()));
        assert_eq!(
            reason("https://mobile.twitter.com/messages?x=1"),
            Some("path:/messages".into())
        );
        assert_eq!(reason("https://x.com/home"), None);
        assert_eq!(reason("https://x.com/messagesfan/status/1"), None);
        assert_eq!(reason("https://example.com/messages"), None);
        assert_eq!(
            reason("https://x.com/a/status/123?freebyrd=1"),
            Some("freebyrd".into())
        );
    }

    #[test]
    fn site_drops_www() {
        assert_eq!(
            site("https://www.github.com/a?b#c").as_deref(),
            Some("github.com")
        );
        assert_eq!(site("http://[::1]:3000/").as_deref(), Some("::1"));
    }
}
