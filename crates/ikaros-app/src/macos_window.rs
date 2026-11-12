use i_slint_backend_winit::{winit, EventResult, WinitWindowAccessor};
use objc2_app_kit::NSView;
use objc2_foundation::NSString;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// winit 每个窗口只允许注册一个事件过滤器，所以原生窗口事件在这里集中分发。
/// `on_event` 负责业务侧的拖放等事件，本函数自身只处理窗口位置的恢复与保存。
pub fn bind(
    window: &slint::Window,
    mut on_event: impl FnMut(&winit::event::WindowEvent) + 'static,
) {
    let mut initialized = false;
    // Slint creates the native window only after its event loop starts.
    window.on_winit_window_event(move |window, event| {
        if !initialized {
            initialized = window
                .with_winit_window(restore_and_autosave)
                .unwrap_or(false);
        }
        on_event(event);
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
