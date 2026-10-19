//! Lets the overlay's page draw at the display's refresh rate.
//!
//! WebKit renders pages at about 60 fps even on 120 Hz ProMotion displays,
//! through its "Prefer Page Rendering Updates near 60fps" feature (on by
//! default, macOS 26). Turning it off for this one web view lets the island's
//! animations run at 120 fps. The switch is WebKit SPI, so it is looked up by
//! name and skipped if a later macOS removes it.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, sel};
use objc2_foundation::{NSArray, NSString};

const FEATURE: &str = "PreferPageRenderingUpdatesNear60FPSEnabled";

/// Turns off WebKit's 60 fps preference for the `WKWebView` at `webview`.
/// Returns whether the switch was found. Runs on the main thread.
pub fn unlock(webview: *mut c_void) -> bool {
    let Some(class) = AnyClass::get(c"WKPreferences") else {
        return false;
    };
    let webview: &AnyObject = unsafe { &*webview.cast() };
    unsafe {
        let has_features: bool = msg_send![class, respondsToSelector: sel!(_features)];
        if !has_features {
            return false;
        }
        let configuration: Retained<AnyObject> = msg_send![webview, configuration];
        let preferences: Retained<AnyObject> = msg_send![&configuration, preferences];
        let can_set: bool =
            msg_send![&preferences, respondsToSelector: sel!(_setEnabled:forFeature:)];
        if !can_set {
            return false;
        }
        let features: Retained<NSArray<AnyObject>> = msg_send![class, _features];
        for feature in features.iter() {
            let key: Retained<NSString> = msg_send![&feature, key];
            if key.to_string() == FEATURE {
                let _: () = msg_send![&preferences, _setEnabled: false, forFeature: &*feature];
                return true;
            }
        }
    }
    false
}
