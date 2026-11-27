use super::*;
use crate::app_config::AppConfig;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use std::cell::RefCell;
use std::path::Path;

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
    let model = restore_tags(&ui, tags);
    select_device(&ui, Some("device"), || AppConfig {
        scrcpy_extra_args: extra_args.into(),
        ..Default::default()
    });
    let saved = saved.clone();
    bind_preferences_persistence(&ui, move |serial, preferences| {
        assert_eq!(serial, "device");
        saved.borrow_mut().push(preferences.extra_args);
    });
    bind_input_sync(&ui, model.clone());
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

fn bound_device_preferences(path: &Path) -> AppWindow {
    let ui = AppWindow::new().unwrap();
    let model = restore_tags(
        &ui,
        vec!["--turn-screen-off".into(), "--keep-active".into()],
    );
    bind_input_sync(&ui, model);
    let path = path.to_owned();
    bind_preferences_persistence(&ui, move |serial, preferences| {
        app_config::update_at(&path, |config| {
            config.scrcpy_devices.insert(serial.to_owned(), preferences);
        });
    });
    ui
}

#[test]
fn preferences_follow_each_device_across_switches_and_app_restart() {
    slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");
    let load = || app_config::load_from(&path);
    app_config::update_at(&path, |config| {
        config.scrcpy_extra_args = "--keep-active".into()
    });
    let ui = bound_device_preferences(&path);
    select_device(&ui, Some("first"), load);
    assert_eq!(selections(&ui), [false, true]);

    let preferred = ScrcpyPreferences {
        max_size_index: 4,
        max_fps_index: 1,
        bit_rate_index: 3,
        codec_index: 2,
        audio: true,
        stay_awake: false,
        extra_args: "--turn-screen-off".into(),
    };
    apply_preferences(&ui, &preferred);
    ui.global::<App>().invoke_scrcpy_preferences_changed();
    ui.global::<App>().set_scrcpy_using_fallback(true);
    assert_eq!(current_preferences(&ui), preferred);
    select_device(&ui, Some("second"), load);
    assert_eq!(selections(&ui), [false, true]);
    assert_eq!(current_preferences(&ui).codec_index, 0);
    ui.global::<App>().set_scrcpy_codec(1);
    ui.global::<App>().invoke_scrcpy_preferences_changed();

    select_device(&ui, Some("first"), load);
    assert_eq!(current_preferences(&ui), preferred);
    assert_eq!(selections(&ui), [true, false]);
    let reopened = bound_device_preferences(&path);
    select_device(&reopened, Some("second"), load);
    assert_eq!(current_preferences(&reopened).codec_index, 1);
    select_device(&reopened, Some("first"), load);
    assert_eq!(current_preferences(&reopened), preferred);
}
