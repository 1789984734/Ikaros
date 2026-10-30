//! 投屏页面自定义参数 Tag 的交互绑定: 输入框内容与 Tag 列表联动并持久化

use crate::{
    scrcpy, scrcpy_parameters, scrcpy_tags, App, AppWindow, ScrcpyParameterInfo, ScrcpyTagData,
};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::collections::HashSet;
use std::rc::Rc;

/// 绑定 Tag 相关回调, 启动时从本地配置恢复 Tag 列表与选中参数
pub fn bind(ui: &AppWindow) {
    bind_parameter_help(ui);
    let model = restore_tags(
        ui,
        scrcpy_tags::load_tags(),
        crate::app_config::load().scrcpy_extra_args,
    );

    bind_input_sync(ui, model.clone(), save_extra_args);
    bind_add(ui, model.clone());
    bind_toggle(ui, model.clone());
    bind_remove(ui, model);
}

fn restore_tags(
    ui: &AppWindow,
    tags: Vec<String>,
    extra_args: String,
) -> Rc<VecModel<ScrcpyTagData>> {
    ui.global::<App>().set_scrcpy_extra_args(extra_args.into());
    let model = Rc::new(VecModel::default());
    for text in tags {
        model.push(ScrcpyTagData {
            text: text.into(),
            selected: false,
        });
    }
    ui.global::<App>()
        .set_scrcpy_custom_tags(ModelRc::from(model.clone()));
    sync_tag_selection(ui, &model);
    model
}

fn bind_parameter_help(ui: &AppWindow) {
    let parameters = scrcpy_parameters::PARAMETERS
        .iter()
        .map(|parameter| ScrcpyParameterInfo {
            example: parameter.example.into(),
            aliases: parameter
                .short
                .map(|short| format!("短参数：-{short}"))
                .unwrap_or_default()
                .into(),
            description: parameter.description.into(),
        })
        .collect::<Vec<_>>();
    ui.global::<App>()
        .set_scrcpy_parameter_help(ModelRc::from(Rc::new(VecModel::from(parameters))));
    ui.global::<App>()
        .set_scrcpy_help_version(scrcpy_parameters::DOCUMENTATION_VERSION.into());
    ui.global::<App>().on_describe_scrcpy_parameter(|argument| {
        scrcpy_parameters::describe_argument(argument.as_str()).into()
    });
    ui.global::<App>().on_open_scrcpy_parameter_doc(|index| {
        if let Some(parameter) = scrcpy_parameters::PARAMETERS.get(index as usize) {
            let url = parameter.documentation_url();
            tokio::spawn(async move { open_documentation(&url).await });
        }
    });
}

async fn open_documentation(url: &str) {
    #[cfg(target_os = "macos")]
    let mut command = tokio::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = tokio::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = tokio::process::Command::new("xdg-open");

    match command.arg(url).status().await {
        Ok(status) if !status.success() => tracing::warn!(%status, "打开 scrcpy 官方文档失败"),
        Err(error) => tracing::warn!(%error, "打开 scrcpy 官方文档失败"),
        _ => {}
    }
}

fn bind_input_sync(
    ui: &AppWindow,
    model: Rc<VecModel<ScrcpyTagData>>,
    save: impl Fn(String) + 'static,
) {
    let handle = ui.as_weak();
    ui.global::<App>().on_scrcpy_extra_args_changed(move || {
        if let Some(ui) = handle.upgrade() {
            sync_tag_selection(&ui, &model);
            save(ui.global::<App>().get_scrcpy_extra_args().to_string());
        }
    });
}

pub(crate) fn save_extra_args(extra_args: String) {
    let mut config = crate::app_config::load();
    if config.scrcpy_extra_args != extra_args {
        config.scrcpy_extra_args = extra_args;
        crate::app_config::save(&config);
    }
}

/// 把输入框当前解析出的参数逐个保存为 Tag (自动去重)
fn bind_add(ui: &AppWindow, model: Rc<VecModel<ScrcpyTagData>>) {
    let handle = ui.as_weak();
    ui.global::<App>().on_add_scrcpy_tag(move || {
        let Some(ui) = handle.upgrade() else { return };
        let mut changed = false;
        for arg in scrcpy::parse_extra_args(&ui.global::<App>().get_scrcpy_extra_args()) {
            let exists = (0..model.row_count()).any(|i| {
                model
                    .row_data(i)
                    .map(|tag| tag.text.as_str() == arg.as_str())
                    .unwrap_or(false)
            });
            if !exists {
                model.push(ScrcpyTagData {
                    text: arg.into(),
                    selected: false,
                });
                changed = true;
            }
        }
        if changed {
            save_tag_model(&model);
            sync_tag_selection(&ui, &model);
        }
    });
}

/// 点击 Tag: 参数已在输入框则移除, 否则追加
fn bind_toggle(ui: &AppWindow, model: Rc<VecModel<ScrcpyTagData>>) {
    let handle = ui.as_weak();
    ui.global::<App>().on_toggle_scrcpy_tag(move |index| {
        let Some(ui) = handle.upgrade() else { return };
        let Some(tag) = model.row_data(index as usize) else {
            return;
        };
        let mut args = scrcpy::parse_extra_args(&ui.global::<App>().get_scrcpy_extra_args());
        if let Some(pos) = args
            .iter()
            .position(|arg| arg.as_str() == tag.text.as_str())
        {
            args.remove(pos);
        } else {
            args.push(tag.text.to_string());
        }
        ui.global::<App>()
            .set_scrcpy_extra_args(rebuild_extra_args(&args).into());
        ui.global::<App>().invoke_scrcpy_extra_args_changed();
    });
}

fn bind_remove(ui: &AppWindow, model: Rc<VecModel<ScrcpyTagData>>) {
    ui.global::<App>().on_remove_scrcpy_tag(move |index| {
        if (index as usize) < model.row_count() {
            model.remove(index as usize);
            save_tag_model(&model);
        }
    });
}

/// 依据输入框当前解析出的参数刷新每个 Tag 的选中态
fn sync_tag_selection(ui: &AppWindow, model: &VecModel<ScrcpyTagData>) {
    let active: HashSet<String> =
        scrcpy::parse_extra_args(&ui.global::<App>().get_scrcpy_extra_args())
            .into_iter()
            .collect();
    for i in 0..model.row_count() {
        if let Some(tag) = model.row_data(i) {
            let selected = active.contains(tag.text.as_str());
            if tag.selected != selected {
                model.set_row_data(
                    i,
                    ScrcpyTagData {
                        text: tag.text,
                        selected,
                    },
                );
            }
        }
    }
}

/// 把当前 Tag 列表写入本地配置文件
fn save_tag_model(model: &VecModel<ScrcpyTagData>) {
    let tags: Vec<String> = (0..model.row_count())
        .filter_map(|i| model.row_data(i))
        .map(|tag| tag.text.to_string())
        .collect();
    scrcpy_tags::save_tags(&tags);
}

/// 重新序列化参数列表为输入框文本 (含空格的参数自动加引号)
fn rebuild_extra_args(args: &[String]) -> String {
    shlex::try_join(args.iter().map(|arg| arg.as_str())).unwrap_or_else(|_| args.join(" "))
}

#[cfg(test)]
#[path = "scrcpy_ui_tests.rs"]
mod tests;
