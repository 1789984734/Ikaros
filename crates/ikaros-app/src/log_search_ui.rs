use crate::{log_filter, App, AppWindow, LogFilterCompletion};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::rc::Rc;

pub fn bind(ui: &AppWindow) {
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
}
