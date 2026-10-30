use crate::{App, AppWindow, LogItemData};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

#[derive(Default)]
struct Selection {
    rows: BTreeSet<usize>,
    anchor: Option<usize>,
    drag: Option<(usize, BTreeSet<usize>)>,
}

impl Selection {
    fn select(&mut self, index: usize, shift: bool, additive: bool, context: bool) {
        if context && self.rows.contains(&index) {
            return;
        }
        let anchor = if shift {
            self.anchor.unwrap_or(index)
        } else {
            index
        };
        if !additive || context {
            self.rows.clear();
        }
        let base = self.rows.clone();
        if shift {
            self.rows.extend(anchor.min(index)..=anchor.max(index));
        } else if !self.rows.remove(&index) {
            self.rows.insert(index);
        }
        self.anchor = Some(anchor);
        self.drag = (!context).then_some((anchor, base));
    }

    fn drag_to(&mut self, index: usize) {
        let Some((anchor, base)) = &self.drag else {
            return;
        };
        self.rows = base.clone();
        self.rows
            .extend((*anchor).min(index)..=(*anchor).max(index));
    }

    fn reconcile(&mut self, old: &[LogItemData], new: &[LogItemData]) {
        let mut next_old = 0;
        let mut mapping = vec![None; old.len()];
        for (new_index, row) in new.iter().enumerate() {
            if let Some(offset) = old[next_old..].iter().position(|entry| entry == row) {
                let old_index = next_old + offset;
                mapping[old_index] = Some(new_index);
                next_old = old_index + 1;
            }
        }
        self.rows = self
            .rows
            .iter()
            .filter_map(|&index| mapping[index])
            .collect();
        self.anchor = self.anchor.and_then(|index| mapping[index]);
        if old != new {
            self.drag = self.drag.take().and_then(|(anchor, base)| {
                mapping[anchor].map(|anchor| {
                    (
                        anchor,
                        base.into_iter()
                            .filter_map(|index| mapping[index])
                            .collect(),
                    )
                })
            });
        }
    }

    fn copy_text(&self, entries: &[LogItemData], contents_only: bool) -> String {
        self.rows
            .iter()
            .filter_map(|&index| entries.get(index))
            .map(|entry| {
                if contents_only {
                    entry.message.to_string()
                } else {
                    format!(
                        "{} {} {} {}: {}",
                        entry.timestamp, entry.package_name, entry.level, entry.tag, entry.message
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn publish(ui: &AppWindow, selection: &Selection) {
    let flags = (0..ui.global::<App>().get_log_items().row_count())
        .map(|index| selection.rows.contains(&index))
        .collect::<Vec<_>>();
    ui.global::<App>().set_log_selection(ModelRc::from(Rc::new(VecModel::from(flags))));
    ui.global::<App>().set_log_selection_count(selection.rows.len() as i32);
}

pub fn bind(ui: &AppWindow) {
    let selection = Rc::new(RefCell::new(Selection::default()));
    let entries = Rc::new(VecModel::<LogItemData>::default());
    ui.global::<App>().set_log_items(ModelRc::from(entries.clone()));
    bind_entries(ui, selection.clone(), entries);
    bind_selection(ui, selection.clone());
    bind_actions(ui, selection);
}

fn bind_entries(
    ui: &AppWindow,
    selection: Rc<RefCell<Selection>>,
    entries: Rc<VecModel<LogItemData>>,
) {
    let pending = Rc::new(RefCell::new(None));
    bind_follow(ui, pending.clone());
    let weak = ui.as_weak();
    let mut context = log_context(ui);
    ui.global::<App>().on_update_log_entries(move |new| {
        let Some(ui) = weak.upgrade() else { return };
        let old = entries.iter().collect::<Vec<_>>();
        let new = new.iter().collect::<Vec<_>>();
        let next_context = log_context(&ui);
        let reset = next_context != context || new.is_empty();
        context = next_context;
        if reset {
            pending.borrow_mut().take();
            ui.global::<App>().set_follow_latest_logs(true);
        }
        if !ui.global::<App>().get_follow_latest_logs() && old != new {
            *pending.borrow_mut() = Some(new);
            ui.global::<App>().set_log_updates_pending(true);
            return;
        }
        pending.borrow_mut().take();
        ui.global::<App>().set_log_updates_pending(false);
        if old == new {
            return;
        }
        let mut selection = selection.borrow_mut();
        selection.reconcile(&old, &new);
        update_rows(&entries, &old, new);
        publish(&ui, &selection);
    });
}

fn log_context(ui: &AppWindow) -> (slint::SharedString, slint::SharedString, i32) {
    (
        ui.global::<App>().get_current_serial(),
        ui.global::<App>().get_log_search_query(),
        ui.global::<App>().get_log_level_selected(),
    )
}

fn bind_follow(ui: &AppWindow, pending: Rc<RefCell<Option<Vec<LogItemData>>>>) {
    let weak = ui.as_weak();
    ui.global::<App>().on_log_follow_changed(move |following| {
        if !following {
            return;
        }
        let Some(ui) = weak.upgrade() else { return };
        let latest = pending.borrow_mut().take();
        if let Some(latest) = latest {
            ui.global::<App>().invoke_update_log_entries(ModelRc::from(Rc::new(VecModel::from(latest))));
        }
    });
}

// Preserve surviving rows when the bounded log window drops old entries and appends new ones.
fn update_rows<T: Clone + PartialEq + 'static>(entries: &VecModel<T>, old: &[T], new: Vec<T>) {
    let overlap = (0..old.len()).find(|&start| new.starts_with(&old[start..]));
    let Some(trim) = overlap else {
        entries.set_vec(new);
        return;
    };
    for _ in 0..trim {
        entries.remove(0);
    }
    let retained = old.len() - trim;
    if retained < new.len() {
        entries.extend(new.into_iter().skip(retained));
    }
}

fn bind_selection(ui: &AppWindow, selection: Rc<RefCell<Selection>>) {
    let weak = ui.as_weak();
    let state = selection.clone();
    ui.global::<App>().on_select_log(move |index, shift, additive, context| {
        let Some(ui) = weak.upgrade() else { return };
        if index < 0 || index as usize >= ui.global::<App>().get_log_items().row_count() {
            return;
        }
        let mut state = state.borrow_mut();
        state.select(index as usize, shift, additive, context);
        publish(&ui, &state);
    });
    let weak = ui.as_weak();
    ui.global::<App>().on_drag_select_log(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        if index < 0 || index as usize >= ui.global::<App>().get_log_items().row_count() {
            return;
        }
        let mut state = selection.borrow_mut();
        state.drag_to(index as usize);
        publish(&ui, &state);
    });
}

fn bind_actions(ui: &AppWindow, selection: Rc<RefCell<Selection>>) {
    let weak = ui.as_weak();
    let state = selection.clone();
    ui.global::<App>().on_copy_selected_logs(move |contents_only| {
        let Some(ui) = weak.upgrade() else { return };
        let rows = ui.global::<App>().get_log_items().iter().collect::<Vec<_>>();
        let text = state.borrow().copy_text(&rows, contents_only);
        if !text.is_empty() {
            ui.global::<App>().invoke_copy_clipboard(text.into());
        }
    });
    let weak = ui.as_weak();
    let state = selection.clone();
    ui.global::<App>().on_select_all_logs(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut state = state.borrow_mut();
        state.rows = (0..ui.global::<App>().get_log_items().row_count()).collect();
        state.anchor = state.rows.first().copied();
        state.drag = None;
        publish(&ui, &state);
    });
    let weak = ui.as_weak();
    ui.global::<App>().on_clear_log_selection(move || {
        let Some(ui) = weak.upgrade() else { return };
        *selection.borrow_mut() = Selection::default();
        publish(&ui, &selection.borrow());
    });
}

#[cfg(test)]
#[path = "log_selection_ui_tests.rs"]
mod ui_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(messages: &[&str]) -> Vec<LogItemData> {
        messages
            .iter()
            .map(|message| LogItemData {
                timestamp: "10-29 12:00:00.000".into(),
                package_name: "com.example".into(),
                level: "I".into(),
                tag: "Tag".into(),
                message: (*message).into(),
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn shift_extends_from_anchor_and_drag_can_shrink() {
        let mut state = Selection::default();
        state.select(4, false, false, false);
        state.select(1, true, false, false);
        assert_eq!(state.rows, (1..=4).collect());
        state.drag_to(3);
        assert_eq!(state.rows, (3..=4).collect());
    }

    #[test]
    fn additive_toggle_and_drag_preserve_existing_rows() {
        let mut state = Selection::default();
        state.select(1, false, false, false);
        state.select(4, false, true, false);
        state.drag_to(6);
        assert_eq!(state.rows, [1, 4, 5, 6].into());
        state.select(5, false, true, false);
        assert_eq!(state.rows, [1, 4, 6].into());
    }

    #[test]
    fn right_click_preserves_group_or_selects_unselected_row() {
        let mut state = Selection::default();
        state.select(1, false, false, false);
        state.select(3, true, false, false);
        state.select(2, false, false, true);
        assert_eq!(state.rows, (1..=3).collect());
        state.select(5, false, false, true);
        assert_eq!(state.rows, [5].into());
    }

    #[test]
    fn refresh_retains_selection_after_append_trim_and_filter() {
        let mut state = Selection::default();
        state.select(1, false, false, false);
        state.select(2, false, true, false);
        state.reconcile(&rows(&["a", "b", "c"]), &rows(&["a", "b", "c", "d"]));
        assert_eq!(state.rows, [1, 2].into());
        state.reconcile(&rows(&["a", "b", "c", "d"]), &rows(&["b", "c", "d", "e"]));
        assert_eq!(state.rows, [0, 1].into());
        state.reconcile(&rows(&["b", "c", "d", "e"]), &rows(&["c", "e"]));
        assert_eq!(state.rows, [0].into());
        assert_eq!(state.anchor, Some(0));
        state.reconcile(&rows(&["c", "e"]), &[]);
        assert!(state.rows.is_empty());
        assert_eq!(state.anchor, None);
    }

    #[test]
    fn copy_is_in_display_order_and_preserves_multiline_messages() {
        let entries = rows(&["first", "second\ncontinued", "third"]);
        let mut state = Selection::default();
        state.select(2, false, false, false);
        state.select(1, false, true, false);
        assert_eq!(state.copy_text(&entries, true), "second\ncontinued\nthird");
        assert_eq!(state.copy_text(&entries, false),
            "10-29 12:00:00.000 com.example I Tag: second\ncontinued\n10-29 12:00:00.000 com.example I Tag: third");
    }
}
