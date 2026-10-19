//! Hangs the overlay from the notch, so it can open out of it the way the
//! Dynamic Island does.
//!
//! The window sits over the menu bar at the top of the screen with the
//! pointer, centered on that screen's notch. Screens without a notch get the
//! same island at the middle of their top edge.

use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSEvent, NSScreen, NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior,
    NSWindowSharingType,
};
use objc2_foundation::{NSPoint, NSRect};
use serde::Serialize;
use tauri::WebviewWindow;

/// The notch's size in points. Zero on a screen without one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Notch {
    pub width: f64,
    pub height: f64,
}

/// Lets the overlay cover the menu bar, where the notch is, in every Space,
/// including full-screen apps. With `hide_from_capture`, screen sharing and
/// screenshots leave the overlay out: it shows recaps and other memory.
/// Littlebird offers the same for its overlay (`littlebird/FINDINGS.md`).
pub fn raise(window: &WebviewWindow, hide_from_capture: bool) -> tauri::Result<()> {
    let ns_window = ns_window(window)?;
    ns_window.setLevel(NSStatusWindowLevel);
    ns_window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    if hide_from_capture {
        ns_window.setSharingType(NSWindowSharingType::None);
    }
    Ok(())
}

/// Shows the overlay without taking focus, for the glow. Ordering a window
/// front while this app is active makes it the key window, taking the
/// keyboard from the app the person is in, so an active app is hidden first,
/// which hands focus back. Closing hides the app the same way; its windows
/// are then brought back without activating it. Runs on the main thread.
pub fn show_inactive(window: &WebviewWindow) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    if app.isActive() {
        app.hide(None);
    }
    app.unhideWithoutActivation();
    if let Ok(ns_window) = ns_window(window) {
        ns_window.orderFrontRegardless();
    }
}

/// Moves the overlay's top edge to the top of the screen with the pointer,
/// centered on its notch, and returns the notch's size. Runs on the main
/// thread.
pub fn place(window: &WebviewWindow) -> tauri::Result<Notch> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Ok(Notch::default());
    };
    let ns_window = ns_window(window)?;
    let pointer = NSEvent::mouseLocation();
    let screens = NSScreen::screens(mtm);
    let Some(screen) = screens
        .iter()
        .find(|screen| contains(screen.frame(), pointer))
        .or_else(|| screens.firstObject())
    else {
        return Ok(Notch::default());
    };

    let frame = screen.frame();
    let height = screen.safeAreaInsets().top;
    let (notch, center) = if height > 0.0 {
        // The notch is the gap between the two unobscured strips beside it.
        let left = screen.auxiliaryTopLeftArea();
        let right = screen.auxiliaryTopRightArea();
        let start = left.origin.x + left.size.width;
        let width = right.origin.x - start;
        (Notch { width, height }, start + width / 2.0)
    } else {
        (Notch::default(), frame.origin.x + frame.size.width / 2.0)
    };
    let width = ns_window.frame().size.width;
    ns_window.setFrameTopLeftPoint(NSPoint::new(
        center - width / 2.0,
        frame.origin.y + frame.size.height,
    ));
    Ok(notch)
}

/// The window's top left corner in global points, with the origin at the top
/// left of the main screen, as the pointer's position is given (`hover`).
/// Runs on the main thread.
pub fn top_left(window: &WebviewWindow) -> Option<(f64, f64)> {
    let mtm = MainThreadMarker::new()?;
    let frame = ns_window(window).ok()?.frame();
    // The first screen is the one with the menu bar, where both origins are.
    let main = NSScreen::screens(mtm).firstObject()?.frame();
    Some((
        frame.origin.x,
        main.size.height - (frame.origin.y + frame.size.height),
    ))
}

fn contains(rect: NSRect, point: NSPoint) -> bool {
    (rect.origin.x..rect.origin.x + rect.size.width).contains(&point.x)
        && (rect.origin.y..=rect.origin.y + rect.size.height).contains(&point.y)
}

fn ns_window(window: &WebviewWindow) -> tauri::Result<&NSWindow> {
    // Tauri owns the window and keeps it alive as long as `window`.
    Ok(unsafe { &*window.ns_window()?.cast::<NSWindow>() })
}
