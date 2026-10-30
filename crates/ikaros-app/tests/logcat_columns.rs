use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Rgb8Pixel, SharedPixelBuffer};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

thread_local! {
    static NOW: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

slint::slint! {
    import { LogcatView } from "../ui/logcat.slint";
    export component ColumnTestWindow inherits Window {
        width: 900px;
        height: 80px;
        background: #11121a;
        in property <bool> show-package: true;
        in property <bool> show-level: true;
        in property <bool> show-tag: true;
        in property <bool> wrapping;
        in property <string> package-filter: "com.example";
        in property <string> package-display: "com.example";
        in property <string> tag-text: "Network";
        in property <string> message-text: "message-body";
        callback add-filter(string, string);
        LogcatView {
            width: parent.width;
            height: parent.height;
            entries: [{
                timestamp: "now", package_name: root.package-display, level: "I",
                package_filter: root.package-filter,
                tag: root.tag-text, message: root.message-text, level_color: #ff5555
            }];
            show-package: root.show-package;
            show-level: root.show-level;
            show-tag: root.show-tag;
            wrap-lines: root.wrapping;
            follow-latest: false;
            add-filter(field, value) => { root.add-filter(field, value); }
        }
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

fn render(window: &MinimalSoftwareWindow) -> SharedPixelBuffer<Rgb8Pixel> {
    NOW.with(|now| now.set(now.get() + Duration::from_millis(16)));
    slint::platform::update_timers_and_animations();
    let size = window.size();
    let mut pixels = SharedPixelBuffer::new(size.width, size.height);
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(pixels.make_mut_slice(), size.width as usize);
    });
    pixels
}

fn marker_x(pixels: &SharedPixelBuffer<Rgb8Pixel>, color: [u8; 3]) -> Option<u32> {
    pixels
        .as_slice()
        .iter()
        .enumerate()
        .filter(|(index, pixel)| {
            *index / (pixels.width() as usize) < 30
                && [pixel.r, pixel.g, pixel.b]
                    .iter()
                    .zip(color)
                    .all(|(&actual, expected)| actual.abs_diff(expected) <= 6)
        })
        .map(|(index, _)| index as u32 % pixels.width())
        .min()
}

fn set_columns(ui: &ColumnTestWindow, mask: u8) {
    ui.set_show_package(mask & 1 != 0);
    ui.set_show_level(mask & 2 != 0);
    ui.set_show_tag(mask & 4 != 0);
}

fn check_column_combinations(ui: &ColumnTestWindow, window: &MinimalSoftwareWindow) {
    set_columns(ui, 7);
    let baseline = marker_x(&render(window), [248, 248, 242]).unwrap();
    for mask in 0..8 {
        set_columns(ui, mask);
        let pixels = render(window);
        for (bit, color) in [
            (1, [189, 147, 249]),
            (2, [255, 85, 85]),
            (4, [139, 233, 253]),
        ] {
            assert_eq!(
                marker_x(&pixels, color).is_some(),
                mask & bit != 0,
                "column {bit}, mask {mask}"
            );
        }
        assert!(marker_x(&pixels, [98, 114, 164]).is_some());
        let message_x = marker_x(&pixels, [248, 248, 242]).unwrap();
        if mask != 7 {
            assert!(
                message_x < baseline,
                "hidden columns must free message space"
            );
        } else {
            assert_eq!(message_x, baseline, "restoring columns must restore layout");
        }
    }
}

#[test]
fn filtered_columns_disappear_and_release_space_in_both_wrap_modes() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    let ui = ColumnTestWindow::new().unwrap();
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(900, 80));
    for wrapping in [false, true] {
        ui.set_wrapping(wrapping);
        check_column_combinations(&ui, &window);
    }
}

fn click(ui: &ColumnTestWindow, x: f32, y: f32, button: PointerEventButton) {
    let position = LogicalPosition::new(x, y);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    ui.window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
}

fn choose_last_menu_action(ui: &ColumnTestWindow, window: &MinimalSoftwareWindow, x: f32) {
    let before = render(window);
    click(ui, x, 12., PointerEventButton::Right);
    let after = render(window);
    let mut left = after.width();
    let mut bottom = 0;
    for (index, (old, new)) in before.as_slice().iter().zip(after.as_slice()).enumerate() {
        let y = index as u32 / after.width();
        if y > 30 && old != new {
            left = left.min(index as u32 % after.width());
            bottom = bottom.max(y);
        }
    }
    assert!(bottom > 30, "the context menu must be visible at x={x}");
    click(
        ui,
        (left + 15) as f32,
        (bottom - 12) as f32,
        PointerEventButton::Left,
    );
}

fn check_context_filters(
    ui: &ColumnTestWindow,
    window: &MinimalSoftwareWindow,
    filters: &RefCell<Vec<(String, String)>>,
    mask: u8,
) {
    set_columns(ui, mask);
    for (bit, color, field, value) in [
        (1, [189, 147, 249], "package", "com.example"),
        (2, [255, 85, 85], "level", "I"),
        (4, [139, 233, 253], "tag", "Network Worker"),
        (0, [248, 248, 242], "message", "message-body"),
    ] {
        if bit != 0 && mask & bit == 0 {
            continue;
        }
        let x = marker_x(&render(window), color).unwrap() as f32 + 2.;
        let previous = filters.borrow().len();
        choose_last_menu_action(ui, window, x);
        assert_eq!(filters.borrow().len(), previous + 1, "{field}, mask {mask}");
        assert_eq!(
            filters.borrow().last().unwrap(),
            &(field.into(), value.into())
        );
    }
}

#[test]
fn context_filters_target_visible_columns_and_ignore_unknown_packages() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    let ui = ColumnTestWindow::new().unwrap();
    ui.set_tag_text("Network Worker".into());
    let filters = Rc::new(RefCell::new(Vec::new()));
    let captured = filters.clone();
    ui.on_add_filter(move |field, value| captured.borrow_mut().push((field.into(), value.into())));
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(900, 400));
    for wrapping in [false, true] {
        ui.set_wrapping(wrapping);
        for mask in [7, 6, 3, 0] {
            check_context_filters(&ui, &window, &filters, mask);
        }
    }
    check_horizontal_context_filters(&ui, &window, &filters);
    ui.set_package_display("com.example".into());
    set_columns(&ui, 7);
    ui.set_package_filter("".into());
    let x = marker_x(&render(&window), [189, 147, 249]).unwrap() as f32 + 2.;
    let previous = filters.borrow().len();
    choose_last_menu_action(&ui, &window, x);
    assert_eq!(filters.borrow().len(), previous);
    choose_last_menu_action(&ui, &window, 12.);
    assert_eq!(filters.borrow().len(), previous);
}

fn check_horizontal_context_filters(
    ui: &ColumnTestWindow,
    window: &MinimalSoftwareWindow,
    filters: &RefCell<Vec<(String, String)>>,
) {
    ui.set_wrapping(false);
    ui.set_package_display("p".repeat(100).into());
    set_columns(ui, 7);
    let before = marker_x(&render(window), [189, 147, 249]).unwrap();
    ui.window().dispatch_event(WindowEvent::PointerScrolled {
        position: LogicalPosition::new(500., 12.),
        delta_x: -240.,
        delta_y: 0.,
    });
    NOW.with(|now| now.set(now.get() + Duration::from_secs(1)));
    let after = marker_x(&render(window), [189, 147, 249]).unwrap();
    assert!(after < before, "the viewport must scroll horizontally");
    check_context_filters(ui, window, filters, 7);
}
