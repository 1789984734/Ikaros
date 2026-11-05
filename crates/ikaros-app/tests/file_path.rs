use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{
    Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
};
use slint::{ComponentHandle, LogicalPosition, ModelRc, Rgb8Pixel, SharedPixelBuffer, VecModel};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

thread_local! {
    static NOW: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

slint::slint! {
    import { FilePathBar } from "../ui/file-path.slint";
    import { Button } from "std-widgets.slint";
    export component PathTestWindow inherits Window {
        width: 600px;
        height: 360px;
        in property <string> path: "/sdcard/Down";
        in property <string> completion-query;
        in property <[string]> choices;
        callback edited(string);
        callback navigate(string);
        FilePathBar {
            y: 0px;
            width: parent.width;
            current-path: root.path;
            completion-query: root.completion-query;
            choices: root.choices;
            edited(query) => { root.edited(query); }
            navigate(path) => { root.navigate(path); }
        }
        Button { x: 0px; y: 300px; width: 100px; height: 32px; text: "其他控件"; }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        NOW.with(Cell::get)
    }
}

fn render(window: &MinimalSoftwareWindow) {
    NOW.with(|now| now.set(now.get() + Duration::from_millis(16)));
    slint::platform::update_timers_and_animations();
    let size = window.size();
    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(size.width, size.height);
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(pixels.make_mut_slice(), size.width as usize);
    });
}

fn click(ui: &PathTestWindow, y: f32) {
    let position = LogicalPosition::new(60., y);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    for event in [
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    ] {
        ui.window().dispatch_event(event);
    }
}

fn key(ui: &PathTestWindow, text: slint::SharedString) {
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn suggestions(ui: &PathTestWindow, window: &MinimalSoftwareWindow, query: &str, choices: &[&str]) {
    render(window);
    render(window);
    click(ui, 16.);
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    key(ui, "a".into());
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    key(ui, query.into());
    ui.set_completion_query(query.into());
    ui.set_choices(ModelRc::from(Rc::new(VecModel::from(
        choices
            .iter()
            .map(|choice| (*choice).into())
            .collect::<Vec<_>>(),
    ))));
    render(window);
}

fn check_tab_and_enter(
    ui: &PathTestWindow,
    window: &MinimalSoftwareWindow,
    edited: &RefCell<Vec<String>>,
    navigated: &RefCell<Vec<String>>,
) {
    suggestions(ui, window, "/sdcard/Down", &["/sdcard/Download/"]);
    key(ui, Key::Tab.into());
    assert_eq!(edited.borrow().last().unwrap(), "/sdcard/Download/");
    key(ui, "child".into());
    key(ui, Key::Return.into());
    assert_eq!(navigated.borrow().last().unwrap(), "/sdcard/Download/child");
    suggestions(ui, window, "/sdcard/中", &["/sdcard/中文 资料/"]);
    key(ui, Key::Tab.into());
    assert_eq!(edited.borrow().last().unwrap(), "/sdcard/中文 资料/");
    key(ui, "子目录".into());
    key(ui, Key::Return.into());
    assert_eq!(
        navigated.borrow().last().unwrap(),
        "/sdcard/中文 资料/子目录"
    );
}

fn check_selection_and_dismissal(
    ui: &PathTestWindow,
    window: &MinimalSoftwareWindow,
    navigated: &RefCell<Vec<String>>,
) {
    let choices = ["/sdcard/Documents/", "/sdcard/Download/"];
    suggestions(ui, window, "/sdcard/D", &choices);
    key(ui, Key::DownArrow.into());
    key(ui, Key::DownArrow.into());
    key(ui, Key::Return.into());
    assert_eq!(navigated.borrow().last().unwrap(), "/sdcard/Download/");
    suggestions(ui, window, "/sdcard/D", &choices);
    click(ui, 52.);
    assert_eq!(navigated.borrow().last().unwrap(), "/sdcard/Documents/");
    suggestions(ui, window, "/sdcard/D", &choices);
    key(ui, Key::Escape.into());
    key(ui, Key::Return.into());
    assert_eq!(navigated.borrow().last().unwrap(), "/sdcard/D");
}

#[test]
fn path_bar_supports_keyboard_mouse_and_unicode_completions() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    let ui = PathTestWindow::new().unwrap();
    let edited = Rc::new(RefCell::new(Vec::new()));
    let changes = edited.clone();
    ui.on_edited(move |query| changes.borrow_mut().push(query.to_string()));
    let navigated = Rc::new(RefCell::new(Vec::new()));
    let paths = navigated.clone();
    let weak = ui.as_weak();
    ui.on_navigate(move |path| {
        paths.borrow_mut().push(path.to_string());
        weak.upgrade()
            .unwrap()
            .set_path(path.trim_end_matches('/').into());
    });
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(600, 360));
    render(&window);
    check_tab_and_enter(&ui, &window, &edited, &navigated);
    check_selection_and_dismissal(&ui, &window, &navigated);
}
