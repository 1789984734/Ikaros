// JSON 提取弹窗 UI 事件绑定模块

use crate::log_json::{self, JsonFieldItem};
use crate::{App, AppWindow, JsonFieldItemData};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct JsonExtractorUiState {
    all_fields: Vec<JsonFieldItem>,
    pretty_json: String,
}

pub fn bind(ui: &AppWindow) {
    let state = Rc::new(RefCell::new(JsonExtractorUiState::default()));

    // 打开提取弹窗
    let weak = ui.as_weak();
    let state_clone = state.clone();
    ui.global::<App>().on_open_json_extractor(move |message| {
        let Some(ui) = weak.upgrade() else { return };
        match log_json::parse_log_json(message.as_str()) {
            Some((pretty, items)) => {
                let total = items.len() as i32;
                let data_items: Vec<JsonFieldItemData> = items
                    .iter()
                    .map(|item| JsonFieldItemData {
                        path: item.path.clone().into(),
                        value: item.value.clone().into(),
                        formatted_value: item.formatted_value.clone().into(),
                        value_type: item.value_type.clone().into(),
                    })
                    .collect();

                state_clone.borrow_mut().all_fields = items;
                state_clone.borrow_mut().pretty_json = pretty.clone();

                ui.global::<App>().set_json_extractor_search("".into());
                ui.global::<App>().set_json_extractor_total_count(total);
                ui.global::<App>()
                    .set_json_extractor_raw_pretty(pretty.into());
                ui.global::<App>()
                    .set_json_extractor_fields(ModelRc::from(Rc::new(VecModel::from(data_items))));
                ui.global::<App>().set_json_extractor_visible(true);
                ui.global::<App>()
                    .set_status_message(format!("已提取 {} 个 JSON 字段", total).into());
            }
            None => {
                ui.global::<App>()
                    .set_status_message("所选日志未包含有效的 JSON 内容".into());
            }
        }
    });

    // 搜索过滤
    let weak = ui.as_weak();
    let state_clone = state.clone();
    ui.global::<App>().on_search_json_fields(move |query| {
        let Some(ui) = weak.upgrade() else { return };
        let filtered = {
            let state = state_clone.borrow();
            log_json::filter_fields(&state.all_fields, query.as_str())
                .into_iter()
                .map(|item| JsonFieldItemData {
                    path: item.path.clone().into(),
                    value: item.value.clone().into(),
                    formatted_value: item.formatted_value.clone().into(),
                    value_type: item.value_type.clone().into(),
                })
                .collect::<Vec<_>>()
        };
        ui.global::<App>()
            .set_json_extractor_fields(ModelRc::from(Rc::new(VecModel::from(filtered))));
    });

    // 关闭弹窗
    let weak = ui.as_weak();
    ui.global::<App>().on_close_json_extractor(move || {
        let Some(ui) = weak.upgrade() else { return };
        ui.global::<App>().set_json_extractor_visible(false);
    });

    // 复制格式化 JSON
    let weak = ui.as_weak();
    ui.global::<App>().on_copy_pretty_json(move |message| {
        let Some(ui) = weak.upgrade() else { return };
        match log_json::parse_log_json(message.as_str()) {
            Some((pretty, _)) => {
                ui.global::<App>().invoke_copy_clipboard(pretty.into());
                ui.global::<App>()
                    .set_status_message("已复制格式化 JSON 到剪贴板".into());
            }
            None => {
                ui.global::<App>()
                    .set_status_message("所选日志未包含有效的 JSON 内容".into());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Platform, PlatformError, WindowAdapter};
    use slint::Model;

    struct TestPlatform;

    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
        }
    }

    #[test]
    fn test_log_json_ui_flow() {
        let _ = slint::platform::set_platform(Box::new(TestPlatform));
        let ui = AppWindow::new().unwrap();
        bind(&ui);

        let copied = Rc::new(RefCell::new(Vec::<String>::new()));
        let copied_clone = copied.clone();
        ui.global::<App>().on_copy_clipboard(move |text| {
            copied_clone.borrow_mut().push(text.to_string());
        });

        let user_log = r#"[DeviceAuthHttp] RESPONSE_BODY id=11 json={"requestId":"KY4jGsOj","code":200,"msg":"success","developerMsg":"success","timestamp":1794214163723,"data":{"qrCode":"XXX","expiresAt":"XXX"}}"#;

        // 1. 触发打开提取器
        ui.global::<App>()
            .invoke_open_json_extractor(user_log.into());

        assert!(ui.global::<App>().get_json_extractor_visible());
        assert!(ui.global::<App>().get_json_extractor_total_count() > 0);
        let fields = ui.global::<App>().get_json_extractor_fields();
        let qrcode_item = fields
            .iter()
            .find(|i| i.path == "data.qrCode")
            .expect("应包含 data.qrCode");
        assert_eq!(qrcode_item.value, "XXX");

        // 2. 搜索过滤（支持路径匹配）
        ui.global::<App>()
            .invoke_search_json_fields("data.expiresat".into());
        let filtered = ui.global::<App>().get_json_extractor_fields();
        assert_eq!(filtered.row_count(), 1);
        assert_eq!(filtered.iter().next().unwrap().path, "data.expiresAt");

        // 3. 搜索清空
        ui.global::<App>().invoke_search_json_fields("".into());
        let all = ui.global::<App>().get_json_extractor_fields();
        assert_eq!(
            all.row_count(),
            ui.global::<App>().get_json_extractor_total_count() as usize
        );

        // 4. 关闭提取器
        ui.global::<App>().invoke_close_json_extractor();
        assert!(!ui.global::<App>().get_json_extractor_visible());

        // 5. 复制美化 JSON
        ui.global::<App>().invoke_copy_pretty_json(user_log.into());
        let last_copied = copied.borrow().last().cloned().unwrap();
        assert!(last_copied.contains("\"requestId\": \"KY4jGsOj\""));
        assert!(last_copied.contains("\"qrCode\": \"XXX\""));
    }
}
