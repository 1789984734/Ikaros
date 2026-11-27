use std::cell::RefCell;
use std::rc::Rc;

use i_slint_backend_winit::{winit, EventResult, WinitWindowAccessor};
use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_core_graphics::{kCGColorSpaceSRGB, CGColorSpace};
use objc2_foundation::NSString;
use objc2_quartz_core::CAMetalLayer;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::RenderingState;

/// winit 每个窗口只允许注册一个事件过滤器，所以原生窗口事件在这里集中分发。
/// `on_event` 负责业务侧的拖放等事件，本函数自身负责窗口位置恢复保存与 Metal 图层配置。
pub fn bind(
    window: &slint::Window,
    mut on_event: impl FnMut(&winit::event::WindowEvent) + 'static,
) {
    let mut initialized = false;
    let metal_layer: Rc<RefCell<Option<Retained<CAMetalLayer>>>> = Rc::new(RefCell::new(None));
    let metal_layer_for_notifier = metal_layer.clone();

    // software 渲染器不支持 rendering notifier，忽略即可
    let _ = window.set_rendering_notifier(move |state, _graphics_api| {
        if matches!(
            state,
            RenderingState::RenderingSetup | RenderingState::AfterRendering
        ) {
            if let Some(layer) = metal_layer_for_notifier.borrow().as_ref() {
                apply_metal_layer_config(layer);
            }
        }
    });

    // Slint creates the native window only after its event loop starts.
    window.on_winit_window_event(move |window, event| {
        if !initialized {
            if let Some(found_layer) = window.with_winit_window(init_window).flatten() {
                *metal_layer.borrow_mut() = found_layer;
                initialized = true;
            }
        }
        on_event(event);
        EventResult::Propagate
    });
}

fn init_window(window: &winit::window::Window) -> Option<Option<Retained<CAMetalLayer>>> {
    let Ok(handle) = window.window_handle() else {
        return None;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // The handle borrows a live NSView; this callback runs on the UI thread.
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    let ns_window = view.window()?;

    let name = NSString::from_str("Ikaros Main Window");
    ns_window.setFrameUsingName(&name);
    if !ns_window.setFrameAutosaveName(&name) {
        tracing::warn!("无法启用 macOS 窗口位置和大小自动保存");
    }

    let metal_layer = find_metal_layer(view);
    if let Some(layer) = &metal_layer {
        apply_metal_layer_config(layer);
        tracing::info!("已为 macOS CAMetalLayer 绑定 sRGB 色彩空间并启用事务同步提交");
    }
    Some(metal_layer)
}

/// 在 NSView 的根图层或其子图层中查找 wgpu 使用的 `CAMetalLayer`
fn find_metal_layer(view: &NSView) -> Option<Retained<CAMetalLayer>> {
    let root_layer = view.layer()?;
    match root_layer.downcast::<CAMetalLayer>() {
        Ok(metal_layer) => Some(metal_layer),
        Err(root_layer) => {
            let sublayers = unsafe { root_layer.sublayers() }?;
            sublayers
                .to_vec()
                .into_iter()
                .find_map(|layer| layer.downcast::<CAMetalLayer>().ok())
        }
    }
}

/// 显式绑定 sRGB 色彩空间并开启事务同步提交，避免 WindowServer 合成路径切换闪烁
fn apply_metal_layer_config(layer: &CAMetalLayer) -> bool {
    let mut changed = false;
    if !layer.presentsWithTransaction() {
        layer.setPresentsWithTransaction(true);
        changed = true;
    }
    if layer.colorspace().is_none() {
        let srgb = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }));
        if srgb.is_some() {
            layer.setColorspace(srgb.as_deref());
            changed = true;
        }
    }
    changed
}
