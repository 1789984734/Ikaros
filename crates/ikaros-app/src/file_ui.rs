use crate::{
    mvi::{AppIntent, AppViewState},
    App, AppWindow, FileItemData,
};
use ikaros_domain::RemoteFile;
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{path::Path, path::PathBuf, rc::Rc};
use tokio::sync::mpsc::UnboundedSender;

pub fn bind(ui: &AppWindow, tx: UnboundedSender<AppIntent>) {
    bind_navigation(ui, tx.clone());
    bind_download(ui, tx.clone());
    bind_upload(ui, tx.clone());
    bind_download_dir(ui);
    bind_sort(ui, tx);
}

fn bind_navigation(ui: &AppWindow, tx: UnboundedSender<AppIntent>) {
    let path_tx = tx.clone();
    ui.global::<App>().on_browse_directory(move |path| {
        let _ = path_tx.send(AppIntent::BrowseDirectory(path.to_string()));
    });
    let completion_tx = tx.clone();
    ui.global::<App>().on_complete_file_path(move |query| {
        let _ = completion_tx.send(AppIntent::CompleteFilePath(query.to_string()));
    });
    let up_tx = tx.clone();
    ui.global::<App>().on_navigate_up(move || {
        let _ = up_tx.send(AppIntent::NavigateUpDirectory);
    });
    let refresh_tx = tx.clone();
    let handle = ui.as_weak();
    ui.global::<App>().on_refresh_files(move || {
        if let Some(ui) = handle.upgrade() {
            let _ = refresh_tx.send(AppIntent::BrowseDirectory(
                ui.global::<App>().get_current_dir_path().to_string(),
            ));
        }
    });
    ui.global::<App>().on_enter_directory(move |name| {
        let _ = tx.send(AppIntent::EnterDirectory(name.to_string()));
    });
}

fn bind_download(ui: &AppWindow, tx: UnboundedSender<AppIntent>) {
    ui.global::<App>().on_pull_file(move |remote_path| {
        let Some(name) = Path::new(remote_path.as_str()).file_name() else {
            return;
        };
        let local_target = default_download_dir().join(name);
        let _ = tx.send(AppIntent::PullRemoteFile {
            remote_path: remote_path.to_string(),
            local_target: local_target.to_string_lossy().into_owned(),
        });
    });
}

/// 下载默认目录：用户配置优先，否则系统 Downloads
fn default_download_dir() -> PathBuf {
    if let Some(dir) = crate::app_config::load().default_download_dir {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    Path::new(&home).join("Downloads")
}

/// 下载目录选择：选完持久化并更新面板上的目录文本
fn bind_download_dir(ui: &AppWindow) {
    ui.global::<App>().set_download_dir_text(
        crate::app_config::load()
            .default_download_dir
            .map(Into::into)
            .unwrap_or_default(),
    );
    let handle = ui.as_weak();
    ui.global::<App>().on_pick_download_dir(move || {
        let handle = handle.clone();
        let _ = slint::spawn_local(async move {
            let Some(dir) = rfd::AsyncFileDialog::new()
                .set_title("选择下载保存目录")
                .pick_folder()
                .await
            else {
                return;
            };
            let Some(ui) = handle.upgrade() else { return };
            let path = dir.path().to_string_lossy().into_owned();
            crate::app_config::update(|config| config.default_download_dir = Some(path.clone()));
            ui.global::<App>().set_download_dir_text(path.into());
        });
    });
}

fn bind_upload(ui: &AppWindow, tx: UnboundedSender<AppIntent>) {
    let handle = ui.as_weak();
    let file_handle = handle.clone();
    let file_tx = tx.clone();
    ui.global::<App>().on_upload_file(move |directory| {
        pick_upload_paths(file_handle.clone(), file_tx.clone(), directory, false);
    });
    ui.global::<App>().on_upload_folder(move |directory| {
        pick_upload_paths(handle.clone(), tx.clone(), directory, true);
    });
}

fn pick_upload_paths(
    handle: slint::Weak<AppWindow>,
    tx: UnboundedSender<AppIntent>,
    directory: slint::SharedString,
    folders: bool,
) {
    let Some(ui) = handle.upgrade() else { return };
    let serial = ui.global::<App>().get_current_serial();
    let _ = slint::spawn_local(async move {
        let kind = if folders {
            "文件夹"
        } else {
            "文件（可多选）"
        };
        let dialog =
            rfd::AsyncFileDialog::new().set_title(format!("选择要上传到 {directory} 的{kind}"));
        let paths = if folders {
            dialog.pick_folders().await
        } else {
            dialog.pick_files().await
        };
        let Some(paths) = paths else { return };
        let Some(ui) = handle.upgrade() else { return };
        if ui.global::<App>().get_current_serial() != serial || paths.is_empty() {
            return;
        }
        let _ = tx.send(AppIntent::PushLocalPaths {
            local_paths: paths
                .into_iter()
                .map(|path| path.path().to_owned())
                .collect(),
            remote_directory: directory.to_string(),
        });
    });
}

fn bind_sort(ui: &AppWindow, tx: UnboundedSender<AppIntent>) {
    // 排序键与方向的切换逻辑在 store 中，UI 只上报点击的列
    ui.global::<App>().on_sort_files(move |key| {
        let _ = tx.send(AppIntent::SetFileSort(key as i32));
    });
}

pub fn apply(ui: &AppWindow, state: &AppViewState) {
    apply_path_suggestions(ui, state);
    ui.global::<App>()
        .set_current_dir_path(state.current_dir.clone().into());
    ui.global::<App>()
        .set_file_status_text(state.file_transfer_message.clone().into());
    ui.global::<App>()
        .set_file_transfer_percent(state.file_transfer_percent);
    ui.global::<App>().set_file_sort_key(state.file_sort_key);
    ui.global::<App>().set_file_sort_asc(state.file_sort_asc);
    let mut files = state.file_list.clone();
    files.sort_by(|a, b| compare_files(a, b, state.file_sort_key, state.file_sort_asc));
    let items: Vec<FileItemData> = files
        .iter()
        .map(|file| FileItemData {
            name: file.name.clone().into(),
            path: file.path.clone().into(),
            is_dir: file.is_dir,
            size_str: format_size(file.size, file.is_dir).into(),
            permissions: file.permissions.clone().into(),
            created_str: format_file_time(file.created_time).into(),
            modified_str: format_file_time(Some(file.modified_time)).into(),
        })
        .collect();
    if !ui
        .global::<App>()
        .get_file_items()
        .iter()
        .eq(items.iter().cloned())
    {
        ui.global::<App>()
            .set_file_items(ModelRc::from(Rc::new(VecModel::from(items))));
    }
}

fn apply_path_suggestions(ui: &AppWindow, state: &AppViewState) {
    ui.global::<App>()
        .set_file_path_completion_query(state.file_path_completion_query.clone().into());
    let suggestions: Vec<slint::SharedString> =
        state.file_path_suggestions.iter().map(Into::into).collect();
    if !ui
        .global::<App>()
        .get_file_path_suggestions()
        .iter()
        .eq(suggestions.iter().cloned())
    {
        ui.global::<App>()
            .set_file_path_suggestions(ModelRc::from(Rc::new(VecModel::from(suggestions))));
    }
}

/// 目录始终排在文件前面，其余按所选字段与方向比较
fn compare_files(a: &RemoteFile, b: &RemoteFile, key: i32, asc: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if a.is_dir != b.is_dir {
        return if a.is_dir {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    let ord = match key {
        1 => a.size.cmp(&b.size),
        2 => a.modified_time.cmp(&b.modified_time),
        3 => match (a.created_time, b.created_time) {
            (Some(a), Some(b)) => a.cmp(&b),
            (None, Some(_)) => return Ordering::Greater,
            (Some(_), None) => return Ordering::Less,
            (None, None) => Ordering::Equal,
        },
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    };
    if asc {
        ord
    } else {
        ord.reverse()
    }
}

/// epoch 秒转本地中文日期时间，保留秒；未知或无效时间显示破折号。
fn format_file_time(secs: Option<u64>) -> String {
    use chrono::TimeZone;
    let Some(secs) = secs
        .filter(|&secs| secs > 0)
        .and_then(|secs| i64::try_from(secs).ok())
    else {
        return "—".into();
    };
    chrono::Local
        .timestamp_opt(secs, 0)
        .single()
        .map(|t| t.format("%Y 年 %-m 月 %-d 日 %H:%M:%S").to_string())
        .unwrap_or_else(|| "—".into())
}

fn format_size(bytes: u64, is_dir: bool) -> String {
    if is_dir {
        return "—".into();
    }
    let units = ["B", "KB", "MB", "GB", "TB", "PB", "EB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < units.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.2} {}", units[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, size: u64, is_dir: bool, mtime: u64) -> RemoteFile {
        RemoteFile {
            name: name.into(),
            path: format!("/sdcard/{name}"),
            size,
            is_dir,
            is_symlink: false,
            permissions: "rw-r--r--".into(),
            created_time: None,
            modified_time: mtime,
        }
    }

    #[test]
    fn directories_stay_first_regardless_of_sort_direction() {
        let file = entry("zzz.txt", 9, false, 100);
        let dir = entry("aaa", 0, true, 50);
        for asc in [true, false] {
            assert_eq!(compare_files(&dir, &file, 1, asc), std::cmp::Ordering::Less);
            assert_eq!(
                compare_files(&file, &dir, 1, asc),
                std::cmp::Ordering::Greater
            );
        }
    }

    #[test]
    fn sort_keys_compare_numeric_fields_with_direction() {
        let small = entry("a.txt", 1, false, 100);
        let large = entry("b.txt", 9000, false, 50);
        assert_eq!(
            compare_files(&small, &large, 1, true),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_files(&small, &large, 1, false),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_files(&large, &small, 2, true),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_files(&large, &small, 2, false),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn name_sort_ignores_case() {
        let upper = entry("B.txt", 1, false, 0);
        let lower = entry("a.txt", 1, false, 0);
        assert_eq!(
            compare_files(&upper, &lower, 0, true),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn file_times_use_the_requested_localized_format_and_preserve_seconds() {
        use chrono::TimeZone;
        let time = chrono::Local
            .with_ymd_and_hms(2026, 11, 5, 17, 11, 8)
            .single()
            .unwrap();
        assert_eq!(
            format_file_time(Some(time.timestamp() as u64)),
            "2026 年 11 月 5 日 17:11:08"
        );
        for unknown in [None, Some(0), Some(u64::MAX)] {
            assert_eq!(format_file_time(unknown), "—");
        }
    }

    #[test]
    fn creation_time_sorting_keeps_unknown_values_last_in_both_directions() {
        let unknown = entry("a.txt", 1, false, 500);
        let mut early = entry("c.txt", 1, false, 500);
        early.created_time = Some(100);
        let mut late = entry("b.txt", 1, false, 1);
        late.created_time = Some(200);
        assert!(compare_files(&early, &late, 3, true).is_lt());
        assert!(compare_files(&early, &late, 3, false).is_gt());
        for asc in [true, false] {
            assert!(compare_files(&unknown, &early, 3, asc).is_gt());
            assert!(compare_files(&late, &unknown, 3, asc).is_lt());
        }
    }
}
