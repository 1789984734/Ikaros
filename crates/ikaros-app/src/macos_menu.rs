use muda::{Menu, PredefinedMenuItem, Submenu};

pub fn init() -> Result<Menu, Box<dyn std::error::Error>> {
    init_backend()?;

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

fn init_backend() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = i_slint_backend_winit::Backend::builder().with_default_menu_bar(false);
    match std::env::var("IKAROS_RENDERER") {
        Ok(renderer) if matches!(renderer.as_str(), "software" | "femtovg") => {
            tracing::info!(renderer, "使用指定的 Slint 渲染器");
            builder = builder.with_renderer_name(renderer);
        }
        Ok(renderer) => {
            return Err(format!(
                "不支持的 IKAROS_RENDERER={renderer:?}，可选值为 software 或 femtovg"
            )
            .into());
        }
        Err(std::env::VarError::NotPresent) => {}
        Err(error) => return Err(error.into()),
    }
    slint::platform::set_platform(Box::new(builder.build()?))?;
    Ok(())
}
