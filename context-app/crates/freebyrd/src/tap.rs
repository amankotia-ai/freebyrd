//! Detects, anywhere in macOS, a double tap of the Option key (the overlay)
//! and Command and Option pressed together on their own and let go (the
//! to-do list, the person's ask of 2026-10-13).
//!
//! A listen-only event tap watches modifier changes, key presses and mouse
//! clicks. Ordinary key presses and clicks only cancel a gesture in
//! progress; which key was pressed is never read or kept, only which
//! modifiers are down.

use std::cell::RefCell;
use std::ffi::c_void;
use std::fmt;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use core_foundation_sys::mach_port::{CFMachPortCreateRunLoopSource, CFMachPortRef};
use core_foundation_sys::runloop::{
    CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRun, kCFRunLoopCommonModes,
};

/// Longest press that still counts as a tap rather than a held modifier.
const MAX_PRESS: Duration = Duration::from_millis(250);
/// Longest pause between the first tap's release and the second tap's release.
const MAX_GAP: Duration = Duration::from_millis(400);
/// Longest time from the first of Command and Option going down to both
/// being let go, for the pair to count as the to-do list's shortcut rather
/// than modifiers held for something else.
const MAX_CHORD: Duration = Duration::from_millis(800);

/// What the person did on the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    /// Option tapped twice: the overlay.
    DoubleTap,
    /// Command and Option together, on their own: the to-do list.
    Tasks,
}

/// Command and Option pressed together and let go with nothing else in
/// between: no other modifier, key or click, so ⌘⌥Esc, ⌘⌥D or a ⌘⌥-drag
/// never count.
#[derive(Debug, Default)]
pub struct Chord {
    started_at: Option<Instant>,
    both: bool,
    broken: bool,
}

impl Chord {
    /// The modifiers now down (from a modifier change). Returns true when
    /// both are let go and the pair counts.
    pub fn modifiers(&mut self, command: bool, option: bool, others: bool, now: Instant) -> bool {
        if !command && !option {
            let counts = self.both
                && !self.broken
                && !others
                && self
                    .started_at
                    .is_some_and(|at| now.duration_since(at) <= MAX_CHORD);
            *self = Self::default();
            return counts;
        }
        if self.started_at.is_none() {
            self.started_at = Some(now);
        }
        self.both |= command && option;
        self.broken |= others;
        false
    }

    /// A key press or click while the modifiers are held.
    pub fn interrupted(&mut self) {
        if self.started_at.is_some() {
            self.broken = true;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// The watched modifier went down with no other modifier held.
    Down,
    /// The watched modifier went up.
    Up,
    /// Anything else: another modifier, a key press, a click.
    Other,
}

/// Turns a stream of inputs into double-tap detections.
#[derive(Debug, Default)]
pub struct DoubleTap {
    pressed_at: Option<Instant>,
    last_tap_at: Option<Instant>,
}

impl DoubleTap {
    /// Whether a tap is under way, so that other input would cancel it.
    pub fn in_progress(&self) -> bool {
        self.pressed_at.is_some() || self.last_tap_at.is_some()
    }

    /// Returns true when `input` completes a double tap.
    pub fn feed(&mut self, input: Input, now: Instant) -> bool {
        match input {
            Input::Down => {
                self.pressed_at = Some(now);
                false
            }
            Input::Up => {
                let Some(pressed_at) = self.pressed_at.take() else {
                    return false;
                };
                if now.duration_since(pressed_at) > MAX_PRESS {
                    self.last_tap_at = None;
                    return false;
                }
                match self.last_tap_at.take() {
                    Some(previous) if now.duration_since(previous) <= MAX_GAP => true,
                    _ => {
                        self.last_tap_at = Some(now);
                        false
                    }
                }
            }
            Input::Other => {
                *self = Self::default();
                false
            }
        }
    }
}

// CoreGraphics event tap API.
type CGEventRef = *mut c_void;
type CGEventTapProxy = *mut c_void;
type TapCallback =
    unsafe extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;

const SESSION_EVENT_TAP: u32 = 1;
const HEAD_INSERT_EVENT_TAP: u32 = 0;
const LISTEN_ONLY: u32 = 1;

const LEFT_MOUSE_DOWN: u32 = 1;
const RIGHT_MOUSE_DOWN: u32 = 3;
const KEY_DOWN: u32 = 10;
const FLAGS_CHANGED: u32 = 12;
const OTHER_MOUSE_DOWN: u32 = 25;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

const KEYBOARD_EVENT_KEYCODE: u32 = 9;
const LEFT_OPTION: i64 = 58;
const RIGHT_OPTION: i64 = 61;

const FLAG_SHIFT: u64 = 0x2_0000;
const FLAG_CONTROL: u64 = 0x4_0000;
const FLAG_OPTION: u64 = 0x8_0000;
const FLAG_COMMAND: u64 = 0x10_0000;
const FLAG_FN: u64 = 0x80_0000;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

#[derive(Debug)]
pub enum TapError {
    /// macOS refused the event tap; Input Monitoring is probably not granted.
    NotPermitted,
    ThreadFailed,
}

impl fmt::Display for TapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPermitted => write!(
                f,
                "macOS refused to watch the keyboard. Allow the app that launched the overlay in \
                 System Settings > Privacy & Security > Input Monitoring, then restart the overlay."
            ),
            Self::ThreadFailed => write!(f, "the keyboard watcher thread stopped unexpectedly"),
        }
    }
}

impl std::error::Error for TapError {}

/// State reachable from the C callback. It lives for the whole process and is
/// only touched on the tap thread.
struct Watcher {
    detector: RefCell<DoubleTap>,
    chord: RefCell<Chord>,
    port: RefCell<CFMachPortRef>,
    detected: Sender<Gesture>,
    /// Set by `FREEBYRD_DEBUG_TAP=1`: log Option presses and tap state.
    debug_since: Option<Instant>,
}

impl Watcher {
    fn debug(&self, now: Instant, message: &str) {
        if let Some(start) = self.debug_since {
            eprintln!(
                "tap: {:>7} ms  {message}",
                now.duration_since(start).as_millis()
            );
        }
    }
}

/// Calls `on_gesture` on a background thread each time Option is double
/// tapped or Command and Option are pressed together on their own. Returns
/// once the watcher is running, or with the reason it could not start.
pub fn watch(on_gesture: impl Fn(Gesture) + Send + 'static) -> Result<(), TapError> {
    let (detected, detections) = mpsc::channel::<Gesture>();
    let (started, start_result) = mpsc::channel();

    thread::Builder::new()
        .name("double-tap watcher".to_owned())
        .spawn(move || {
            // Asks macOS to show its Input Monitoring prompt the first time.
            if !unsafe { CGPreflightListenEventAccess() } {
                unsafe { CGRequestListenEventAccess() };
            }
            let watcher: &'static Watcher = Box::leak(Box::new(Watcher {
                detector: RefCell::new(DoubleTap::default()),
                chord: RefCell::new(Chord::default()),
                port: RefCell::new(std::ptr::null_mut()),
                detected,
                debug_since: std::env::var_os("FREEBYRD_DEBUG_TAP").map(|_| Instant::now()),
            }));
            let mask = [
                LEFT_MOUSE_DOWN,
                RIGHT_MOUSE_DOWN,
                OTHER_MOUSE_DOWN,
                KEY_DOWN,
                FLAGS_CHANGED,
            ]
            .iter()
            .fold(0u64, |mask, event| mask | 1 << event);
            let port = unsafe {
                CGEventTapCreate(
                    SESSION_EVENT_TAP,
                    HEAD_INSERT_EVENT_TAP,
                    LISTEN_ONLY,
                    mask,
                    on_event,
                    std::ptr::from_ref(watcher).cast_mut().cast(),
                )
            };
            if port.is_null() {
                let _ = started.send(Err(TapError::NotPermitted));
                return;
            }
            *watcher.port.borrow_mut() = port;
            unsafe {
                let source = CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0);
                CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
                CGEventTapEnable(port, true);
            }
            let _ = started.send(Ok(()));
            unsafe { CFRunLoopRun() };
        })
        .map_err(|_| TapError::ThreadFailed)?;

    start_result
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| TapError::ThreadFailed)??;

    // Handle detections off the tap thread so the tap callback stays fast;
    // macOS disables taps whose callbacks are slow.
    thread::Builder::new()
        .name("double-tap handler".to_owned())
        .spawn(move || {
            for gesture in detections {
                on_gesture(gesture);
            }
        })
        .map_err(|_| TapError::ThreadFailed)?;
    Ok(())
}

unsafe extern "C" fn on_event(
    _proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    let watcher = unsafe { &*user_info.cast::<Watcher>() };
    let now = Instant::now();
    let input = match event_type {
        TAP_DISABLED_BY_TIMEOUT | TAP_DISABLED_BY_USER_INPUT => {
            watcher.debug(
                now,
                &format!("macOS disabled the tap ({event_type:#x}); re-enabling"),
            );
            unsafe { CGEventTapEnable(*watcher.port.borrow(), true) };
            return event;
        }
        FLAGS_CHANGED => {
            let keycode = unsafe { CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE) };
            let flags = unsafe { CGEventGetFlags(event) };
            let others = FLAG_SHIFT | FLAG_CONTROL | FLAG_COMMAND | FLAG_FN;
            let input = match (keycode, flags & FLAG_OPTION != 0) {
                (LEFT_OPTION | RIGHT_OPTION, true) if flags & others == 0 => Input::Down,
                (LEFT_OPTION | RIGHT_OPTION, false) => Input::Up,
                _ => Input::Other,
            };
            if matches!(keycode, LEFT_OPTION | RIGHT_OPTION) {
                watcher.debug(now, &format!("option {input:?} flags={flags:#x}"));
            }
            // Only which modifiers are down, never which key.
            let chord = watcher.chord.borrow_mut().modifiers(
                flags & FLAG_COMMAND != 0,
                flags & FLAG_OPTION != 0,
                flags & (FLAG_SHIFT | FLAG_CONTROL | FLAG_FN) != 0,
                now,
            );
            if chord {
                watcher.debug(now, "command and option: the to-do list");
                let _ = watcher.detected.send(Gesture::Tasks);
            }
            input
        }
        _ => {
            watcher.chord.borrow_mut().interrupted();
            Input::Other
        }
    };
    let mut detector = watcher.detector.borrow_mut();
    if input == Input::Other && detector.in_progress() {
        // Which key or button is never recorded, only that one interrupted.
        watcher.debug(now, "cancelled by another key, modifier or click");
    }
    if detector.feed(input, now) {
        watcher.debug(now, "double tap");
        let _ = watcher.detected.send(Gesture::DoubleTap);
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(steps: &[(u64, Input)]) -> Vec<bool> {
        let start = Instant::now();
        let mut detector = DoubleTap::default();
        steps
            .iter()
            .map(|&(ms, input)| detector.feed(input, start + Duration::from_millis(ms)))
            .collect()
    }

    use Input::{Down, Other, Up};

    #[test]
    fn two_quick_taps_trigger_once() {
        let fired = run(&[(0, Down), (80, Up), (200, Down), (280, Up)]);
        assert_eq!(fired, [false, false, false, true]);
    }

    #[test]
    fn a_third_tap_starts_over() {
        let fired = run(&[
            (0, Down),
            (80, Up),
            (200, Down),
            (280, Up),
            (400, Down),
            (480, Up),
        ]);
        assert_eq!(fired.iter().filter(|&&f| f).count(), 1);
    }

    #[test]
    fn taps_too_far_apart_do_not_trigger() {
        let fired = run(&[(0, Down), (80, Up), (600, Down), (680, Up)]);
        assert!(!fired.contains(&true));
    }

    #[test]
    fn a_held_option_is_not_a_tap() {
        let fired = run(&[(0, Down), (400, Up), (500, Down), (560, Up)]);
        assert!(!fired.contains(&true));
    }

    /// Command, Option and any other modifier down; `None` for a key press
    /// or click.
    type Mods = Option<(bool, bool, bool)>;

    /// Feeds modifier states at their times; returns when the chord counted.
    fn chord(steps: &[(u64, Mods)]) -> Vec<bool> {
        let start = Instant::now();
        let mut chord = Chord::default();
        steps
            .iter()
            .map(|&(ms, step)| match step {
                Some((command, option, others)) => {
                    chord.modifiers(command, option, others, start + Duration::from_millis(ms))
                }
                None => {
                    chord.interrupted();
                    false
                }
            })
            .collect()
    }

    #[test]
    fn command_and_option_on_their_own_open_the_list() {
        // Down together, let go one after the other.
        let fired = chord(&[
            (0, Some((true, false, false))),
            (30, Some((true, true, false))),
            (180, Some((false, true, false))),
            (200, Some((false, false, false))),
        ]);
        assert_eq!(fired, [false, false, false, true]);
    }

    #[test]
    fn a_shortcut_or_a_drag_with_command_and_option_does_not() {
        // ⌘⌥Esc: a key while held.
        let key = chord(&[
            (0, Some((true, true, false))),
            (100, None),
            (200, Some((false, false, false))),
        ]);
        assert!(!key.contains(&true));
        // Held for a while, as for a drag.
        let held = chord(&[
            (0, Some((true, true, false))),
            (1500, Some((false, false, false))),
        ]);
        assert!(!held.contains(&true));
        // With Shift too.
        let shift = chord(&[
            (0, Some((true, true, false))),
            (50, Some((true, true, true))),
            (150, Some((false, false, false))),
        ]);
        assert!(!shift.contains(&true));
        // Command alone, or Option alone.
        let alone = chord(&[
            (0, Some((true, false, false))),
            (100, Some((false, false, false))),
        ]);
        assert!(!alone.contains(&true));
    }

    #[test]
    fn a_key_press_between_taps_cancels() {
        // Option-Left twice, as when moving the cursor by words.
        let fired = run(&[
            (0, Down),
            (40, Other),
            (80, Up),
            (150, Down),
            (190, Other),
            (230, Up),
        ]);
        assert!(!fired.contains(&true));
    }
}
