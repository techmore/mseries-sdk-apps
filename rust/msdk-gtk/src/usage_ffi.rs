//! C boundary for embedding the SDK usage control in GTK hosts.
//! All calls, including destruction, must occur on the GTK main thread.
use crate::usage_control::UsageControl;
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use msdk::usage::{Coverage, Snapshot};
use std::ffi::{CStr, c_char};
use std::path::PathBuf;
use std::ptr;
use std::time::{SystemTime, UNIX_EPOCH};

/// An opaque host-owned usage control. The returned widget is borrowed and
/// stays valid until the handle is freed; the host must unparent it first.
pub struct MsdkUsageControl(UsageControl);

/// Create a control without starting a collector or making a read.
///
/// # Safety
/// `executable` must be a valid, NUL-terminated UTF-8 string for this call.
/// GTK must be initialized, and the caller must use the GTK main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn msdk_usage_control_new(
    executable: *const c_char,
) -> *mut MsdkUsageControl {
    if executable.is_null() {
        return ptr::null_mut();
    }
    // A Vala/C host can initialize GTK without initializing gtk-rs's own
    // main-thread bookkeeping. gtk::init is idempotent on the GTK thread.
    if gtk::init().is_err() {
        return ptr::null_mut();
    }
    // SAFETY: the caller promises a valid NUL-terminated string.
    let Ok(executable) = (unsafe { CStr::from_ptr(executable) }).to_str() else {
        return ptr::null_mut();
    };
    let executable = PathBuf::from(executable);
    if !executable.is_absolute() {
        return ptr::null_mut();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().min(i64::MAX as u64) as i64);
    let initial = Snapshot {
        coverage: Coverage::Unknown,
        observations: vec![],
    };
    match UsageControl::new(&initial, now, executable, 300) {
        Ok(control) => Box::into_raw(Box::new(MsdkUsageControl(control))),
        Err(_) => ptr::null_mut(),
    }
}

/// # Safety
/// `handle` must be a live handle returned by `msdk_usage_control_new`, on the GTK thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn msdk_usage_control_widget(
    handle: *mut MsdkUsageControl,
) -> *mut gtk::ffi::GtkWidget {
    // SAFETY: the caller guarantees handle validity; null is accepted.
    unsafe { handle.as_ref() }.map_or(ptr::null_mut(), |handle| {
        handle
            .0
            .widget()
            .upcast_ref::<gtk::Widget>()
            .to_glib_none()
            .0
    })
}

/// # Safety
/// `handle` must be a live handle returned by `msdk_usage_control_new`, on the GTK thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn msdk_usage_control_open(handle: *mut MsdkUsageControl) -> bool {
    // SAFETY: the caller guarantees handle validity; null is accepted.
    unsafe { handle.as_ref() }.is_some_and(|handle| handle.0.open().is_ok())
}

/// # Safety
/// `handle` must be a live handle returned by `msdk_usage_control_new`, on the GTK thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn msdk_usage_control_close(handle: *mut MsdkUsageControl) {
    // SAFETY: the caller guarantees handle validity; null is accepted.
    if let Some(handle) = unsafe { handle.as_ref() } {
        handle.0.close();
    }
}

/// # Safety
/// `handle` must be null or a live handle returned by `msdk_usage_control_new`.
/// Its widget must be unparented, and it must not be used again after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn msdk_usage_control_free(handle: *mut MsdkUsageControl) {
    if !handle.is_null() {
        // SAFETY: the caller transfers the unique handle back exactly once.
        drop(unsafe { Box::from_raw(handle) });
    }
}
