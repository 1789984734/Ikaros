use i_slint_backend_winit::{winit, EventResult, WinitWindowAccessor};
use objc2_app_kit::NSView;
use objc2_foundation::NSString;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub fn bind(window: &slint::Window) {
    let mut initialized = false;
    // Slint creates the native window only after its event loop starts.
    window.on_winit_window_event(move |window, _| {
        if !initialized {
            initialized = window
                .with_winit_window(restore_and_autosave)
                .unwrap_or(false);
        }
        EventResult::Propagate
    });
}

fn restore_and_autosave(window: &winit::window::Window) -> bool {
    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return false;
    };
    // The handle borrows a live NSView; this callback runs on the UI thread.
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    let Some(window) = view.window() else {
        return false;
    };

    let name = NSString::from_str("Ikaros Main Window");
    window.setFrameUsingName(&name);
    if !window.setFrameAutosaveName(&name) {
        tracing::warn!("无法启用 macOS 窗口位置和大小自动保存");
    }
    true
}
