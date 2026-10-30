use super::{AppIntent, AppStore};
use ikaros_domain::{AppFilterType, AppInfo};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::error;

impl AppStore {
    pub(super) fn on_load_packages(&mut self, filter: AppFilterType) {
        let serial = self.state.current_serial();
        if serial == "未连接" {
            return;
        }

        self.state.is_busy = true;
        if self.state.app_filter != filter {
            self.state.app_list.clear();
        }
        self.state.app_filter = filter;
        self.state.app_status_message = "正在读取应用包列表...".to_string();

        let usecases = Arc::clone(&self.app_usecases);
        let tx = self.intent_sender.clone();

        tokio::spawn(async move {
            let result = usecases
                .get_installed_packages(&serial, filter)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppIntent::PackagesLoaded {
                serial,
                filter,
                result,
            });
        });
    }

    pub(super) fn on_packages_loaded(
        &mut self,
        serial: String,
        filter: AppFilterType,
        result: Result<Vec<AppInfo>, String>,
    ) {
        if serial != self.state.current_serial() || filter != self.state.app_filter {
            return;
        }
        self.state.is_busy = false;
        match result {
            Ok(apps) => {
                self.state.app_status_message = format!("已加载 {} 个应用包", apps.len());
                self.state.app_list = apps;
            }
            Err(error) => {
                self.state.app_status_message = format!("查询应用失败: {error}");
                self.state.status_message = self.state.app_status_message.clone();
            }
        }
    }

    pub(super) fn on_set_app_search_query(&mut self, query: String) {
        self.state.app_search_query = query;
    }

    pub(super) fn on_launch_app(&mut self, pkg: String) {
        let serial = self.state.current_serial();
        let usecases = Arc::clone(&self.app_usecases);
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            let message = match usecases.launch_app(&serial, &pkg).await {
                Ok(()) => format!("已启动应用: {}", pkg),
                Err(error) => format!("启动应用失败: {}", error),
            };
            let _ = tx.send(AppIntent::SetStatusMessage {
                message,
                is_busy: false,
            });
        });
    }

    pub(super) fn on_stop_app(&mut self, pkg: String) {
        let serial = self.state.current_serial();
        let usecases = Arc::clone(&self.app_usecases);
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            let _ = usecases.stop_app(&serial, &pkg).await;
            let _ = tx.send(AppIntent::SetStatusMessage {
                message: format!("已强行停止应用: {}", pkg),
                is_busy: false,
            });
        });
    }

    pub(super) fn on_clear_app_data(&mut self, pkg: String) {
        let serial = self.state.current_serial();
        let usecases = Arc::clone(&self.app_usecases);
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            match usecases.clear_app_data(&serial, &pkg).await {
                Ok(_) => {
                    let _ = tx.send(AppIntent::SetStatusMessage {
                        message: format!("已成功清除数据: {}", pkg),
                        is_busy: false,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppIntent::SetStatusMessage {
                        message: format!("清除数据失败: {}", e),
                        is_busy: false,
                    });
                }
            }
        });
    }

    pub(super) fn on_uninstall_app(&mut self, pkg: String) {
        if self.state.is_busy {
            return;
        }
        let serial = self.state.current_serial();
        if serial == "未连接" {
            self.state.status_message = "请先连接或选择一台设备再卸载应用".into();
            return;
        }
        let Some(app) = self
            .state
            .app_list
            .iter()
            .find(|app| app.package_name == pkg)
            .cloned()
        else {
            self.state.status_message = "应用列表已变化，请刷新后重试".into();
            return;
        };
        self.state.is_busy = true;
        self.state.app_status_message = format!("正在卸载应用: {pkg}...");
        self.state.status_message = self.state.app_status_message.clone();
        let usecases = Arc::clone(&self.app_usecases);
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            let result = usecases
                .uninstall_app(&serial, &app)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppIntent::AppUninstalled {
                serial,
                package_name: pkg,
                result,
            });
        });
    }

    pub(super) fn on_app_uninstalled(
        &mut self,
        serial: String,
        package_name: String,
        result: Result<(), String>,
    ) {
        if serial != self.state.current_serial() {
            return;
        }
        self.state.is_busy = false;
        let message = match result {
            Ok(()) => {
                self.state
                    .app_list
                    .retain(|app| app.package_name != package_name);
                let _ = self
                    .intent_sender
                    .send(AppIntent::LoadPackages(self.state.app_filter));
                format!("已成功卸载应用: {package_name}")
            }
            Err(error) => format!("卸载应用失败: {error}"),
        };
        self.state.app_status_message = message.clone();
        self.state.status_message = message;
    }

    pub(super) fn on_extract_apk(&mut self, remote_apk_path: String, package_name: String) {
        let serial = self.state.current_serial();
        let file_usecases = Arc::clone(&self.file_usecases);
        let tx = self.intent_sender.clone();
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let local_target = PathBuf::from(format!("{}/Downloads/{}.apk", home, package_name));

        tokio::spawn(async move {
            match file_usecases
                .pull_file(&serial, &remote_apk_path, &local_target, None)
                .await
            {
                Ok(_) => {
                    let _ = tx.send(AppIntent::SetStatusMessage {
                        message: format!("APK 已成功提取至: {:?}", local_target),
                        is_busy: false,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppIntent::SetStatusMessage {
                        message: format!("提取 APK 失败: {}", e),
                        is_busy: false,
                    });
                }
            }
        });
    }

    pub(super) fn on_install_apk(&mut self, apk_path: PathBuf) {
        let serial = self.state.current_serial();
        if serial == "未连接" {
            let _ = self.intent_sender.send(AppIntent::SetStatusMessage {
                message: "请先连接或选择一台设备再安装 APK".to_string(),
                is_busy: false,
            });
            return;
        }

        let file_name = apk_path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "app.apk".to_string());

        self.state.is_busy = true;
        let loading_msg = format!("正在安装 APK: {} ...", file_name);
        self.state.status_message = loading_msg.clone();
        self.state.app_status_message = loading_msg;

        let usecases = Arc::clone(&self.app_usecases);
        let tx = self.intent_sender.clone();
        let filter = self.state.app_filter;

        tokio::spawn(async move {
            match usecases.install_apk(&serial, &apk_path).await {
                Ok(_) => {
                    let _ = tx.send(AppIntent::SetStatusMessage {
                        message: format!("🎉 APK 安装成功: {}", file_name),
                        is_busy: false,
                    });
                    // 自动刷新应用列表
                    let _ = tx.send(AppIntent::LoadPackages(filter));
                }
                Err(e) => {
                    error!("APK 安装失败: {}", e);
                    let _ = tx.send(AppIntent::SetStatusMessage {
                        message: format!("❌ APK 安装失败: {}", e),
                        is_busy: false,
                    });
                }
            }
        });
    }
}
