//! Lets the closed island be clicked where it shows, as the Dynamic Island
//! can be tapped, while clicks anywhere else around it still reach the menu
//! bar. The closed overlay ignores clicks (`glow` in main.rs); this watches
//! the pointer and says when it is over the island's shape, so the window
//! takes clicks only there. It reads where the pointer is, nothing else:
//! no clicks, no keys.

use std::ffi::c_void;
use std::thread;
use std::time::Duration;

/// The closed island's shape, in global points with the origin at the top
/// left of the main screen (as `CGEventGetLocation` gives the pointer).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Shape {
    fn contains(&self, (x, y): (f64, f64)) -> bool {
        (self.left..=self.right).contains(&x) && (self.top..=self.bottom).contains(&y)
    }

    /// How far a point is from the shape, 0 inside it.
    fn distance(&self, (x, y): (f64, f64)) -> f64 {
        let dx = (self.left - x).max(x - self.right).max(0.0);
        let dy = (self.top - y).max(y - self.bottom).max(0.0);
        dx.hypot(dy)
    }
}

/// Pointer checks: often when it is near the island, so it answers before
/// a click; seldom when it is far, or nothing shows.
const NEAR: Duration = Duration::from_millis(40);
const FAR: Duration = Duration::from_millis(250);
const NOTHING: Duration = Duration::from_millis(500);
/// Within this many points of the island counts as near.
const NEAR_POINTS: f64 = 160.0;

/// Watches the pointer on a thread of its own. `shape` gives the closed
/// island's shape while it shows; `over` hears, on every check, whether the
/// pointer is over it (it acts only on a change).
pub fn watch(
    shape: impl Fn() -> Option<Shape> + Send + 'static,
    over: impl Fn(bool) + Send + 'static,
) {
    let spawned = thread::Builder::new()
        .name("hover".to_owned())
        .spawn(move || {
            loop {
                let wait = match (shape(), pointer()) {
                    (Some(shape), Some(at)) => {
                        over(shape.contains(at));
                        pause(shape.distance(at))
                    }
                    _ => {
                        over(false);
                        NOTHING
                    }
                };
                thread::sleep(wait);
            }
        });
    if let Err(e) = spawned {
        eprintln!("freebyrd: the closed island cannot be clicked: {e}");
    }
}

fn pause(distance: f64) -> Duration {
    if distance <= NEAR_POINTS { NEAR } else { FAR }
}

/// Where the pointer is now.
fn pointer() -> Option<(f64, f64)> {
    // SAFETY: an event made from no source holds the pointer's position;
    // it is released here and nothing else keeps it.
    unsafe {
        let event = CGEventCreate(std::ptr::null_mut());
        if event.is_null() {
            return None;
        }
        let at = CGEventGetLocation(event);
        CFRelease(event);
        Some((at.x, at.y))
    }
}

#[repr(C)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *mut c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: *mut c_void);
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISLAND: Shape = Shape {
        left: 600.0,
        top: 0.0,
        right: 900.0,
        bottom: 32.0,
    };

    #[test]
    fn over_the_island_and_its_edges_counts_and_the_menu_bar_beside_it_does_not() {
        assert!(ISLAND.contains((750.0, 10.0)));
        assert!(ISLAND.contains((600.0, 32.0)));
        assert!(!ISLAND.contains((599.0, 10.0)));
        assert!(!ISLAND.contains((750.0, 33.0)));
    }

    #[test]
    fn checks_often_only_near_the_island() {
        assert_eq!(ISLAND.distance((750.0, 10.0)), 0.0);
        assert_eq!(ISLAND.distance((500.0, 10.0)), 100.0);
        assert_eq!(pause(ISLAND.distance((500.0, 10.0))), NEAR);
        assert_eq!(pause(ISLAND.distance((750.0, 600.0))), FAR);
    }

    #[test]
    fn the_pointer_can_be_read() {
        // A Mac always has a pointer position, even with no mouse.
        assert!(pointer().is_some());
    }
}
