//! Reads other apps' on-screen content through the macOS Accessibility API.
//!
//! `ax` wraps the C API, `snapshot` walks an app's element tree, and `text`
//! flattens a snapshot into readable text. `browser` holds page snapshots
//! that the browser extension sends, since browsers hide page content from
//! the Accessibility API, and `bridge` asks the extension for a page on demand.

pub mod ax;
pub mod bridge;
pub mod browser;
pub mod snapshot;
pub mod text;
