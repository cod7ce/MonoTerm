//! Native chrome controls absent from the pinned GPUI API.
#![allow(unexpected_cfgs)] // objc 0.2 expands its legacy cargo-clippy cfg in callers.

#[cfg(target_os = "macos")]
mod mac {
    use objc::{
        msg_send,
        runtime::{BOOL, Class, NO, Object, YES},
        sel, sel_impl,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::c_void;

    fn with_window<T>(
        window: &gpui::Window,
        update: impl FnOnce(*mut Object) -> Result<T, String>,
    ) -> Result<T, String> {
        let class = Class::get("NSThread").ok_or("NSThread is unavailable")?;
        // SAFETY: Foundation permits querying the current thread from any thread.
        let main: BOOL = unsafe { msg_send![class, isMainThread] };
        if main == NO {
            return Err("Window appearance requires the main thread".into());
        }
        let handle = HasWindowHandle::window_handle(window).map_err(|e| e.to_string())?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err("Expected an AppKit window".into());
        };
        let view = handle.ns_view.as_ptr().cast::<Object>();
        // SAFETY: GPUI's borrowed window owns this live NSView. Access is synchronous
        // on the main thread; no pointer is retained beyond this function.
        let native: *mut Object = unsafe { msg_send![view, window] };
        if native.is_null() {
            return Err("Native view has no window".into());
        }
        update(native)
    }

    pub fn set_application_icon() -> Result<(), String> {
        let thread = Class::get("NSThread").ok_or("NSThread is unavailable")?;
        // SAFETY: Foundation permits querying the current thread from any thread.
        let main: BOOL = unsafe { msg_send![thread, isMainThread] };
        if main == NO {
            return Err("Application icon requires the main thread".into());
        }
        let data_class = Class::get("NSData").ok_or("NSData is unavailable")?;
        let image_class = Class::get("NSImage").ok_or("NSImage is unavailable")?;
        let application_class =
            Class::get("NSApplication").ok_or("NSApplication is unavailable")?;
        let bytes = include_bytes!("../assets/AppIcon.icns");
        // SAFETY: NSData copies the embedded bytes. NSImage owns its decoded data;
        // NSApplication retains the assigned image before our ownership is released.
        unsafe {
            let data: *mut Object = msg_send![data_class,
                dataWithBytes: bytes.as_ptr().cast::<c_void>() length: bytes.len()];
            if data.is_null() {
                return Err("Cannot allocate application icon data".into());
            }
            let image: *mut Object = msg_send![image_class, alloc];
            let image: *mut Object = msg_send![image, initWithData: data];
            if image.is_null() {
                return Err("Cannot decode application icon".into());
            }
            let application: *mut Object = msg_send![application_class, sharedApplication];
            // Unbundled executable launches have no bundle icon; explicit assignment
            // also avoids a stale Dock image after rebuilding an existing bundle.
            let _: () = msg_send![application, setApplicationIconImage: image];
            let _: () = msg_send![image, release];
        }
        Ok(())
    }

    pub fn set_zen(window: &gpui::Window, zen: bool) -> Result<(), String> {
        with_window(window, |native| {
            // SAFETY: Standard NSWindowButton values are close=0, minimize=1, zoom=2.
            let buttons: [*mut Object; 3] =
                [0_u64, 1, 2].map(|kind| unsafe { msg_send![native, standardWindowButton: kind] });
            if buttons.iter().any(|b| b.is_null()) {
                return Err("Native titlebar controls are unavailable".into());
            }
            for button in buttons {
                let hidden: BOOL = if zen { YES } else { NO };
                // SAFETY: These NSButtons are owned by the live native window.
                unsafe {
                    let _: () = msg_send![button, setHidden: hidden];
                }
            }
            Ok(())
        })
    }

    pub fn set_blur_radius(window: &gpui::Window, radius: u32) -> Result<(), String> {
        if radius > 64 {
            return Err("Blur radius must be within 0–64".into());
        }
        with_window(window, |native| {
            type Connection = unsafe extern "C" fn() -> *mut c_void;
            type SetRadius = unsafe extern "C" fn(*mut c_void, isize, i64) -> i32;
            // SAFETY: CoreGraphics is already loaded by GPUI. These private SPI
            // signatures match GPUI's pinned macOS backend; resolve at runtime so
            // an unavailable symbol is an error rather than a launch failure.
            let (connection, radius_fn) = unsafe {
                (
                    libc::dlsym(libc::RTLD_DEFAULT, c"CGSMainConnectionID".as_ptr()),
                    libc::dlsym(
                        libc::RTLD_DEFAULT,
                        c"CGSSetWindowBackgroundBlurRadius".as_ptr(),
                    ),
                )
            };
            if connection.is_null() || radius_fn.is_null() {
                return Err("Numeric backdrop blur is unavailable".into());
            }
            // SAFETY: Non-null symbols have the signatures declared above and
            // their framework remains loaded throughout this synchronous call.
            let status = unsafe {
                let connection = std::mem::transmute::<*mut c_void, Connection>(connection);
                let set_radius = std::mem::transmute::<*mut c_void, SetRadius>(radius_fn);
                let number: isize = msg_send![native, windowNumber];
                set_radius(connection(), number, i64::from(radius))
            };
            if status == 0 {
                Ok(())
            } else {
                Err(format!("macOS rejected backdrop blur ({status})"))
            }
        })
    }
}

#[cfg(target_os = "macos")]
pub use mac::{set_application_icon, set_blur_radius, set_zen};

pub fn apply_material(window: &mut gpui::Window, alpha: f32, radius: u32) -> Result<(), String> {
    use gpui::WindowBackgroundAppearance;
    window.set_background_appearance(if alpha < 1.0 {
        WindowBackgroundAppearance::Transparent
    } else {
        WindowBackgroundAppearance::Opaque
    });
    set_blur_radius(window, if alpha < 1.0 { radius } else { 0 })
}
