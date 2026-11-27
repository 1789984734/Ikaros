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

/// 解析 macOS 诊断环境变量 `IKAROS_RENDERER` 指定的渲染器名称
fn parse_renderer_name(value: Option<&str>) -> Result<Option<&str>, String> {
    match value {
        None => Ok(None),
        Some(renderer) if matches!(renderer, "femtovg-wgpu" | "software") => Ok(Some(renderer)),
        Some(renderer) => Err(format!(
            "不支持的 IKAROS_RENDERER={renderer:?}，可选值为 femtovg-wgpu 或 software"
        )),
    }
}

fn init_backend() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = i_slint_backend_winit::Backend::builder().with_default_menu_bar(false);
    let env_renderer = match std::env::var("IKAROS_RENDERER") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(renderer) = parse_renderer_name(env_renderer.as_deref())? {
        tracing::info!(renderer, "使用指定的 Slint 渲染器");
        builder = builder.with_renderer_name(renderer);
    }
    slint::platform::set_platform(Box::new(builder.build()?))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_renderer_name;

    #[test]
    fn parse_renderer_name_accepts_unset_and_supported_values() {
        assert_eq!(parse_renderer_name(None), Ok(None));
        assert_eq!(
            parse_renderer_name(Some("femtovg-wgpu")),
            Ok(Some("femtovg-wgpu"))
        );
        assert_eq!(parse_renderer_name(Some("software")), Ok(Some("software")));
    }

    #[test]
    fn parse_renderer_name_rejects_unsupported_values() {
        let legacy = parse_renderer_name(Some("femtovg")).unwrap_err();
        assert!(legacy.contains("femtovg-wgpu"));
        assert!(legacy.contains("software"));

        assert!(parse_renderer_name(Some("")).is_err());
    }
}
