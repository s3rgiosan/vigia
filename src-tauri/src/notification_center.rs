//! Delivers notifications through macOS's notification center and reports clicks.
//!
//! Each notification is sent from its own thread, which waits until the notification is
//! clicked or removed from Notification Center and then calls `on_click` for a click. Waiting
//! threads are capped; past the cap a notification is still shown but a click only brings the
//! app forward.

use std::sync::atomic::{AtomicUsize, Ordering};

use mac_notification_sys::{Notification as MacNotification, NotificationResponse};
use url::Url;

use crate::config::Config;
use crate::links::check_url;
use crate::notify::{ClickTarget, Notification};

/// Most notifications waiting for a click at once.
const MAX_WAITING: usize = 8;

static WAITING: AtomicUsize = AtomicUsize::new(0);

/// Attributes notifications to the app. Development runs have no registered bundle, so they
/// borrow Terminal's, like tauri-plugin-notification does.
pub fn init(bundle_id: &str) {
    // `is_dev` is true for `tauri dev`, which runs the bare binary with no registered bundle;
    // debug bundles are registered and keep their own id.
    let id = attributed_bundle(bundle_id, tauri::is_dev());
    if let Err(e) = mac_notification_sys::set_application(id) {
        log::warn!("notifications could not be attributed to {id}: {e}");
    }
}

/// Shows `notification` and calls `on_click` from a background thread when it is clicked.
pub fn send(notification: &Notification, on_click: impl FnOnce() + Send + 'static) {
    let title = notification.title.clone();
    let body = notification.body.clone();
    let slot = WaitSlot::claim(&WAITING);
    let spawned = std::thread::Builder::new()
        .name("notification".into())
        .spawn(move || {
            let wait = slot.wait;
            let mut options = MacNotification::new();
            options.wait_for_click(wait);
            options.asynchronous(!wait);
            let response =
                mac_notification_sys::send_notification(&title, None, &body, Some(&options));
            drop(slot);
            handle_response(response, on_click);
        });
    if let Err(e) = spawned {
        log::warn!("notification thread could not start: {e}");
    }
}

/// What a click on a notification does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClickAction {
    ShowPopup,
    /// Opens the run page, already checked against its account.
    OpenUrl(Url),
}

/// Resolves a click target against the config. A run of an unknown account, or a URL outside
/// the account's host, shows the popup.
pub fn click_action(target: &ClickTarget, config: &Config) -> ClickAction {
    let ClickTarget::Run { account_id, url } = target else {
        return ClickAction::ShowPopup;
    };
    let Some(account) = config.account(account_id) else {
        return ClickAction::ShowPopup;
    };
    match check_url(account, url) {
        Ok(checked) => ClickAction::OpenUrl(checked),
        Err(e) => {
            log::warn!("notification link rejected: {e}");
            ClickAction::ShowPopup
        }
    }
}

/// The bundle notifications are attributed to.
fn attributed_bundle(bundle_id: &str, dev: bool) -> &str {
    if dev {
        "com.apple.Terminal"
    } else {
        bundle_id
    }
}

/// Calls `on_click` for a click; logs a failed delivery.
fn handle_response<E: std::fmt::Display>(
    response: Result<NotificationResponse, E>,
    on_click: impl FnOnce(),
) {
    match response {
        Ok(NotificationResponse::Click) => on_click(),
        Ok(_) => {}
        Err(e) => log::warn!("notification failed: {e}"),
    }
}

/// One notification's place in the waiting count, given back when dropped.
struct WaitSlot {
    counter: &'static AtomicUsize,
    /// Whether this notification waits for a click: only the first `MAX_WAITING` do.
    wait: bool,
}

impl WaitSlot {
    fn claim(counter: &'static AtomicUsize) -> WaitSlot {
        let before = counter.fetch_add(1, Ordering::SeqCst);
        WaitSlot {
            counter,
            wait: before < MAX_WAITING,
        }
    }
}

impl Drop for WaitSlot {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::config::{Account, AccountKind};

    #[test]
    fn only_the_first_slots_wait_and_dropping_frees_them() {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let slots: Vec<WaitSlot> = (0..MAX_WAITING)
            .map(|_| WaitSlot::claim(&COUNTER))
            .collect();
        assert!(slots.iter().all(|s| s.wait));
        let over = WaitSlot::claim(&COUNTER);
        assert!(!over.wait);
        assert_eq!(COUNTER.load(Ordering::SeqCst), MAX_WAITING + 1);

        drop(over);
        drop(slots);
        assert_eq!(COUNTER.load(Ordering::SeqCst), 0);
        assert!(WaitSlot::claim(&COUNTER).wait);
        assert_eq!(COUNTER.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn only_a_click_runs_the_handler() {
        let clicked = Cell::new(0);
        let click = || clicked.set(clicked.get() + 1);
        handle_response::<String>(Ok(NotificationResponse::Click), click);
        handle_response::<String>(Ok(NotificationResponse::None), click);
        handle_response::<String>(Ok(NotificationResponse::CloseButton("x".into())), click);
        handle_response(Err("not delivered".to_string()), click);
        assert_eq!(clicked.get(), 1);
    }

    #[test]
    fn development_runs_borrow_the_terminal_bundle() {
        assert_eq!(
            attributed_bundle("test.example.app", true),
            "com.apple.Terminal"
        );
        assert_eq!(
            attributed_bundle("test.example.app", false),
            "test.example.app"
        );
    }

    #[test]
    fn click_targets_resolve_against_the_config() {
        let account = Account::new(AccountKind::GitHub, "a", None);
        let mut config = Config::default();
        config.accounts.push(account.clone());
        let run = |account_id: &str, url: &str| ClickTarget::Run {
            account_id: account_id.into(),
            url: url.into(),
        };

        assert_eq!(
            click_action(&ClickTarget::Popup, &config),
            ClickAction::ShowPopup
        );
        let page = "https://github.com/acme/r1/actions/runs/1";
        assert_eq!(
            click_action(&run(&account.id, page), &config),
            ClickAction::OpenUrl(Url::parse(page).unwrap())
        );
        assert_eq!(
            click_action(&run("gone", page), &config),
            ClickAction::ShowPopup
        );
        assert_eq!(
            click_action(&run(&account.id, "https://evil.example.test/x"), &config),
            ClickAction::ShowPopup
        );
    }
}
