pub mod aggregate;
pub mod app;
pub mod commands;
pub mod config;
#[cfg(debug_assertions)]
pub mod debug_setup;
pub mod filters;
pub mod http;
pub mod links;
pub mod model;
pub mod notes;
#[cfg(target_os = "macos")]
mod notification_center;
pub mod notifications;
pub mod notify;
pub mod poller;
pub mod pool;
pub mod providers;
pub mod secrets;
pub mod settings;
#[cfg(target_os = "macos")]
mod settings_toolbar;
mod tray;
pub mod updates;
pub mod windows;

use std::sync::Arc;

use tauri::menu::MenuItem;
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::app::{Runtime, SNAPSHOT_EVENT};
use crate::config::{ConfigStore, DEV_FILE_NAME, FILE_NAME};
use crate::notify::{ClickTarget, Notification};
use crate::poller::Snapshot;
use crate::secrets::{SecretStore, Secrets};
use crate::updates::{UpdateCheckResult, UpdateManager, UPDATE_CHECK_RESULT_EVENT};

/// The tray's Pause item, relabeled to Resume while paused.
struct PauseItem(MenuItem<Wry>);

struct Actions {
    app: AppHandle,
}

impl tray::TrayActions for Actions {
    fn refresh(&self) {
        self.app.state::<Arc<Runtime>>().refresh_now();
    }

    fn toggle_pause(&self) {
        let runtime = self.app.state::<Arc<Runtime>>();
        let paused = !runtime.controls.is_paused();
        runtime.set_paused(paused);
    }

    fn open_settings(&self) {
        windows::open_settings(&self.app, windows::SettingsTarget::default());
    }

    /// Shows the popup and sends it the outcome of a check run now.
    fn check_for_updates(&self) {
        let app = self.app.clone();
        tray::show_popup(&app);
        tauri::async_runtime::spawn(async move {
            let updates = app.state::<Arc<UpdateManager>>().inner().clone();
            let result = updates.check_now().await;
            if let Err(e) = &result {
                log::debug!("update check failed: {e}");
            }
            let payload = UpdateCheckResult::from(&result);
            let _ = app.emit_to(tray::POPUP_LABEL, UPDATE_CHECK_RESULT_EVENT, payload);
        });
    }
}

#[cfg(target_os = "macos")]
fn send_notification(app: &AppHandle, notification: &Notification) {
    let handle = app.clone();
    let target = notification.target.clone();
    notification_center::send(notification, move || open_click_target(&handle, &target));
}

#[cfg(not(target_os = "macos"))]
fn send_notification(app: &AppHandle, notification: &Notification) {
    use tauri_plugin_notification::NotificationExt;
    let result = app
        .notification()
        .builder()
        .title(&notification.title)
        .body(&notification.body)
        .show();
    if let Err(e) = result {
        log::warn!("notification failed: {e}");
    }
}

/// Handles a click on a notification: opens the run in the browser, or shows the popup.
#[cfg(target_os = "macos")]
fn open_click_target(app: &AppHandle, target: &ClickTarget) {
    use notification_center::ClickAction;
    use tauri_plugin_opener::OpenerExt;
    let action = {
        let runtime = app.state::<Arc<Runtime>>();
        runtime.with_config(|config| notification_center::click_action(target, config.config()))
    };
    match action {
        ClickAction::ShowPopup => tray::show_popup(app),
        ClickAction::OpenUrl(url) => {
            if let Err(e) = app.opener().open_url(url.as_str(), None::<&str>) {
                log::warn!("run page could not be opened: {e}");
            }
        }
    }
}

fn publish(app: &AppHandle, snapshot: &Snapshot) {
    log::debug!("snapshot: {:?} ({})", snapshot.color, snapshot.tooltip);
    let look = tray::TrayLook::from_snapshot(snapshot.color, snapshot.paused);
    let _ = tray::set_look(app, look, &snapshot.tooltip);
    // Pause can be toggled from the popup too, so the menu label follows the snapshot.
    if let Some(item) = app.try_state::<PauseItem>() {
        tray::set_pause_label(&item.0, snapshot.paused);
    }
    let _ = app.emit(SNAPSHOT_EVENT, snapshot);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(if cfg!(debug_assertions) {
                    log::LevelFilter::Debug
                } else {
                    log::LevelFilter::Info
                })
                // The updater logs an unreachable server as an error; Vigia logs the failure itself.
                .level_for("tauri_plugin_updater", log::LevelFilter::Off)
                .build(),
        )
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_popup(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::refresh_now,
            commands::set_paused,
            commands::open_url,
            commands::hide_popup,
            commands::open_settings,
            commands::get_settings,
            commands::test_connection,
            commands::add_account,
            commands::rename_account,
            commands::replace_token,
            commands::delete_account,
            commands::delete_all_accounts,
            commands::list_picker_repos,
            commands::set_watched,
            commands::set_repo_branches,
            commands::set_repo_ignored_workflows,
            commands::set_repo_include_tags,
            commands::set_org_filters,
            commands::unwatch_repo,
            commands::restore_watched_repo,
            commands::clear_repo_overrides,
            commands::update_settings,
            commands::retry_secrets,
            commands::reset_secrets,
            commands::open_github_token_page,
            commands::select_settings_pane,
            commands::set_settings_toolbar_enabled,
            commands::check_for_updates_now,
            commands::install_update,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            #[cfg(target_os = "macos")]
            notification_center::init(&app.config().identifier);

            app.handle().plugin(tauri_plugin_positioner::init())?;

            let handle = app.handle().clone();
            let (_tray, pause_item) = tray::build(
                &handle,
                Box::new(Actions {
                    app: handle.clone(),
                }),
            )?;
            app.manage(PauseItem(pause_item));

            // Hiding the popup on blur keeps it behaving like a menu bar panel.
            if let Some(popup) = app.get_webview_window(tray::POPUP_LABEL) {
                let window = popup.clone();
                let blur_handle = handle.clone();
                popup.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(false) = event {
                        let was_visible = window.is_visible().unwrap_or(false);
                        if was_visible {
                            tray::note_blur_hide();
                        }
                        let _ = window.hide();
                        tray::set_highlight(&blur_handle, false);
                    }
                });
            }

            let config_file = if cfg!(debug_assertions) {
                DEV_FILE_NAME
            } else {
                FILE_NAME
            };
            let config_path = app.path().app_config_dir()?.join(config_file);
            let config = match ConfigStore::load_or_recover(&config_path) {
                Ok(config) => config,
                Err(e) => {
                    log::error!("config could not be loaded: {e}");
                    ConfigStore::unavailable(&e)
                }
            };
            let store: Box<dyn SecretStore> = if cfg!(debug_assertions) {
                let path = app.path().app_config_dir()?.join("tokens.dev.json");
                log::warn!("debug build: tokens are stored in {}", path.display());
                Box::new(secrets::FileStore::new(path))
            } else {
                let bundle_id = app.config().identifier.clone();
                Box::new(secrets::KeychainStore::new(&secrets::service_name(
                    &bundle_id,
                )))
            };
            let secrets = Arc::new(Secrets::new(store));
            log::info!("secrets: {}", secrets.state());
            if config::should_prune_secrets(&config, secrets.is_blocked()) {
                let keep: Vec<String> = config
                    .config()
                    .accounts
                    .iter()
                    .map(|a| a.id.clone())
                    .collect();
                match secrets.prune(&keep) {
                    Ok(0) => {}
                    Ok(removed) => log::info!("removed {removed} orphaned tokens"),
                    Err(e) => log::warn!("could not prune orphaned tokens: {e}"),
                }
            }

            let publisher_handle = handle.clone();
            let notifier_handle = handle.clone();
            #[cfg(debug_assertions)]
            let popup_handle = handle.clone();
            // The repo list fetched at launch fills the settings window's cache.
            let repo_lists = settings::RepoListCache::default();
            let seeded_lists = repo_lists.clone();
            let runtime = Arc::new(
                Runtime::new(
                    config,
                    secrets,
                    Arc::new(move |snapshot| publish(&publisher_handle, snapshot)),
                    Arc::new(move |notification| send_notification(&notifier_handle, notification)),
                )
                .with_repo_list_sink(Arc::new(move |account, repos| {
                    seeded_lists.seed(&account.id, repos.to_vec())
                })),
            );
            windows::sync_autostart(
                &handle,
                runtime.with_config(|c| c.config().settings.launch_at_login),
            );

            let update_sink = Arc::clone(&runtime);
            let updates = Arc::new(UpdateManager::new(
                Arc::new(updates::PluginSource::new(handle.clone())),
                Arc::new(move |update| update_sink.set_update(update)),
            ));
            let schedule_runtime = Arc::clone(&runtime);
            tauri::async_runtime::spawn(updates::run_schedule(Arc::clone(&updates), move || {
                schedule_runtime.with_config(|c| c.config().settings.check_for_updates)
            }));

            app.manage(runtime.clone());
            app.manage(updates);
            app.manage(repo_lists);
            app.manage(settings::UnwatchedRepos::default());

            tauri::async_runtime::spawn(async move {
                #[cfg(debug_assertions)]
                debug_setup::apply(&runtime).await;
                runtime.start_heartbeat();
                runtime.restart();
                // Debug aid: show the popup at launch so it can be screenshotted.
                #[cfg(debug_assertions)]
                if std::env::var_os("VIGIA_DEBUG_SHOW_POPUP").is_some() {
                    tray::show_popup(&popup_handle);
                }
                #[cfg(debug_assertions)]
                if std::env::var_os("VIGIA_DEBUG_OPEN_SETTINGS").is_some() {
                    windows::open_settings(&popup_handle, windows::SettingsTarget::default());
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match event {
            // With no windows open the app must stay resident in the menu bar.
            tauri::RunEvent::ExitRequested {
                api, code: None, ..
            } => api.prevent_exit(),
            tauri::RunEvent::Exit => {
                app.state::<Arc<Runtime>>().controls.shutdown();
            }
            _ => {}
        });
}
