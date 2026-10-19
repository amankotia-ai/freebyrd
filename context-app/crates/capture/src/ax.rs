//! Safe wrapper over the macOS Accessibility (`AXUIElement`) C API.
//!
//! Every attribute read is a cross-process call into the target app, so
//! callers should treat each one as slow and fallible.

use std::ffi::c_void;
use std::fmt;

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_foundation::url::CFURL;
use core_foundation_sys::array::{
    CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex, CFArrayRef,
};
use core_foundation_sys::base::{CFGetTypeID, CFTypeID, CFTypeRef};
use core_foundation_sys::dictionary::CFDictionaryRef;
use core_foundation_sys::string::CFStringRef;

type AXUIElementRef = CFTypeRef;
type AXValueRef = CFTypeRef;
type AXErrorCode = i32;

const AX_SUCCESS: AXErrorCode = 0;
const AX_INVALID_UI_ELEMENT: AXErrorCode = -25202;
const AX_CANNOT_COMPLETE: AXErrorCode = -25204;
const AX_ATTRIBUTE_UNSUPPORTED: AXErrorCode = -25205;
const AX_NOT_IMPLEMENTED: AXErrorCode = -25208;
const AX_API_DISABLED: AXErrorCode = -25211;
const AX_NO_VALUE: AXErrorCode = -25212;

const AX_VALUE_CG_POINT: u32 = 1;
const AX_VALUE_CG_SIZE: u32 = 2;
const AX_VALUE_CF_RANGE: u32 = 4;

#[repr(C)]
#[derive(Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Default)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Default)]
struct CFRange {
    location: isize,
    length: isize,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    static kAXTrustedCheckOptionPrompt: CFStringRef;

    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXErrorCode;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> AXErrorCode;
    fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> AXErrorCode;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXErrorCode;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, seconds: f32) -> AXErrorCode;
    fn AXUIElementGetTypeID() -> CFTypeID;
    fn AXValueGetTypeID() -> CFTypeID;
    fn AXValueGetType(value: AXValueRef) -> u32;
    fn AXValueGetValue(value: AXValueRef, value_type: u32, out: *mut c_void) -> u8;
}

/// Failures worth distinguishing. Missing or unsupported attributes are not
/// errors; reads return `Ok(None)` for those.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxError {
    /// This process has not been granted Accessibility access.
    ApiDisabled,
    /// The element no longer exists (window closed, app quit).
    InvalidElement,
    /// The target app did not answer in time or is busy.
    CannotComplete,
    NotImplemented,
    Other(i32),
}

impl AxError {
    fn from_code(code: AXErrorCode) -> Self {
        match code {
            AX_API_DISABLED => Self::ApiDisabled,
            AX_INVALID_UI_ELEMENT => Self::InvalidElement,
            AX_CANNOT_COMPLETE => Self::CannotComplete,
            AX_NOT_IMPLEMENTED => Self::NotImplemented,
            other => Self::Other(other),
        }
    }
}

impl fmt::Display for AxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiDisabled => write!(f, "Accessibility access is not granted to this process"),
            Self::InvalidElement => write!(f, "the element no longer exists"),
            Self::CannotComplete => write!(f, "the target app did not respond"),
            Self::NotImplemented => write!(f, "the target app does not implement this"),
            Self::Other(code) => write!(f, "Accessibility error {code}"),
        }
    }
}

impl std::error::Error for AxError {}

/// Whether this process may use the Accessibility API. With `prompt`, macOS
/// shows its permission dialog when access is missing.
///
/// macOS attributes the request to the "responsible" app, which for a
/// command-line tool is usually the terminal that launched it.
pub fn is_trusted(prompt: bool) -> bool {
    let key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
    let options = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::from(prompt).as_CFType())]);
    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) != 0 }
}

/// A decoded attribute value.
#[derive(Debug, Clone)]
pub enum Value {
    String(String),
    Bool(bool),
    Number(f64),
    Element(Element),
    Array(Vec<Value>),
    Point {
        x: f64,
        y: f64,
    },
    Size {
        width: f64,
        height: f64,
    },
    Url(String),
    /// A span of text, in UTF-16 code units, such as the characters a text
    /// view shows (`AXVisibleCharacterRange`).
    Range {
        location: usize,
        length: usize,
    },
    /// A type this wrapper does not decode yet (attributed strings, rects).
    Other,
}

impl Value {
    fn from_cf(cf: CFType) -> Self {
        let raw = cf.as_CFTypeRef();
        let type_id = unsafe { CFGetTypeID(raw) };
        if type_id == CFString::type_id() {
            Self::String(unsafe { CFString::wrap_under_get_rule(raw as _) }.to_string())
        } else if type_id == CFBoolean::type_id() {
            Self::Bool(unsafe { CFBoolean::wrap_under_get_rule(raw as _) }.into())
        } else if type_id == CFNumber::type_id() {
            let number = unsafe { CFNumber::wrap_under_get_rule(raw as _) };
            number.to_f64().map_or(Self::Other, Self::Number)
        } else if type_id == unsafe { AXUIElementGetTypeID() } {
            Self::Element(Element(cf))
        } else if type_id == unsafe { CFArrayGetTypeID() } {
            let array = raw as CFArrayRef;
            let count = unsafe { CFArrayGetCount(array) };
            let items = (0..count)
                .map(|i| {
                    let item = unsafe { CFArrayGetValueAtIndex(array, i) };
                    Self::from_cf(unsafe { CFType::wrap_under_get_rule(item) })
                })
                .collect();
            Self::Array(items)
        } else if type_id == unsafe { AXValueGetTypeID() } {
            Self::from_ax_value(raw)
        } else if type_id == CFURL::type_id() {
            let url = unsafe { CFURL::wrap_under_get_rule(raw as _) };
            Self::Url(url.get_string().to_string())
        } else {
            Self::Other
        }
    }

    fn from_ax_value(raw: AXValueRef) -> Self {
        unsafe {
            match AXValueGetType(raw) {
                AX_VALUE_CG_POINT => {
                    let mut point = CGPoint::default();
                    if AXValueGetValue(raw, AX_VALUE_CG_POINT, (&raw mut point).cast()) != 0 {
                        return Self::Point {
                            x: point.x,
                            y: point.y,
                        };
                    }
                }
                AX_VALUE_CG_SIZE => {
                    let mut size = CGSize::default();
                    if AXValueGetValue(raw, AX_VALUE_CG_SIZE, (&raw mut size).cast()) != 0 {
                        return Self::Size {
                            width: size.width,
                            height: size.height,
                        };
                    }
                }
                AX_VALUE_CF_RANGE => {
                    let mut range = CFRange::default();
                    if AXValueGetValue(raw, AX_VALUE_CF_RANGE, (&raw mut range).cast()) != 0
                        && let (Ok(location), Ok(length)) = (
                            usize::try_from(range.location),
                            usize::try_from(range.length),
                        )
                    {
                        return Self::Range { location, length };
                    }
                }
                _ => {}
            }
        }
        Self::Other
    }
}

/// A reference to one UI element in another process.
#[derive(Debug, Clone)]
pub struct Element(CFType);

impl Element {
    /// Takes ownership of a reference returned by a `Create`/`Copy` call.
    unsafe fn from_create_rule(raw: AXUIElementRef) -> Option<Self> {
        (!raw.is_null()).then(|| Self(unsafe { CFType::wrap_under_create_rule(raw) }))
    }

    fn raw(&self) -> AXUIElementRef {
        self.0.as_CFTypeRef()
    }

    /// The top-level element of a running app.
    pub fn application(pid: i32) -> Option<Self> {
        unsafe { Self::from_create_rule(AXUIElementCreateApplication(pid)) }
    }

    /// The element that represents the whole system, for global queries such
    /// as the focused app.
    pub fn system_wide() -> Option<Self> {
        unsafe { Self::from_create_rule(AXUIElementCreateSystemWide()) }
    }

    /// Caps how long each call waits for the target app. On the system-wide
    /// element this sets the default for every element.
    pub fn set_messaging_timeout(&self, seconds: f32) {
        unsafe {
            AXUIElementSetMessagingTimeout(self.raw(), seconds);
        }
    }

    pub fn pid(&self) -> Option<i32> {
        let mut pid = 0;
        let code = unsafe { AXUIElementGetPid(self.raw(), &mut pid) };
        (code == AX_SUCCESS).then_some(pid)
    }

    pub fn attribute(&self, name: &str) -> Result<Option<Value>, AxError> {
        let key = CFString::new(name);
        let mut value: CFTypeRef = std::ptr::null();
        let code = unsafe {
            AXUIElementCopyAttributeValue(self.raw(), key.as_concrete_TypeRef(), &mut value)
        };
        match code {
            AX_SUCCESS if !value.is_null() => Ok(Some(Value::from_cf(unsafe {
                CFType::wrap_under_create_rule(value)
            }))),
            AX_SUCCESS | AX_NO_VALUE | AX_ATTRIBUTE_UNSUPPORTED => Ok(None),
            other => Err(AxError::from_code(other)),
        }
    }

    /// A string attribute, or `None` when it is missing, not a string, or
    /// the read failed.
    pub fn string(&self, name: &str) -> Option<String> {
        match self.attribute(name) {
            Ok(Some(Value::String(s))) => Some(s),
            _ => None,
        }
    }

    pub fn element(&self, name: &str) -> Option<Element> {
        match self.attribute(name) {
            Ok(Some(Value::Element(e))) => Some(e),
            _ => None,
        }
    }

    pub fn url(&self, name: &str) -> Option<String> {
        match self.attribute(name) {
            Ok(Some(Value::Url(url))) => Some(url),
            _ => None,
        }
    }

    /// An attribute holding a list of elements, such as `AXChildren` or
    /// `AXWindows`. Non-element entries are skipped.
    pub fn elements(&self, name: &str) -> Result<Vec<Element>, AxError> {
        Ok(match self.attribute(name)? {
            Some(Value::Array(items)) => items
                .into_iter()
                .filter_map(|v| match v {
                    Value::Element(e) => Some(e),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        })
    }

    /// Presses the element, as a click on a button would (`AXPress`).
    pub fn press(&self) -> Result<(), AxError> {
        let action = CFString::new("AXPress");
        let code = unsafe { AXUIElementPerformAction(self.raw(), action.as_concrete_TypeRef()) };
        if code == AX_SUCCESS {
            Ok(())
        } else {
            Err(AxError::from_code(code))
        }
    }

    pub fn set_bool(&self, name: &str, value: bool) -> Result<(), AxError> {
        let key = CFString::new(name);
        let value = CFBoolean::from(value);
        let code = unsafe {
            AXUIElementSetAttributeValue(
                self.raw(),
                key.as_concrete_TypeRef(),
                value.as_CFTypeRef(),
            )
        };
        if code == AX_SUCCESS {
            Ok(())
        } else {
            Err(AxError::from_code(code))
        }
    }
}
