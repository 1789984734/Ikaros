use crate::{log_filter, mvi::AppIntent, App, AppWindow, LogFilterCompletion};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::rc::Rc;
use tokio::sync::mpsc::UnboundedSender;

pub fn bind(ui: &AppWindow, tx: UnboundedSender<AppIntent>) {
    ui.global::<App>().on_complete_log_filter(|query, cursor| {
        let choices = log_filter::completions(query.as_str(), cursor.max(0) as usize)
            .into_iter()
            .map(|choice| LogFilterCompletion {
                prefix: choice.prefix.into(),
                description: choice.description.into(),
                query: choice.query.into(),
                cursor: choice.cursor as i32,
            })
            .collect::<Vec<_>>();
        ModelRc::from(Rc::new(VecModel::from(choices)))
    });

    let commit_tx = tx.clone();
    ui.global::<App>()
        .on_commit_log_filter(move |query, force| {
            let _ = commit_tx.send(AppIntent::CommitLogFilterHistory {
                generation: None,
                query: query.to_string(),
                force,
            });
        });

    let select_tx = tx.clone();
    ui.global::<App>()
        .on_select_log_filter_history(move |query| {
            let _ = select_tx.send(AppIntent::SelectLogFilterHistory(query.to_string()));
        });

    let remove_tx = tx.clone();
    ui.global::<App>()
        .on_remove_log_filter_history(move |query| {
            let _ = remove_tx.send(AppIntent::RemoveLogFilterHistory(query.to_string()));
        });

    ui.global::<App>().on_clear_log_filter_history(move || {
        let _ = tx.send(AppIntent::ClearLogFilterHistory);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvi::AppViewState;
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
    fn bindings_forward_intents_and_apply_projects_log_history() {
        let _ = slint::platform::set_platform(Box::new(TestPlatform));
        let ui = AppWindow::new().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        bind(&ui, tx);

        // 投影日志状态到 LogcatTab 所在的全局 App 状态
        let state = AppViewState {
            active_tab: 4,
            log_search_query: "tag:Network".into(),
            log_search_sync_rev: 3,
            log_filter_history: vec!["tag:Network".into(), "package:com.example".into()],
            ..Default::default()
        };
        crate::apply_logcat_to_ui(&ui, &state);
        assert_eq!(ui.global::<App>().get_log_search_query(), "tag:Network");
        assert_eq!(ui.global::<App>().get_log_search_sync_rev(), 3);
        let projected: Vec<String> = ui
            .global::<App>()
            .get_log_filter_history()
            .iter()
            .map(|item| item.to_string())
            .collect();
        assert_eq!(projected, vec!["tag:Network", "package:com.example"]);

        // 补全与最近筛选相关回调均正确转为对应 Intent
        let completions = ui
            .global::<App>()
            .invoke_complete_log_filter("pa".into(), 2);
        assert_eq!(completions.row_data(0).unwrap().query, "package:");

        ui.global::<App>()
            .invoke_commit_log_filter("tag:Net".into(), true);
        assert!(matches!(
            rx.try_recv().unwrap(),
            AppIntent::CommitLogFilterHistory {
                generation: None,
                query,
                force: true,
            } if query == "tag:Net"
        ));

        ui.global::<App>()
            .invoke_select_log_filter_history("package:com.example".into());
        assert!(matches!(
            rx.try_recv().unwrap(),
            AppIntent::SelectLogFilterHistory(query) if query == "package:com.example"
        ));

        ui.global::<App>()
            .invoke_remove_log_filter_history("tag:Network".into());
        assert!(matches!(
            rx.try_recv().unwrap(),
            AppIntent::RemoveLogFilterHistory(query) if query == "tag:Network"
        ));

        ui.global::<App>().invoke_clear_log_filter_history();
        assert!(matches!(
            rx.try_recv().unwrap(),
            AppIntent::ClearLogFilterHistory
        ));
    }
}
