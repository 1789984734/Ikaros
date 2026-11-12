//! 系统文件拖拽安装 APK。
//!
//! Slint 1.18 的 winit 后端不会把操作系统的文件拖放投递给 `DropArea`，
//! 因此这里直接消费 winit 的原生窗口事件，复用 MVI 的 `InstallApk` 意图完成安装。

use std::path::Path;

use i_slint_backend_winit::winit::event::WindowEvent;
use slint::ComponentHandle;
use tokio::sync::mpsc::UnboundedSender;

use crate::{App, AppIntent, AppWindow};

/// 把窗口级文件拖放事件转换成悬停反馈与安装意图，由 `macos_window::bind` 统一分发。
pub fn handler(ui: &AppWindow, tx: UnboundedSender<AppIntent>) -> impl FnMut(&WindowEvent) {
    let ui = ui.as_weak();
    move |event| match event {
        WindowEvent::HoveredFile(path) => {
            if is_apk(path) {
                set_hover(&ui, true);
            }
        }
        WindowEvent::HoveredFileCancelled => set_hover(&ui, false),
        WindowEvent::DroppedFile(path) => {
            set_hover(&ui, false);
            if is_apk(path) {
                let _ = tx.send(AppIntent::InstallApk(path.clone()));
            }
        }
        _ => {}
    }
}

fn is_apk(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("apk"))
}

fn set_hover(ui: &slint::Weak<AppWindow>, hovering: bool) {
    if let Some(ui) = ui.upgrade() {
        ui.global::<App>().set_apk_drag_hover(hovering);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Platform, PlatformError, WindowAdapter};
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    struct TestPlatform;

    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
        }
    }

    #[test]
    fn only_apk_files_are_installed() {
        assert!(is_apk(Path::new("/tmp/demo.apk")));
        assert!(is_apk(Path::new("/tmp/demo.APK")));
        assert!(!is_apk(Path::new("/tmp/demo.zip")));
        assert!(!is_apk(Path::new("/tmp/apk")));
    }

    #[test]
    fn window_drop_events_drive_hover_and_install() {
        let _ = slint::platform::set_platform(Box::new(TestPlatform));
        let ui = AppWindow::new().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppIntent>();
        let mut on_event = handler(&ui, tx);

        // 悬停 APK: 显示全屏释放提示
        on_event(&WindowEvent::HoveredFile("/tmp/demo.apk".into()));
        assert!(ui.global::<App>().get_apk_drag_hover());

        // 拖出窗口: 撤掉提示
        on_event(&WindowEvent::HoveredFileCancelled);
        assert!(!ui.global::<App>().get_apk_drag_hover());

        // 悬停非 APK: 不提示
        on_event(&WindowEvent::HoveredFile("/tmp/demo.zip".into()));
        assert!(!ui.global::<App>().get_apk_drag_hover());

        // 释放 APK: 派发安装意图并收起提示
        on_event(&WindowEvent::DroppedFile("/tmp/demo.apk".into()));
        assert!(!ui.global::<App>().get_apk_drag_hover());
        match rx.try_recv() {
            Ok(AppIntent::InstallApk(path)) => assert_eq!(path, PathBuf::from("/tmp/demo.apk")),
            other => panic!("期望收到 InstallApk 意图, 实际: {other:?}"),
        }

        // 释放非 APK: 什么也不做
        on_event(&WindowEvent::DroppedFile("/tmp/demo.zip".into()));
        assert!(rx.try_recv().is_err());
    }
}
