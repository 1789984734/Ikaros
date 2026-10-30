use super::*;
use crate::app_config::AppConfig;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use std::cell::RefCell;

struct TestPlatform;

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

fn bound_ui(extra_args: &str, saved: &Rc<RefCell<Vec<String>>>) -> AppWindow {
    let ui = AppWindow::new().unwrap();
    let tags = ["--turn-screen-off", "--record=/tmp/a b.mp4", "--stay-awake"]
        .map(String::from)
        .to_vec();
    let model = restore_tags(&ui, tags, extra_args.into());
    let saved = saved.clone();
    bind_input_sync(&ui, model.clone(), move |args| {
        saved.borrow_mut().push(args);
    });
    bind_toggle(&ui, model);
    ui
}

fn restarted_ui(saved: &Rc<RefCell<Vec<String>>>) -> AppWindow {
    let config = AppConfig {
        scrcpy_extra_args: saved.borrow().last().unwrap().clone(),
        ..AppConfig::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    let restored: AppConfig = serde_json::from_str(&json).unwrap();
    bound_ui(&restored.scrcpy_extra_args, saved)
}

fn selections(ui: &AppWindow) -> Vec<bool> {
    ui.global::<App>()
        .get_scrcpy_custom_tags()
        .iter()
        .map(|tag| tag.selected)
        .collect()
}

#[test]
fn selected_tags_survive_restart_and_follow_parameter_edits() {
    slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
    let saved = Rc::new(RefCell::new(Vec::new()));
    let ui = bound_ui(
        "--turn-screen-off '--record=/tmp/a b.mp4' --max-fps=30",
        &saved,
    );
    assert_eq!(selections(&ui), [true, true, false]);
    assert!(saved.borrow().is_empty());

    ui.global::<App>().invoke_toggle_scrcpy_tag(0);
    assert_eq!(selections(&ui), [false, true, false]);
    let ui = restarted_ui(&saved);
    assert_eq!(selections(&ui), [false, true, false]);
    assert_eq!(
        scrcpy::parse_extra_args(&ui.global::<App>().get_scrcpy_extra_args()),
        ["--record=/tmp/a b.mp4", "--max-fps=30"]
    );

    ui.global::<App>().invoke_toggle_scrcpy_tag(1);
    assert_eq!(selections(&ui), [false, false, false]);
    assert_eq!(
        scrcpy::parse_extra_args(saved.borrow().last().unwrap()),
        ["--max-fps=30"]
    );
    ui.global::<App>().invoke_toggle_scrcpy_tag(2);
    let ui = restarted_ui(&saved);
    assert_eq!(selections(&ui), [false, false, true]);

    check_parameter_edits_and_clearing(&ui, &saved);
}

fn check_parameter_edits_and_clearing(ui: &AppWindow, saved: &Rc<RefCell<Vec<String>>>) {
    ui.global::<App>()
        .set_scrcpy_extra_args("--turn-screen-off".into());
    ui.global::<App>().invoke_scrcpy_extra_args_changed();
    assert_eq!(selections(ui), [true, false, false]);
    let ui = restarted_ui(saved);
    assert_eq!(selections(&ui), [true, false, false]);
    ui.global::<App>().invoke_toggle_scrcpy_tag(0);
    let ui = restarted_ui(saved);
    assert_eq!(selections(&ui), [false, false, false]);
    assert!(ui.global::<App>().get_scrcpy_extra_args().is_empty());

    ui.global::<App>().invoke_toggle_scrcpy_tag(1);
    ui.global::<App>().set_scrcpy_extra_args("".into());
    ui.global::<App>().invoke_scrcpy_extra_args_changed();
    let ui = restarted_ui(saved);
    assert_eq!(selections(&ui), [false, false, false]);
    assert!(ui.global::<App>().get_scrcpy_extra_args().is_empty());
}
