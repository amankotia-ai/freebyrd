//! A picture of the window the person was in when they opened the overlay.
//! It goes with their question, so Claude can see what the window's text
//! leaves out: charts, images, colors, layout. Littlebird sends a window
//! screenshot with each question too (FINDINGS §5.4).
//!
//! Taken with ScreenCaptureKit, which needs Screen Recording access for the
//! app macOS holds responsible: Freebyrd, or whatever launched it. Only that
//! window is pictured, not what covers it, so the overlay never shows up in
//! it. The picture is kept in memory, never written to disk.

use std::fmt;
use std::sync::Once;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use block2::RcBlock;
use objc2::AllocAnyThread;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImageCompressionFactor};
use objc2_core_foundation::{
    CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    CGImage, CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess,
    CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID, kCGWindowBounds,
    kCGWindowLayer, kCGWindowName, kCGWindowNumber, kCGWindowOwnerPID,
};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSNumber, NSString};
use objc2_screen_capture_kit::{
    SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration,
};
use serde::ser::{Serialize, SerializeStruct, Serializer};

/// Claude scales a picture down to about this on its long edge, so a bigger
/// one only costs time to send.
const MAX_EDGE_PX: f64 = 1568.0;
const JPEG_QUALITY: f64 = 0.8;
const TIMEOUT: Duration = Duration::from_secs(3);
/// A smaller window is a panel or popover, not what the person was reading.
const MIN_WINDOW_PT: f64 = 120.0;

/// The window as a JPEG.
#[derive(Clone)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub jpeg: Vec<u8>,
}

impl fmt::Debug for Picture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Picture({}x{}, {} bytes)",
            self.width,
            self.height,
            self.jpeg.len()
        )
    }
}

/// For "What was read" in the overlay: the size, and the picture itself as a
/// data URL for an `img`.
impl Serialize for Picture {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut picture = serializer.serialize_struct("Picture", 4)?;
        picture.serialize_field("width", &self.width)?;
        picture.serialize_field("height", &self.height)?;
        picture.serialize_field("bytes", &self.jpeg.len())?;
        let url = format!("data:image/jpeg;base64,{}", BASE64.encode(&self.jpeg));
        picture.serialize_field("dataUrl", &url)?;
        picture.end()
    }
}

/// Pictures the front window of the app with this process ID, or says why
/// it could not.
pub fn take(pid: i32) -> Result<Picture, String> {
    if !CGPreflightScreenCaptureAccess() {
        // macOS shows its prompt only the first time an app asks; after
        // that, access is given in System Settings.
        static ASKED: Once = Once::new();
        ASKED.call_once(|| {
            CGRequestScreenCaptureAccess();
        });
        return Err("Screen Recording access is not granted".to_owned());
    }
    let window_id = front_window(pid)
        .ok_or_else(|| "the app has no window on screen".to_owned())?
        .id;

    let (sender, receiver) = mpsc::channel();
    let found = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: ScreenCaptureKit passes a valid object or null.
            let started = match unsafe { content.as_ref() } {
                Some(content) => shoot(content, window_id, sender.clone()),
                None => Err(failure(error)),
            };
            if let Err(e) = started {
                let _ = sender.send(Err(e));
            }
        },
    );
    // SAFETY: the handler's types match what ScreenCaptureKit calls it with.
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true, true, &found,
        );
    }
    receiver
        .recv_timeout(TIMEOUT)
        .unwrap_or_else(|_| Err("the picture took too long".to_owned()))
}

/// The title of the window [`take`] would picture. macOS gives other apps'
/// window titles only with Screen Recording access, so without it this is
/// `None`.
pub fn front_window_title(pid: i32) -> Option<String> {
    front_window(pid)?.title
}

struct FrontWindow {
    id: u32,
    title: Option<String>,
}

/// The app's frontmost ordinary window. The window list runs front to back;
/// layer 0 leaves out menus, the menu bar and floating panels.
fn front_window(pid: i32) -> Option<FrontWindow> {
    let options =
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID)?;
    // SAFETY: the window list is an array of dictionaries keyed by strings.
    let list: CFRetained<CFArray<CFDictionary<CFString, CFType>>> =
        unsafe { CFRetained::cast_unchecked(list) };
    // SAFETY: CoreGraphics' constant keys.
    let (owner, layer, number, bounds, name) = unsafe {
        (
            kCGWindowOwnerPID,
            kCGWindowLayer,
            kCGWindowNumber,
            kCGWindowBounds,
            kCGWindowName,
        )
    };
    let (width, height) = (
        CFString::from_static_str("Width"),
        CFString::from_static_str("Height"),
    );
    list.iter().find_map(|window| {
        let int = |key: &CFString| window.get(key)?.downcast::<CFNumber>().ok()?.as_i64();
        if int(owner)? != i64::from(pid) || int(layer)? != 0 {
            return None;
        }
        let size = window.get(bounds)?.downcast::<CFDictionary>().ok()?;
        // SAFETY: a window's bounds map "X", "Y", "Width", "Height" to numbers.
        let size: CFRetained<CFDictionary<CFString, CFNumber>> =
            unsafe { CFRetained::cast_unchecked(size) };
        let side = |key: &CFString| size.get(key)?.as_f64();
        if side(&width)? < MIN_WINDOW_PT || side(&height)? < MIN_WINDOW_PT {
            return None;
        }
        Some(FrontWindow {
            id: u32::try_from(int(number)?).ok()?,
            title: window
                .get(name)
                .and_then(|title| title.downcast::<CFString>().ok())
                .map(|title| title.to_string()),
        })
    })
}

/// Starts the picture of `window_id`; `sender` gets it, or why it failed.
///
/// A lone window pictured by itself comes with its shadow, the window shrunk
/// to fit it. So the picture is of the window's own area of its screen, with
/// that window drawn and nothing else: no shadow, no windows over it, and
/// never the overlay.
fn shoot(
    content: &SCShareableContent,
    window_id: u32,
    sender: Sender<Result<Picture, String>>,
) -> Result<(), String> {
    // SAFETY: plain property reads on objects ScreenCaptureKit returned.
    let window = unsafe { content.windows() }
        .iter()
        .find(|window| unsafe { window.windowID() } == window_id)
        .ok_or_else(|| "the window cannot be pictured".to_owned())?;
    let frame = unsafe { window.frame() };
    let display = unsafe { content.displays() }
        .iter()
        .map(|display| (overlap(unsafe { display.frame() }, frame), display))
        .filter(|(shared, _)| *shared > 0.0)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, display)| display)
        .ok_or_else(|| "the window is on no screen".to_owned())?;
    let screen = unsafe { display.frame() };
    let filter = unsafe {
        SCContentFilter::initWithDisplay_includingWindows(
            SCContentFilter::alloc(),
            &display,
            &NSArray::from_slice(&[&*window]),
        )
    };
    let scale = pixels_per_point(&frame.size, f64::from(unsafe { filter.pointPixelScale() }));
    let px = |points: f64| (points * scale).round().max(1.0) as usize;
    let config = unsafe { SCStreamConfiguration::new() };
    unsafe {
        config.setSourceRect(window_on_screen(frame, screen));
        config.setWidth(px(frame.size.width));
        config.setHeight(px(frame.size.height));
        config.setShowsCursor(false);
    }

    let done = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
        // SAFETY: ScreenCaptureKit passes a valid image or null.
        let picture = match unsafe { image.as_ref() } {
            Some(image) => encode(image),
            None => Err(failure(error)),
        };
        let _ = sender.send(picture);
    });
    // SAFETY: the handler's types match what ScreenCaptureKit calls it with.
    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            &filter,
            &config,
            Some(&done),
        );
    }
    Ok(())
}

/// The window's frame relative to its screen, where both are in global
/// points.
fn window_on_screen(frame: CGRect, screen: CGRect) -> CGRect {
    CGRect::new(
        CGPoint::new(
            frame.origin.x - screen.origin.x,
            frame.origin.y - screen.origin.y,
        ),
        frame.size,
    )
}

/// How many square points two rectangles share.
fn overlap(a: CGRect, b: CGRect) -> f64 {
    let width =
        (a.origin.x + a.size.width).min(b.origin.x + b.size.width) - a.origin.x.max(b.origin.x);
    let height =
        (a.origin.y + a.size.height).min(b.origin.y + b.size.height) - a.origin.y.max(b.origin.y);
    width.max(0.0) * height.max(0.0)
}

/// Pixels a point for the picture: the window as sharp as the screen shows
/// it (`screen_scale`), or less, so the window fits within `MAX_EDGE_PX`.
fn pixels_per_point(window: &CGSize, screen_scale: f64) -> f64 {
    let long_edge = window.width.max(window.height).max(1.0);
    screen_scale.max(1.0).min(MAX_EDGE_PX / long_edge)
}

fn encode(image: &CGImage) -> Result<Picture, String> {
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), image);
    let quality = NSNumber::new_f64(JPEG_QUALITY);
    let quality: &AnyObject = &quality;
    // SAFETY: an AppKit constant.
    let key: &NSString = unsafe { NSImageCompressionFactor };
    let properties = NSDictionary::from_slices(&[key], &[quality]);
    // SAFETY: the compression factor is a number, as AppKit expects.
    let jpeg =
        unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::JPEG, &properties) }
            .ok_or_else(|| "the picture could not be made into a JPEG".to_owned())?;
    Ok(Picture {
        width: usize::try_from(rep.pixelsWide()).unwrap_or_default(),
        height: usize::try_from(rep.pixelsHigh()).unwrap_or_default(),
        jpeg: jpeg.to_vec(),
    })
}

fn failure(error: *mut NSError) -> String {
    // SAFETY: ScreenCaptureKit passes a valid error or null.
    match unsafe { error.as_ref() } {
        Some(error) => error.localizedDescription().to_string(),
        None => "the picture could not be taken".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_as_sharp_as_the_screen_within_the_limit() {
        let scale = |width, height, screen| pixels_per_point(&CGSize::new(width, height), screen);
        // A small window on a Retina screen keeps both pixels a point.
        assert_eq!(scale(600.0, 400.0, 2.0), 2.0);
        // A large one is scaled to fit the long edge.
        assert_eq!(1512.0 * scale(1512.0, 900.0, 2.0), 1568.0);
        // A tall one fits by its height.
        assert_eq!(2000.0 * scale(500.0, 2000.0, 1.0), 1568.0);
        // A screen that reports no scale counts as one pixel a point.
        assert_eq!(scale(300.0, 200.0, 0.0), 1.0);
    }

    #[test]
    fn the_window_is_found_on_its_screen() {
        let rect =
            |x, y, width, height| CGRect::new(CGPoint::new(x, y), CGSize::new(width, height));
        // A second screen to the right of a 1512-point-wide one.
        let screen = rect(1512.0, 0.0, 1920.0, 1080.0);
        let frame = rect(1600.0, 100.0, 800.0, 600.0);
        assert_eq!(
            window_on_screen(frame, screen),
            rect(88.0, 100.0, 800.0, 600.0)
        );
        assert_eq!(overlap(screen, frame), 800.0 * 600.0);
        // Half on the first screen, half off.
        assert_eq!(
            overlap(
                rect(0.0, 0.0, 1512.0, 982.0),
                rect(1312.0, 0.0, 400.0, 100.0)
            ),
            200.0 * 100.0
        );
        assert_eq!(overlap(rect(0.0, 0.0, 1512.0, 982.0), frame), 0.0);
    }

    #[test]
    fn the_overlay_gets_a_data_url() {
        let picture = Picture {
            width: 2,
            height: 1,
            jpeg: vec![0xff, 0xd8, 0xff],
        };
        let json = serde_json::to_value(&picture).unwrap();
        assert_eq!(json["dataUrl"], "data:image/jpeg;base64,/9j/");
        assert_eq!(json["bytes"], 3);
        assert_eq!(format!("{picture:?}"), "Picture(2x1, 3 bytes)");
    }
}
