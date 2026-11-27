use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{
    Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
};
use slint::{
    ComponentHandle, LogicalPosition, Model, ModelRc, Rgb8Pixel, SharedPixelBuffer, VecModel,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

thread_local! {
    static NOW: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

slint::slint! {
    import { PerformanceAppOption, PerformanceAppPicker } from "../ui/performance-app-picker.slint";

    export component PickerTestWindow inherits Window {
        width: 600px;
        height: 420px;
        in property <string> selected_package: "com.ikko.ab06.settings";
        in property <string> selected_label: "SettingsPie · com.ikko.ab06.settings";
        in property <int> selected_index: 1;
        in property <[PerformanceAppOption]> options;
        out property <bool> dropdown_open: picker.dropdown_open;
        pure callback filter([PerformanceAppOption], string) -> [PerformanceAppOption];
        callback select_package(string);

        if picker.dropdown_open: TouchArea {
            width: parent.width;
            height: parent.height;
            clicked => { picker.close-dropdown(); }
        }

        picker := PerformanceAppPicker {
            x: 0px;
            y: 0px;
            width: parent.width;
            selected_package: root.selected_package;
            selected_label: root.selected_label;
            selected_index: root.selected_index;
            options: root.options;
            filter(options, query) => { root.filter(options, query) }
            select_package(pkg) => { root.select_package(pkg); }
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
    let size = window.size();
    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(size.width, size.height);
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(pixels.make_mut_slice(), size.width as usize);
    });
}

fn click(ui: &PickerTestWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
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

fn key(ui: &PickerTestWindow, text: slint::SharedString) {
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn sample_options() -> Vec<PerformanceAppOption> {
    vec![
        PerformanceAppOption {
            package: "".into(),
            title: "仅查看设备整体".into(),
            subtitle: "".into(),
            is_custom: false,
        },
        PerformanceAppOption {
            package: "com.ikko.ab06.settings".into(),
            title: "SettingsPie".into(),
            subtitle: "com.ikko.ab06.settings".into(),
            is_custom: false,
        },
        PerformanceAppOption {
            package: "com.tencent.mm".into(),
            title: "微信".into(),
            subtitle: "com.tencent.mm".into(),
            is_custom: false,
        },
        PerformanceAppOption {
            package: "com.android.systemui".into(),
            title: "com.android.systemui".into(),
            subtitle: "".into(),
            is_custom: false,
        },
    ]
}

fn test_filter(options: &[PerformanceAppOption], query: &str) -> Vec<PerformanceAppOption> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return options.to_vec();
    }
    let needle = trimmed.to_lowercase();
    let mut result: Vec<PerformanceAppOption> = options
        .iter()
        .filter(|opt| {
            opt.title.to_lowercase().contains(&needle)
                || opt.package.to_lowercase().contains(&needle)
        })
        .cloned()
        .collect();
    if !options.iter().any(|opt| opt.package.as_str() == trimmed)
        && trimmed.is_ascii()
        && !trimmed.chars().any(char::is_whitespace)
    {
        result.push(PerformanceAppOption {
            package: trimmed.into(),
            title: "监控自定义包名".into(),
            subtitle: trimmed.into(),
            is_custom: true,
        });
    }
    result
}

#[test]
fn picker_opens_full_list_selects_all_filters_and_dismisses() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    let ui = PickerTestWindow::new().unwrap();
    let options = sample_options();
    ui.set_options(ModelRc::from(Rc::new(VecModel::from(options.clone()))));

    let filter_queries = Rc::new(RefCell::new(Vec::new()));
    let queries_ref = filter_queries.clone();
    ui.on_filter(move |opts, query| {
        queries_ref.borrow_mut().push(query.to_string());
        let list: Vec<PerformanceAppOption> = opts.iter().collect();
        ModelRc::from(Rc::new(VecModel::from(test_filter(&list, query.as_str()))))
    });

    let selected = Rc::new(RefCell::new(Vec::new()));
    let selected_ref = selected.clone();
    let weak = ui.as_weak();
    let all_options = options.clone();
    ui.on_select_package(move |pkg| {
        let pkg_str = pkg.to_string();
        selected_ref.borrow_mut().push(pkg_str.clone());
        let ui = weak.upgrade().unwrap();
        let idx = all_options
            .iter()
            .position(|opt| opt.package.as_str() == pkg_str)
            .unwrap_or(0);
        let label = match all_options.get(idx) {
            Some(opt) if !pkg_str.is_empty() && !opt.subtitle.is_empty() => {
                format!("{} · {}", opt.title, opt.subtitle)
            }
            _ => pkg_str.clone(),
        };
        ui.set_selected_index(idx as i32);
        ui.set_selected_label(label.into());
        ui.set_selected_package(pkg_str.into());
    });

    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(600, 420));
    render(&window);

    // 1. 点击已选中的输入框：展开完整列表并自动全选，直接输入即可替换而非拼接
    click(&ui, 120., 17.);
    render(&window);
    assert!(ui.get_dropdown_open());
    assert!(
        filter_queries.borrow().is_empty(),
        "首次聚焦展开完整列表时不应拿当前值做过滤"
    );

    key(&ui, "微信".into());
    render(&window);
    assert_eq!(
        filter_queries.borrow().last().unwrap(),
        "微信",
        "全选后直接输入应完整替换原有文本"
    );
    key(&ui, Key::Return.into());
    render(&window);
    assert!(!ui.get_dropdown_open());
    assert_eq!(selected.borrow().last().unwrap(), "com.tencent.mm");

    // 2. 点击输入框展开完整列表后，直接鼠标点击第 4 项（com.android.systemui）
    click(&ui, 120., 17.);
    render(&window);
    assert!(ui.get_dropdown_open());
    // 浮层起点 y = 34 + 4 + 4(padding) = 42px，每项高 32px，第 4 项（index 3）中心约 42 + 3*32 + 16 = 154px
    click(&ui, 120., 154.);
    render(&window);
    assert!(!ui.get_dropdown_open());
    assert_eq!(selected.borrow().last().unwrap(), "com.android.systemui");

    // 3. 输入未安装的自定义包名并按回车确认
    click(&ui, 120., 17.);
    render(&window);
    key(&ui, "com.custom.demo".into());
    render(&window);
    key(&ui, Key::Return.into());
    render(&window);
    assert!(!ui.get_dropdown_open());
    assert_eq!(selected.borrow().last().unwrap(), "com.custom.demo");

    // 4. 筛选态点击 ×（x ≈ 600 - 4 - 26 - 2 - 12 = 556）仅清空搜索词并保持展开
    click(&ui, 120., 17.);
    render(&window);
    key(&ui, "set".into());
    render(&window);
    click(&ui, 556., 17.);
    render(&window);
    assert!(ui.get_dropdown_open());
    assert_eq!(selected.borrow().len(), 3);

    // 5. 点击外部空白区自动收起下拉框
    click(&ui, 300., 360.);
    render(&window);
    assert!(!ui.get_dropdown_open());

    // 6. 非筛选态点击 × 直接切回“仅查看设备整体”
    click(&ui, 556., 17.);
    render(&window);
    assert!(!ui.get_dropdown_open());
    assert_eq!(selected.borrow().last().unwrap(), "");

    // 7. 展开后按 Esc 收起且不触发切换
    click(&ui, 120., 17.);
    render(&window);
    assert!(ui.get_dropdown_open());
    key(&ui, Key::Escape.into());
    render(&window);
    assert!(!ui.get_dropdown_open());
    assert_eq!(selected.borrow().len(), 4);
}
