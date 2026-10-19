// Pages the bridge never reads. Checked in the service worker before any
// extraction is requested, so excluded pages never leave the tab.

const EXCLUDED_DOMAINS = [
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

// Matched anywhere in the hostname, so "hdfcbank.com" and "mylogin.example"
// are excluded. Over-excluding a few unrelated sites is the safer mistake.
const EXCLUDED_HOST_SUBSTRINGS = ["bank", "login", "signin", "password"];

// Too short to match inside words, so matched only as a whole label:
// "auth.example.com" is excluded, "author.example.com" is not.
const EXCLUDED_HOST_LABELS = ["auth", "sso", "accounts", "secure"];

// Private parts of sites that are otherwise read: X's direct messages.
const EXCLUDED_PATHS = [
  { domains: ["x.com", "twitter.com"], prefixes: ["/messages", "/i/chat"] },
];

/** Returns the reason a URL is excluded, or null when it may be read. */
export function exclusionReason(rawUrl) {
  let url;
  try {
    url = new URL(rawUrl);
  } catch {
    return "unparseable-url";
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return "unsupported-scheme";

  const host = url.hostname.toLowerCase();
  const domain = EXCLUDED_DOMAINS.find((d) => host === d || host.endsWith(`.${d}`));
  if (domain) return `domain:${domain}`;

  const substring = EXCLUDED_HOST_SUBSTRINGS.find((k) => host.includes(k));
  if (substring) return `keyword:${substring}`;

  const labels = host.split(/[.-]/);
  const label = EXCLUDED_HOST_LABELS.find((k) => labels.includes(k));
  if (label) return `keyword:${label}`;

  // A post Freebyrd opened by itself, not the person.
  if (url.searchParams.get("freebyrd") === "1") return "freebyrd";

  const path = url.pathname.toLowerCase();
  for (const { domains, prefixes } of EXCLUDED_PATHS) {
    if (!domains.some((d) => host === d || host.endsWith(`.${d}`))) continue;
    const prefix = prefixes.find((p) => path === p || path.startsWith(`${p}/`));
    if (prefix) return `path:${prefix}`;
  }

  return null;
}
