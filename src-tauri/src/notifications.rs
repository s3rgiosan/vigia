//! Asks for notification permission when the user first does something that needs it.

use tauri::plugin::PermissionState;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::config::Settings;

/// Whether `after` turns on a notification kind that `before` had off.
pub fn enables_notifications(before: &Settings, after: &Settings) -> bool {
    (!before.notify_failures && after.notify_failures)
        || (!before.notify_recoveries && after.notify_recoveries)
}

/// Only an undecided permission is requested; a granted or denied one stays as the user left it.
pub fn should_request(state: PermissionState) -> bool {
    state == PermissionState::Prompt
}

/// Requests notification permission when it has not been decided yet. The request runs on its
/// own thread because it can wait for the user's answer.
pub fn ensure_permission(app: &AppHandle) {
    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("notification-permission".into())
        .spawn(move || {
            let notification = handle.notification();
            let Ok(state) = notification.permission_state() else {
                return;
            };
            if should_request(state) {
                let _ = notification.request_permission();
            }
        });
    if let Err(e) = spawned {
        log::warn!("notification permission request could not start: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(failures: bool, recoveries: bool) -> Settings {
        Settings {
            notify_failures: failures,
            notify_recoveries: recoveries,
            ..Settings::default()
        }
    }

    #[test]
    fn requests_only_while_undecided() {
        assert!(should_request(PermissionState::Prompt));
        assert!(!should_request(PermissionState::Granted));
        assert!(!should_request(PermissionState::Denied));
    }

    #[test]
    fn turning_a_kind_on_enables_notifications() {
        assert!(enables_notifications(
            &settings(false, false),
            &settings(true, false)
        ));
        assert!(enables_notifications(
            &settings(true, false),
            &settings(true, true)
        ));
    }

    #[test]
    fn other_changes_do_not_enable_notifications() {
        assert!(!enables_notifications(
            &settings(true, true),
            &settings(true, true)
        ));
        assert!(!enables_notifications(
            &settings(true, true),
            &settings(false, true)
        ));
        assert!(!enables_notifications(
            &settings(false, false),
            &settings(false, false)
        ));
    }
}
