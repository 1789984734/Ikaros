use muda::{Menu, PredefinedMenuItem, Submenu};

pub fn init() -> Result<Menu, Box<dyn std::error::Error>> {
    let backend = i_slint_backend_winit::Backend::builder()
        .with_default_menu_bar(false)
        .build()?;
    slint::platform::set_platform(Box::new(backend))?;

    let menu = Menu::new();
    let app_menu = Submenu::new("Ikaros", true);
    app_menu.append_items(&[
        &PredefinedMenuItem::about(None, None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::services(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::hide(None),
        &PredefinedMenuItem::hide_others(None),
        &PredefinedMenuItem::show_all(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::quit(None),
    ])?;

    let window_menu = Submenu::new("窗口", true);
    window_menu.append_items(&[
        &PredefinedMenuItem::minimize(Some("最小化")),
        &PredefinedMenuItem::maximize(Some("缩放")),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::bring_all_to_front(Some("全部置于顶层")),
    ])?;
    menu.append_items(&[&app_menu, &window_menu])?;
    menu.init_for_nsapp();
    // AppKit supplies and validates the system's window arrangement commands.
    window_menu.set_as_windows_menu_for_nsapp();
    Ok(menu)
}
