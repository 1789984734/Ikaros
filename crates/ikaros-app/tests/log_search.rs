use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{
    Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
};
use slint::{
    ComponentHandle, LogicalPosition, Model, ModelRc, Rgb8Pixel, SharedPixelBuffer, SharedString,
    VecModel,
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
    import { LogFilterCompletion, LogSearch } from "../ui/log-search.slint";

    export component LogSearchTestWindow inherits Window {
        width: 600px;
        height: 420px;
        in property <string> query;
        in property <int> query_sync_rev: 0;
        in property <[string]> history;
        out property <bool> dropdown_open: search.dropdown-open;
        pure callback complete(string, int) -> [LogFilterCompletion];
        callback edited(string);
        callback commit(string, bool);
        callback select_history(string);
        callback remove_history(string);
        callback clear_history();

        if search.dropdown-open: TouchArea {
            width: parent.width;
            height: parent.height;
            clicked => { search.close-dropdown(); }
        }

        search := LogSearch {
            x: 0px;
            y: 0px;
            width: parent.width;
            query: root.query;
            query-sync-rev: root.query_sync_rev;
            history: root.history;
            complete(q, c) => { root.complete(q, c) }
            edited(q) => { root.edited(q); }
            commit(q, force) => { root.commit(q, force); }
            select-history(q) => { root.select_history(q); }
            remove-history(q) => { root.remove_history(q); }
            clear-history => { root.clear_history(); }
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

fn click(ui: &LogSearchTestWindow, x: f32, y: f32) {
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

fn key(ui: &LogSearchTestWindow, text: SharedString) {
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn test_completions(query: &str, cursor: usize) -> Vec<LogFilterCompletion> {
    let before = &query[..cursor.min(query.len())];
    let start = before.rfind(char::is_whitespace).map_or(0, |i| i + 1);
    let typed = before[start..].to_ascii_lowercase();
    [
        "package:",
        "tag:",
        "message:",
        "-package:",
        "-tag:",
        "-message:",
    ]
    .into_iter()
    .filter(|prefix| prefix.starts_with(&typed) && typed != *prefix)
    .map(|prefix| {
        let next_query = format!("{}{}{}", &query[..start], prefix, &query[cursor..]);
        LogFilterCompletion {
            prefix: prefix.into(),
            description: "补全项".into(),
            query: next_query.into(),
            cursor: (start + prefix.len()) as i32,
        }
    })
    .collect()
}

#[test]
fn log_search_shows_recent_history_switches_to_completions_and_supports_actions() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    let ui = LogSearchTestWindow::new().unwrap();

    let history_model = Rc::new(VecModel::from(vec![
        SharedString::from("package:com.example.one"),
        SharedString::from("-tag:Choreographer timeout"),
    ]));
    ui.set_history(ModelRc::from(history_model.clone()));

    ui.on_complete(|q, c| {
        ModelRc::from(Rc::new(VecModel::from(test_completions(
            q.as_str(),
            c.max(0) as usize,
        ))))
    });

    let edits = Rc::new(RefCell::new(Vec::<String>::new()));
    let commits = Rc::new(RefCell::new(Vec::<(String, bool)>::new()));
    let selected_history = Rc::new(RefCell::new(Vec::<String>::new()));

    let edits_ref = edits.clone();
    let weak = ui.as_weak();
    ui.on_edited(move |q| {
        edits_ref.borrow_mut().push(q.to_string());
        weak.upgrade().unwrap().set_query(q);
    });

    let commits_ref = commits.clone();
    ui.on_commit(move |q, force| commits_ref.borrow_mut().push((q.to_string(), force)));

    let selected_ref = selected_history.clone();
    let weak_select = ui.as_weak();
    let history_select = history_model.clone();
    ui.on_select_history(move |q| {
        let value = q.to_string();
        selected_ref.borrow_mut().push(value.clone());
        let mut items: Vec<SharedString> = history_select.iter().collect();
        items.retain(|item| item.as_str() != value);
        items.insert(0, value.clone().into());
        history_select.set_vec(items);
        let ui = weak_select.upgrade().unwrap();
        ui.set_query(value.into());
        ui.set_query_sync_rev(ui.get_query_sync_rev() + 1);
    });

    let history_remove = history_model.clone();
    ui.on_remove_history(move |q| {
        let mut items: Vec<SharedString> = history_remove.iter().collect();
        items.retain(|item| item != &q);
        history_remove.set_vec(items);
    });

    let history_clear = history_model.clone();
    ui.on_clear_history(move || history_clear.set_vec(Vec::new()));

    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(600, 420));
    render(&window);

    // 1. 空输入框聚焦时自动显示最近筛选下拉，键盘 ↓ + Enter 选中第二项
    click(&ui, 120., 16.);
    render(&window);
    assert!(ui.get_dropdown_open());
    key(&ui, Key::DownArrow.into());
    render(&window);
    key(&ui, Key::Return.into());
    render(&window);
    assert!(!ui.get_dropdown_open());
    assert_eq!(
        selected_history.borrow().last().unwrap(),
        "-tag:Choreographer timeout"
    );
    assert_eq!(
        history_model.row_data(0).unwrap().as_str(),
        "-tag:Choreographer timeout"
    );

    // 2. 点击输入框右侧 × 清空文本，不主动弹出下拉
    click(&ui, 556., 16.);
    render(&window);
    assert!(!ui.get_dropdown_open());
    assert_eq!(edits.borrow().last().unwrap(), "");

    // 3. 重新聚焦后输入 "p" 自动切回语法补全，Tab 补全为 "package:"，继续输入并回车强制提交
    click(&ui, 300., 360.);
    render(&window);
    click(&ui, 120., 16.);
    render(&window);
    assert!(ui.get_dropdown_open());
    key(&ui, "p".into());
    render(&window);
    assert!(ui.get_dropdown_open());
    key(&ui, Key::Tab.into());
    render(&window);
    assert!(!ui.get_dropdown_open());
    assert_eq!(edits.borrow().last().unwrap(), "package:");
    key(&ui, "com.demo".into());
    render(&window);
    key(&ui, Key::Return.into());
    render(&window);
    assert_eq!(
        commits.borrow().last().unwrap(),
        &("package:com.demo".into(), true)
    );

    // 4. 非空文本下点击右侧 ▾ 按钮展开最近筛选，点击行内 × 仅删除条目不收起也不应用
    click(&ui, 583., 16.);
    render(&window);
    assert!(ui.get_dropdown_open());
    // 第 1 行 y 中心 = 32 + 4 + 4 + 16 = 56px，删除按钮 x ≈ 578px
    click(&ui, 578., 56.);
    render(&window);
    assert!(ui.get_dropdown_open());
    assert_eq!(history_model.row_count(), 1);
    assert_eq!(
        history_model.row_data(0).unwrap().as_str(),
        "package:com.example.one"
    );
    assert_eq!(ui.get_query().as_str(), "package:com.demo");

    // 5. 点击外部空白区收起下拉并触发失焦提交（force = false）
    let commit_count_before = commits.borrow().len();
    click(&ui, 300., 360.);
    render(&window);
    assert!(!ui.get_dropdown_open());
    assert_eq!(commits.borrow().len(), commit_count_before + 1);
    assert_eq!(
        commits.borrow().last().unwrap(),
        &("package:com.demo".into(), false)
    );

    // 6. 展开最近筛选并点击底部右侧「清空」按钮清空全部历史
    click(&ui, 583., 16.);
    render(&window);
    assert!(ui.get_dropdown_open());
    // 1 条记录时底部栏中心 y = 32 + 4 + 4 + 32 + 2 + 1 + 2 + 11 = 88px，右侧清空按钮 x ≈ 575px
    click(&ui, 575., 88.);
    render(&window);
    assert_eq!(history_model.row_count(), 0);
    assert!(ui.get_dropdown_open());

    // 7. 无历史记录时按 Esc 收起，清空输入框后再次聚焦应回退显示语法补全
    key(&ui, Key::Escape.into());
    render(&window);
    assert!(!ui.get_dropdown_open());
    click(&ui, 556., 16.);
    render(&window);
    click(&ui, 300., 360.);
    render(&window);
    click(&ui, 120., 16.);
    render(&window);
    assert!(ui.get_dropdown_open());
    key(&ui, Key::Tab.into());
    render(&window);
    assert_eq!(edits.borrow().last().unwrap(), "package:");

    // 8. 外部通过 query + query_sync_rev 更新筛选词（模拟右键加入过滤）后应同步输入框，点击历史行可直接应用
    history_model.set_vec(vec![
        SharedString::from("tag:Alpha"),
        SharedString::from("tag:Beta"),
    ]);
    ui.set_query("package:com.ext tag:Net".into());
    ui.set_query_sync_rev(ui.get_query_sync_rev() + 1);
    render(&window);
    // 确认输入框文本已同步为外部新值（点击 × 后触发 edited("") 前仍有可清除内容）
    click(&ui, 583., 16.);
    render(&window);
    assert!(ui.get_dropdown_open());
    // 鼠标直接点击第 2 行（y = 32 + 4 + 4 + 32 + 16 = 88px）应用 "tag:Beta"
    click(&ui, 120., 88.);
    render(&window);
    assert!(!ui.get_dropdown_open());
    assert_eq!(selected_history.borrow().last().unwrap(), "tag:Beta");
}
