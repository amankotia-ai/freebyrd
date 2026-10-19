//! Sends a synthetic double tap of the left Option key, for testing the
//! overlay's trigger without touching the keyboard. Needs Accessibility access.
//! `--activate PID` brings that app to the front first.

use std::ffi::c_void;
use std::thread;
use std::time::Duration;

const LEFT_OPTION: u16 = 58;
const FLAGS_CHANGED: u32 = 12;
const FLAG_OPTION: u64 = 0x8_0000;
const HID_EVENT_TAP: u32 = 0;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateKeyboardEvent(
        source: *const c_void,
        keycode: u16,
        key_down: bool,
    ) -> *mut c_void;
    fn CGEventSetType(event: *mut c_void, event_type: u32);
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
    fn CFRelease(object: *const c_void);
}

fn option(down: bool) {
    unsafe {
        let event = CGEventCreateKeyboardEvent(std::ptr::null(), LEFT_OPTION, down);
        assert!(!event.is_null(), "could not create a keyboard event");
        CGEventSetType(event, FLAGS_CHANGED);
        CGEventSetFlags(event, if down { FLAG_OPTION } else { 0 });
        CGEventPost(HID_EVENT_TAP, event);
        CFRelease(event);
    }
}

/// `tap-sim [--activate PID]`: optionally bring an app to the front first,
/// through Accessibility, since `open` from a background process may not.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, pid] = args.as_slice()
        && flag == "--activate"
    {
        let pid: i32 = pid.parse().expect("--activate needs a process ID");
        let app = capture::ax::Element::application(pid).expect("no such app");
        app.set_bool("AXFrontmost", true)
            .expect("could not bring the app to the front");
        thread::sleep(Duration::from_millis(400));
    }
    let taps = if args.iter().any(|a| a == "--single") {
        1
    } else {
        2
    };
    for tap in 0..taps {
        option(true);
        thread::sleep(Duration::from_millis(60));
        option(false);
        if tap + 1 < taps {
            thread::sleep(Duration::from_millis(120));
        }
    }
    println!("sent {taps} tap(s) of Option");
}
