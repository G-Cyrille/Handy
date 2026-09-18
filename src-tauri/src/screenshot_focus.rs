//! Best-effort geometry of the focused text input. Never reads its text.
use objc2_app_kit::NSWorkspace;
use std::ffi::{c_void, CString};
use std::ptr;

type CFRef = *const c_void;

#[repr(C)]
#[derive(Default)]
struct Point {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Default)]
struct Size {
    width: f64,
    height: f64,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> CFRef;
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementCopyAttributeValue(element: CFRef, attribute: CFRef, value: *mut CFRef) -> i32;
    fn AXUIElementSetMessagingTimeout(element: CFRef, timeout: f32) -> i32;
    fn AXValueGetTypeID() -> usize;
    fn AXValueGetValue(value: CFRef, kind: u32, output: *mut c_void) -> bool;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(allocator: CFRef, text: *const i8, encoding: u32) -> CFRef;
    fn CFRelease(value: CFRef);
    fn CFGetTypeID(value: CFRef) -> usize;
    fn CFEqual(a: CFRef, b: CFRef) -> bool;
}

/// Own a Core Foundation Create/Copy result, releasing it on every return path.
struct OwnedCF(CFRef);
impl Drop for OwnedCF {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}
impl OwnedCF {
    fn new(value: CFRef) -> Option<Self> {
        (!value.is_null()).then(|| Self(value))
    }
    fn string(text: &str) -> Option<Self> {
        let text = CString::new(text).ok()?;
        Self::new(unsafe { CFStringCreateWithCString(ptr::null(), text.as_ptr(), 0x08000100) })
    }
    fn attribute(&self, name: &str) -> Option<Self> {
        let attribute = Self::string(name)?;
        let mut value = ptr::null();
        let status = unsafe { AXUIElementCopyAttributeValue(self.0, attribute.0, &mut value) };
        let value = Self::new(value);
        if status == 0 {
            value
        } else {
            log::debug!("Screenshot focus: {name} returned AXError {status}");
            None
        }
    }
    fn equals_string(&self, text: &str) -> bool {
        Self::string(text).is_some_and(|other| unsafe { CFEqual(self.0, other.0) })
    }
}

/// Desktop coordinates use the same top-left origin as Core Graphics captures.
/// Unsupported roles and secure text inputs deliberately produce no annotation.
pub fn focused_text_bounds() -> Option<(f64, f64, f64, f64)> {
    unsafe {
        if !AXIsProcessTrusted() {
            log::warn!("Screenshot focus unavailable: Accessibility permission is not granted");
            return None;
        }
        // Query the foreground process directly. The system-wide AX object can
        // return CannotComplete even when that application's focused input is accessible.
        let foreground = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let pid = foreground.processIdentifier();
        let application = OwnedCF::new(AXUIElementCreateApplication(pid))?;
        if AXUIElementSetMessagingTimeout(application.0, 0.15) != 0 {
            return None;
        }
        let Some(element) = application.attribute("AXFocusedUIElement") else {
            log::warn!(
                "Screenshot focus unavailable: application {pid} did not expose a focused element"
            );
            return None;
        };
        if AXUIElementSetMessagingTimeout(element.0, 0.15) != 0 {
            return None;
        }
        if CFGetTypeID(element.0) != AXUIElementGetTypeID() {
            return None;
        }
        let role = element.attribute("AXRole")?;
        if !["AXTextField", "AXTextArea", "AXComboBox"]
            .iter()
            .any(|role_name| role.equals_string(role_name))
        {
            return None;
        }
        if element
            .attribute("AXSubrole")
            .is_some_and(|role| role.equals_string("AXSecureTextField"))
        {
            return None;
        }
        let Some(position) = element.attribute("AXPosition") else {
            log::warn!(
                "Screenshot focus unavailable: text input in application {pid} has no position"
            );
            return None;
        };
        let Some(size) = element.attribute("AXSize") else {
            log::warn!("Screenshot focus unavailable: text input in application {pid} has no size");
            return None;
        };
        if CFGetTypeID(position.0) != AXValueGetTypeID()
            || CFGetTypeID(size.0) != AXValueGetTypeID()
        {
            return None;
        }
        let mut point = Point::default();
        let mut dimensions = Size::default();
        // AXValue checks the requested kind before writing into the matching C struct.
        if !AXValueGetValue(position.0, 1, (&mut point as *mut Point).cast())
            || !AXValueGetValue(size.0, 2, (&mut dimensions as *mut Size).cast())
        {
            return None;
        }
        let bounds = (point.x, point.y, dimensions.width, dimensions.height);
        if [bounds.0, bounds.1, bounds.2, bounds.3]
            .iter()
            .all(|value| value.is_finite())
            && bounds.2 > 0.0
            && bounds.3 > 0.0
        {
            log::debug!("Screenshot focus: detected text input in application {pid}");
            Some(bounds)
        } else {
            None
        }
    }
}
