//! The iPad app's own files, built into Freebyrd so nothing is read from the
//! source folder (which syncs to iCloud) at run time. `FREEBYRD_COMPANION_UI`
//! points at a folder to serve from instead, for working on the page.

use super::http::Response;

const CSP: &str = "default-src 'self'; img-src 'self' data: blob:; media-src 'self' blob:; \
style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; worker-src 'self'; \
manifest-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

macro_rules! file {
    ($name:literal) => {
        include_bytes!(concat!("../../companion-ui/", $name)).as_slice()
    };
}

/// Path, type, contents.
const FILES: &[(&str, &str, &[u8])] = &[
    (
        "/index.html",
        "text/html; charset=utf-8",
        file!("index.html"),
    ),
    ("/app.css", "text/css; charset=utf-8", file!("app.css")),
    ("/app.js", "text/javascript; charset=utf-8", file!("app.js")),
    (
        "/screens.js",
        "text/javascript; charset=utf-8",
        file!("screens.js"),
    ),
    ("/sw.js", "text/javascript; charset=utf-8", file!("sw.js")),
    (
        "/manifest.webmanifest",
        "application/manifest+json",
        file!("manifest.webmanifest"),
    ),
    ("/pair.html", "text/html; charset=utf-8", file!("pair.html")),
    ("/pair.js", "text/javascript; charset=utf-8", file!("pair.js")),
    ("/icon-180.png", "image/png", file!("icon-180.png")),
    ("/icon-192.png", "image/png", file!("icon-192.png")),
    ("/icon-512.png", "image/png", file!("icon-512.png")),
];

/// A file of the app, by its path. The pairing page is served by `/pair`
/// only (`pair::page`), which checks it is this Mac asking.
pub fn get(path: &str) -> Response {
    let path = if path == "/" { "/index.html" } else { path };
    if path == "/pair.html" {
        return Response::error(404, "Not here.");
    }
    file(path)
}

pub(super) fn file(path: &str) -> Response {
    let Some((name, kind, built)) = FILES.iter().find(|(name, ..)| *name == path) else {
        return Response::error(404, "Not here.");
    };
    let body = std::env::var_os("FREEBYRD_COMPANION_UI")
        .and_then(|dir| {
            std::fs::read(std::path::Path::new(&dir).join(name.trim_start_matches('/'))).ok()
        })
        .unwrap_or_else(|| built.to_vec());
    let mut response = Response::new(200, kind, body);
    if kind.starts_with("text/html") {
        response = response
            .with("Content-Security-Policy", CSP)
            .with("X-Frame-Options", "DENY");
    }
    // The service worker keeps the app for offline use; the network copy is
    // always checked first.
    response = response.with("Cache-Control", "no-cache");
    if *name == "/sw.js" {
        response = response.with("Service-Worker-Allowed", "/");
    }
    response
}
