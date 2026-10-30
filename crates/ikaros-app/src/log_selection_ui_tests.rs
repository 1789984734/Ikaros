use super::*;
use crate::App;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel};
use slint::platform::{
    Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
};
use std::{cell::Cell, time::Duration};

thread_local! {
    static NOW: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

slint::slint! {
    import { LogcatView } from "../ui/logcat.slint";
    export component LogSelectionHarness inherits Window {
        width: 900px;
        height: 300px;
        in property <[bool]> flags;
        in property <int> count;
        in property <bool> wrapping: false;
        in property <length> inset-x: 0px;
        in property <length> inset-y: 0px;
        callback select(int, bool, bool, bool);
        callback drag(int);
        callback copy();
        callback all();
        callback clear();
        LogcatView {
            x: root.inset-x;
            y: root.inset-y;
            width: parent.width - self.x;
            height: parent.height - self.y;
            entries: [
                { timestamp: "now", package_name: "app", level: "I", tag: "Tag", message: "one", level_color: green },
                { timestamp: "now", package_name: "app", level: "I", tag: "Tag", message: "two\nline\nline\nline", level_color: green },
                { timestamp: "now", package_name: "app", level: "I", tag: "Tag", message: "three", level_color: green },
                { timestamp: "now", package_name: "app", level: "I", tag: "Tag", message: "four", level_color: green },
                { timestamp: "now", package_name: "app", level: "I", tag: "Tag", message: "five", level_color: green },
                { timestamp: "now", package_name: "app", level: "I", tag: "Tag", message: "six", level_color: green },
            ];
            selection: root.flags;
            selection-count: root.count;
            wrap-lines: root.wrapping;
            follow-latest: false;
            select(index, shift, additive, context) => { root.select(index, shift, additive, context); }
            drag-select(index) => { root.drag(index); }
            copy-selected(_) => { root.copy(); }
            select-all => { root.all(); }
            clear-selection => { root.clear(); }
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

fn render(window: &MinimalSoftwareWindow) {
    NOW.with(|now| now.set(now.get() + Duration::from_millis(16)));
    slint::platform::update_timers_and_animations();
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        let size = window.size();
        renderer.render(
            &mut vec![Rgb565Pixel::default(); (size.width * size.height) as usize],
            size.width as usize,
        );
    });
}

fn pointer(ui: &LogSelectionHarness, y: f32, down: bool) {
    let position = slint::LogicalPosition::new(500., y);
    let button = PointerEventButton::Left;
    ui.window().dispatch_event(if down {
        WindowEvent::PointerPressed { position, button }
    } else {
        WindowEvent::PointerReleased { position, button }
    });
}

fn click(ui: &LogSelectionHarness, y: f32) {
    pointer(ui, y, true);
    pointer(ui, y, false);
}

fn key(ui: &LogSelectionHarness, text: slint::SharedString, down: bool) {
    ui.window().dispatch_event(if down {
        WindowEvent::KeyPressed { text }
    } else {
        WindowEvent::KeyReleased { text }
    });
}

fn publish_harness(ui: &LogSelectionHarness, state: &Selection) {
    ui.set_flags(ModelRc::from(Rc::new(VecModel::from(
        (0..6)
            .map(|index| state.rows.contains(&index))
            .collect::<Vec<_>>(),
    ))));
    ui.set_count(state.rows.len() as i32);
}

fn bind_harness(ui: &LogSelectionHarness) -> (Rc<RefCell<Selection>>, Rc<RefCell<i32>>) {
    let state = Rc::new(RefCell::new(Selection::default()));
    let weak = ui.as_weak();
    let selected = state.clone();
    ui.on_select(move |index, shift, additive, context| {
        let ui = weak.upgrade().unwrap();
        selected
            .borrow_mut()
            .select(index as usize, shift, additive, context);
        publish_harness(&ui, &selected.borrow());
    });
    let weak = ui.as_weak();
    let selected = state.clone();
    ui.on_drag(move |index| {
        let ui = weak.upgrade().unwrap();
        selected.borrow_mut().drag_to(index as usize);
        publish_harness(&ui, &selected.borrow());
    });
    let copied = Rc::new(RefCell::new(0));
    let count = copied.clone();
    ui.on_copy(move || *count.borrow_mut() += 1);
    let weak = ui.as_weak();
    let selected = state.clone();
    ui.on_all(move || {
        let ui = weak.upgrade().unwrap();
        selected.borrow_mut().rows = (0..6).collect();
        publish_harness(&ui, &selected.borrow());
    });
    let weak = ui.as_weak();
    let selected = state.clone();
    ui.on_clear(move || {
        let ui = weak.upgrade().unwrap();
        *selected.borrow_mut() = Selection::default();
        publish_harness(&ui, &selected.borrow());
    });
    publish_harness(&ui, &state.borrow());
    (state, copied)
}

fn check_clicks_and_shortcuts(
    ui: &LogSelectionHarness,
    state: &RefCell<Selection>,
    copied: &RefCell<i32>,
) {
    click(&ui, 10.);
    assert_eq!(state.borrow().rows, [0].into());
    key(&ui, Key::Shift.into(), true);
    click(&ui, 140.);
    key(&ui, Key::Shift.into(), false);
    assert!(
        state.borrow().rows.len() >= 3,
        "Shift-click must extend selection"
    );

    // Both modifiers are accepted; Slint maps Command to Control on macOS.
    for modifier in [Key::Control, Key::Meta] {
        let was_selected = state.borrow().rows.contains(&0);
        key(&ui, modifier.into(), true);
        click(&ui, 10.);
        key(&ui, modifier.into(), false);
        assert_ne!(state.borrow().rows.contains(&0), was_selected);
    }
    assert!(state.borrow().rows.contains(&0));
    key(&ui, Key::Control.into(), true);
    key(&ui, "c".into(), true);
    key(&ui, "c".into(), false);
    key(&ui, Key::Control.into(), false);
    assert_eq!(*copied.borrow(), 1);
    key(&ui, Key::Control.into(), true);
    key(&ui, "a".into(), true);
    key(&ui, "a".into(), false);
    key(&ui, Key::Control.into(), false);
    assert_eq!(state.borrow().rows.len(), 6);
    key(&ui, Key::Escape.into(), true);
    key(&ui, Key::Escape.into(), false);
    assert!(state.borrow().rows.is_empty());
}

fn check_wrapped_drag(
    ui: &LogSelectionHarness,
    window: &MinimalSoftwareWindow,
    state: &RefCell<Selection>,
) {
    ui.set_wrapping(true);
    render(&window);
    pointer(&ui, 10., true);
    ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(500., 50.),
    });
    render(&window);
    assert_eq!(
        state.borrow().rows,
        [0, 1].into(),
        "drag across a wrapped row"
    );
    ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(500., 140.),
    });
    render(&window);
    assert!(
        state.borrow().rows.len() >= 3,
        "drag must extend past the wrapped row"
    );
    pointer(&ui, 140., false);
    let before = state.borrow().rows.clone();
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position: slint::LogicalPosition::new(500., 10.),
        button: PointerEventButton::Right,
    });
    assert_eq!(
        state.borrow().rows,
        before,
        "right-click must retain the selected group"
    );
}

#[test]
fn logcat_ui_interactions_and_live_scrolling() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    window.set_size(slint::PhysicalSize::new(900, 300));
    let ui = LogSelectionHarness::new().unwrap();
    let (state, copied) = bind_harness(&ui);
    ui.show().unwrap();
    render(&window);
    check_clicks_and_shortcuts(&ui, &state, &copied);
    check_wrapped_drag(&ui, &window, &state);
    check_context_menu_position(&ui, &window, &copied);
    ui.hide().unwrap();
    drop(ui);
    check_live_scrolling(&window);
    check_live_view_buffering(&window);
}

fn check_context_menu_position(
    ui: &LogSelectionHarness,
    window: &MinimalSoftwareWindow,
    copied: &RefCell<i32>,
) {
    key(ui, Key::Escape.into(), true);
    key(ui, Key::Escape.into(), false);
    ui.set_inset_x(70.);
    ui.set_inset_y(40.);
    render(window);
    let before = *copied.borrow();
    let position = slint::LogicalPosition::new(120., 55.);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Right,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Right,
    });
    render(window);
    let position = slint::LogicalPosition::new(135., 70.);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert_eq!(
        *copied.borrow(),
        before + 1,
        "the first menu item must be clickable just below the right-click position"
    );
}

mod scroll_harness {
    slint::slint! {
        import { LogcatView, LogItemData } from "../ui/logcat.slint";
        export { LogItemData }
        export component LogScrollHarness inherits Window {
            width: 900px;
            height: 300px;
            in property <[LogItemData]> entries;
            in-out property <bool> following <=> logs.follow-latest;
            out property <length> offset: logs.scroll-offset;
            out property <int> clicked: -1;
            public function resume() { logs.scroll-to-bottom(); }
            logs := LogcatView {
                width: parent.width;
                height: parent.height;
                entries: root.entries;
                wrap-lines: true;
                select(index, _, _, _) => { root.clicked = index; }
            }
        }
    }
}

fn scroll_rows(start: usize, end: usize) -> Vec<scroll_harness::LogItemData> {
    (start..end)
        .map(|index| scroll_harness::LogItemData {
            timestamp: "now".into(),
            package_name: "app".into(),
            level: "I".into(),
            tag: "Tag".into(),
            message: format!("{index}: {}", "wrapped log message ".repeat(index % 12 + 1)).into(),
            ..Default::default()
        })
        .collect()
}

fn check_live_scrolling(window: &MinimalSoftwareWindow) {
    let ui = scroll_harness::LogScrollHarness::new().unwrap();
    let entries = Rc::new(VecModel::from(scroll_rows(0, 300)));
    ui.set_entries(entries.clone().into());
    ui.show().unwrap();
    for _ in 0..10 {
        render(window);
    }
    assert!(ui.get_following());
    check_follow_stability(&ui, window, &entries);
    let bottom = top_row(&ui);
    ui.invoke_resume();
    for _ in 0..10 {
        render(window);
    }
    ui.window().dispatch_event(WindowEvent::PointerScrolled {
        position: slint::LogicalPosition::new(500., 150.),
        delta_x: 0.,
        delta_y: 160.,
    });
    for _ in 0..20 {
        render(window);
    }
    assert!(
        !ui.get_following(),
        "scrolling upward must suspend follow mode"
    );
    let after = top_row(&ui);
    assert!(
        after < bottom,
        "the viewport must move up: {after} vs {bottom}"
    );
    check_scrollbar_browsing(&ui, window);
    ui.hide().unwrap();
}

fn check_follow_stability(
    ui: &scroll_harness::LogScrollHarness,
    window: &MinimalSoftwareWindow,
    entries: &VecModel<scroll_harness::LogItemData>,
) {
    let mut start = 0;
    for batch in 1..=20 {
        start += if batch % 3 == 0 { 700 } else { 53 };
        let old = entries.iter().collect::<Vec<_>>();
        update_rows(entries, &old, scroll_rows(start, start + 300));
        for _ in 0..6 {
            render(window);
        }
        let settled = ui.get_offset();
        for _ in 0..3 {
            render(window);
            assert!(
                (ui.get_offset() - settled).abs() < 0.5,
                "follow mode must settle instead of oscillating"
            );
        }
        assert!(ui.get_following());
    }
    let position = slint::LogicalPosition::new(500., 290.);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert_eq!(
        ui.get_clicked(),
        299,
        "follow mode must display the final log row"
    );
    ui.invoke_resume();
    for _ in 0..6 {
        render(window);
    }
}

fn check_scrollbar_browsing(ui: &scroll_harness::LogScrollHarness, window: &MinimalSoftwareWindow) {
    ui.invoke_resume();
    for _ in 0..6 {
        render(window);
    }
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position: slint::LogicalPosition::new(894., 270.),
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(894., 110.),
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position: slint::LogicalPosition::new(894., 110.),
        button: PointerEventButton::Left,
    });
    for _ in 0..6 {
        render(window);
    }
    assert!(
        !ui.get_following(),
        "dragging the scrollbar must suspend follow mode"
    );
}

fn app_rows(start: usize) -> ModelRc<crate::LogItemData> {
    let rows = (start..start + 300)
        .map(|index| crate::LogItemData {
            timestamp: "now".into(),
            package_name: "app".into(),
            message: format!("log {index}").into(),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    Rc::new(VecModel::from(rows)).into()
}

fn check_live_view_buffering(window: &MinimalSoftwareWindow) {
    let ui = AppWindow::new().unwrap();
    bind(&ui);
    ui.global::<App>().set_active_tab(4);
    ui.global::<App>().invoke_update_log_entries(app_rows(0));
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(1120, 740));
    render(window);
    ui.global::<App>().set_follow_latest_logs(false);
    ui.global::<App>()
        .invoke_select_log(10, false, false, false);
    let frozen = ui
        .global::<App>()
        .get_log_items()
        .iter()
        .collect::<Vec<_>>();
    for batch in 1..=20 {
        ui.global::<App>()
            .invoke_update_log_entries(app_rows(batch * 200));
    }
    render(window);
    assert_eq!(
        ui.global::<App>()
            .get_log_items()
            .iter()
            .collect::<Vec<_>>(),
        frozen
    );
    assert!(ui.global::<App>().get_log_updates_pending());
    assert_eq!(ui.global::<App>().get_log_selection_count(), 1);
    ui.global::<App>().set_follow_latest_logs(true);
    render(window);
    assert_eq!(ui.global::<App>().get_log_items().row_count(), 300);
    assert_eq!(
        ui.global::<App>()
            .get_log_items()
            .row_data(0)
            .unwrap()
            .message,
        "log 4000"
    );
    assert!(!ui.global::<App>().get_log_updates_pending());
    check_filter_and_clear_while_reading(&ui, window);
    ui.hide().unwrap();
}

fn check_filter_and_clear_while_reading(ui: &AppWindow, window: &MinimalSoftwareWindow) {
    ui.global::<App>().set_follow_latest_logs(false);
    ui.global::<App>().invoke_update_log_entries(app_rows(5000));
    ui.global::<App>().set_log_search_query("tag:NewTag".into());
    ui.global::<App>().invoke_update_log_entries(app_rows(6000));
    render(window);
    assert!(ui.global::<App>().get_follow_latest_logs());
    assert_eq!(
        ui.global::<App>()
            .get_log_items()
            .row_data(0)
            .unwrap()
            .message,
        "log 6000"
    );
    ui.global::<App>().set_follow_latest_logs(false);
    ui.global::<App>().invoke_update_log_entries(app_rows(7000));
    ui.global::<App>()
        .invoke_update_log_entries(ModelRc::default());
    render(window);
    assert_eq!(ui.global::<App>().get_log_items().row_count(), 0);
    assert_eq!(ui.global::<App>().get_log_selection_count(), 0);
    assert!(!ui.global::<App>().get_log_updates_pending());
}

fn top_row(ui: &scroll_harness::LogScrollHarness) -> i32 {
    let position = slint::LogicalPosition::new(500., 5.);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    ui.get_clicked()
}
