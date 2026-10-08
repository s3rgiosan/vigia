//! Menu bar icon: the app's porthole mark with a status badge, tooltip, right click menu,
//! popup toggle.
//!
//! The porthole is always a template image, so macOS tints it like every other menu bar icon.
//! Failed, error, running and passing add a colored badge with a glyph in the lower right
//! corner. The badge is an image view laid over the status bar button (see `badge`), drawn in
//! the system colors for the menu bar's current light or dark appearance.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use tauri::image::Image;
#[cfg(debug_assertions)]
use tauri::menu::Submenu;
use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalRect, PhysicalSize, Runtime};
use tauri_plugin_positioner::{Position, WindowExt};

use crate::model::TrayColor;

pub const TRAY_ID: &str = "vigia-tray";
pub const POPUP_LABEL: &str = "popup";

/// Clicking the tray icon blurs the popup first, which hides it. A toggle arriving within this
/// window after a blur-hide would reopen it, so it is ignored.
const BLUR_TOGGLE_GUARD: Duration = Duration::from_millis(200);

/// What the tray remembers between events.
#[derive(Debug)]
struct TrayState {
    /// When the popup was last hidden because it lost focus.
    last_blur_hide: Option<Instant>,
    /// What the tray currently shows, so a change of menu bar appearance can redraw it.
    shown: Option<(TrayLook, MenuBar)>,
}

static STATE: Mutex<TrayState> = Mutex::new(TrayState {
    last_blur_hide: None,
    shown: None,
});

/// Locks the tray state. The state stays usable after a panic elsewhere, since every field is
/// written in a single assignment.
fn state() -> MutexGuard<'static, TrayState> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Records that the popup was hidden because it lost focus.
pub fn note_blur_hide() {
    state().last_blur_hide = Some(Instant::now());
}

fn within_blur_guard() -> bool {
    let last = state().last_blur_hide;
    blur_guard_active(last, Instant::now())
}

/// Whether a toggle at `now` comes too soon after the blur-hide at `last` to be honored.
fn blur_guard_active(last: Option<Instant>, now: Instant) -> bool {
    last.is_some_and(|at| now.saturating_duration_since(at) < BLUR_TOGGLE_GUARD)
}

/// What a tray menu item does, by its ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    Open,
    Quit,
    Refresh,
    Pause,
    Settings,
    CheckForUpdates,
    /// Debug builds: force one icon.
    #[cfg(debug_assertions)]
    Look(TrayLook),
}

impl MenuAction {
    fn from_id(id: &str) -> Option<MenuAction> {
        match id {
            "open" => Some(MenuAction::Open),
            "quit" => Some(MenuAction::Quit),
            "refresh" => Some(MenuAction::Refresh),
            "pause" => Some(MenuAction::Pause),
            "settings" => Some(MenuAction::Settings),
            "check-updates" => Some(MenuAction::CheckForUpdates),
            #[cfg(debug_assertions)]
            _ => TrayLook::from_menu_id(id).map(MenuAction::Look),
            #[cfg(not(debug_assertions))]
            _ => None,
        }
    }
}

/// What a click on the tray icon does to the popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PopupToggle {
    Hide,
    Show,
    /// The click is the one that just blurred and hid the popup.
    Ignore,
}

fn popup_toggle(visible: bool, blur_guarded: bool) -> PopupToggle {
    if visible {
        PopupToggle::Hide
    } else if blur_guarded {
        PopupToggle::Ignore
    } else {
        PopupToggle::Show
    }
}

/// What the menu bar icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayLook {
    Failed,
    Error,
    Running,
    Success,
    Idle,
    Paused,
}

impl TrayLook {
    #[cfg(any(debug_assertions, test))]
    pub const ALL: [TrayLook; 6] = [
        TrayLook::Failed,
        TrayLook::Error,
        TrayLook::Running,
        TrayLook::Success,
        TrayLook::Idle,
        TrayLook::Paused,
    ];

    pub fn from_snapshot(color: TrayColor, paused: bool) -> TrayLook {
        if paused {
            return TrayLook::Paused;
        }
        match color {
            TrayColor::Red => TrayLook::Failed,
            TrayColor::Orange => TrayLook::Error,
            TrayColor::Yellow => TrayLook::Running,
            TrayColor::Green => TrayLook::Success,
            TrayColor::Gray => TrayLook::Idle,
        }
    }

    /// Which template image the status bar button shows.
    fn icon_kind(self) -> IconKind {
        match self {
            TrayLook::Idle => IconKind::Plain,
            TrayLook::Paused => IconKind::Paused,
            TrayLook::Failed | TrayLook::Error | TrayLook::Running | TrayLook::Success => {
                IconKind::Badged
            }
        }
    }

    fn icon(self) -> Image<'static> {
        match self.icon_kind() {
            IconKind::Plain => tauri::include_image!("icons/tray/porthole@2x.png"),
            IconKind::Paused => tauri::include_image!("icons/tray/paused@2x.png"),
            IconKind::Badged => tauri::include_image!("icons/tray/porthole-badged@2x.png"),
        }
    }

    /// The badge PNG (2x) for the menu bar's appearance; idle and paused have none.
    fn badge(self, bar: MenuBar) -> Option<&'static [u8]> {
        use MenuBar::{Dark, Light};
        let png: &'static [u8] = match (self, bar) {
            (TrayLook::Idle | TrayLook::Paused, _) => return None,
            (TrayLook::Failed, Light) => include_bytes!("../icons/tray/badge-light-failed@2x.png"),
            (TrayLook::Failed, Dark) => include_bytes!("../icons/tray/badge-dark-failed@2x.png"),
            (TrayLook::Error, Light) => include_bytes!("../icons/tray/badge-light-error@2x.png"),
            (TrayLook::Error, Dark) => include_bytes!("../icons/tray/badge-dark-error@2x.png"),
            (TrayLook::Running, Light) => {
                include_bytes!("../icons/tray/badge-light-running@2x.png")
            }
            (TrayLook::Running, Dark) => include_bytes!("../icons/tray/badge-dark-running@2x.png"),
            (TrayLook::Success, Light) => {
                include_bytes!("../icons/tray/badge-light-passing@2x.png")
            }
            (TrayLook::Success, Dark) => {
                include_bytes!("../icons/tray/badge-dark-passing@2x.png")
            }
        };
        Some(png)
    }

    #[cfg(any(debug_assertions, test))]
    fn label(self) -> &'static str {
        match self {
            TrayLook::Failed => "Failed",
            TrayLook::Error => "Error",
            TrayLook::Running => "Running",
            TrayLook::Success => "Passing",
            TrayLook::Idle => "Idle",
            TrayLook::Paused => "Paused",
        }
    }

    #[cfg(any(debug_assertions, test))]
    fn menu_id(self) -> String {
        format!("debug-look-{}", self.label().to_lowercase())
    }

    #[cfg(any(debug_assertions, test))]
    fn from_menu_id(id: &str) -> Option<TrayLook> {
        TrayLook::ALL.into_iter().find(|l| l.menu_id() == id)
    }
}

/// The template images: the porthole alone, dimmed for pause, or notched for a badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IconKind {
    Plain,
    Paused,
    Badged,
}

/// Appearance of the menu bar behind the status item. It follows the wallpaper and can differ
/// from the app's own appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuBar {
    Light,
    Dark,
}

/// The work `show` does to move from what is shown to the wanted look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Redraw {
    /// Nothing was drawn yet and no look was given.
    Nothing,
    /// Same look on the same menu bar: only the badge is placed again.
    PlaceBadge,
    /// Sets the badge for `look`, and the template image too when `icon` is set.
    Draw { look: TrayLook, icon: bool },
}

fn redraw(shown: Option<(TrayLook, MenuBar)>, look: Option<TrayLook>, bar: MenuBar) -> Redraw {
    let Some(look) = look.or(shown.map(|(look, _)| look)) else {
        return Redraw::Nothing;
    };
    if shown == Some((look, bar)) {
        return Redraw::PlaceBadge;
    }
    let icon = shown.is_none_or(|(before, _)| look.icon_kind() != before.icon_kind());
    Redraw::Draw { look, icon }
}

#[cfg(target_os = "macos")]
fn menu_bar<R: Runtime>(tray: &TrayIcon<R>) -> MenuBar {
    tray.with_inner_tray_icon(|inner| {
        use objc2_app_kit::{
            NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
        };
        use objc2_foundation::{MainThreadMarker, NSArray};

        let Some(mtm) = MainThreadMarker::new() else {
            return MenuBar::Light;
        };
        let Some(item) = inner.ns_status_item() else {
            return MenuBar::Light;
        };
        let Some(button) = item.button(mtm) else {
            return MenuBar::Light;
        };
        let appearance = button.effectiveAppearance();
        // SAFETY: the appearance name constants are immutable statics provided by AppKit.
        let names =
            unsafe { NSArray::from_slice(&[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]) };
        let best = appearance.bestMatchFromAppearancesWithNames(&names);
        // SAFETY: as above.
        let dark = unsafe { NSAppearanceNameDarkAqua };
        if best.is_some_and(|name| &*name == dark) {
            MenuBar::Dark
        } else {
            MenuBar::Light
        }
    })
    .unwrap_or(MenuBar::Light)
}

#[cfg(not(target_os = "macos"))]
fn menu_bar<R: Runtime>(_tray: &TrayIcon<R>) -> MenuBar {
    MenuBar::Light
}

/// Draws `look` (or the look already shown, when `None`) for the current menu bar appearance
/// and skips the work when nothing changed. Must run on the main thread: that serializes the
/// read-compare-set-store sequence across every caller. The state lock is released before
/// each tray call.
fn show<R: Runtime>(tray: &TrayIcon<R>, look: Option<TrayLook>) -> tauri::Result<()> {
    let bar = menu_bar(tray);
    let shown = state().shown;
    match redraw(shown, look, bar) {
        Redraw::Nothing => {}
        // The button can change size with the menu bar, so the badge is placed on every pass.
        Redraw::PlaceBadge => place_badge(tray),
        Redraw::Draw { look, icon } => {
            if icon {
                tray.set_icon_with_as_template(Some(look.icon()), true)?;
            }
            set_badge(tray, look.badge(bar));
            state().shown = Some((look, bar));
        }
    }
    Ok(())
}

/// Runs `f` with the status bar button on the main thread.
#[cfg(target_os = "macos")]
fn with_button<R: Runtime>(
    tray: &TrayIcon<R>,
    f: impl FnOnce(&objc2_app_kit::NSStatusBarButton, objc2::MainThreadMarker) + Send + 'static,
) {
    let _ = tray.with_inner_tray_icon(move |inner| {
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return;
        };
        if let Some(button) = inner.ns_status_item().and_then(|item| item.button(mtm)) {
            f(&button, mtm);
        }
    });
}

#[cfg(target_os = "macos")]
fn set_badge<R: Runtime>(tray: &TrayIcon<R>, png: Option<&'static [u8]>) {
    with_button(tray, move |button, mtm| badge::set(button, mtm, png));
}

#[cfg(not(target_os = "macos"))]
fn set_badge<R: Runtime>(_tray: &TrayIcon<R>, _png: Option<&'static [u8]>) {}

#[cfg(target_os = "macos")]
fn place_badge<R: Runtime>(tray: &TrayIcon<R>) {
    with_button(tray, |button, _| badge::place(button));
}

#[cfg(not(target_os = "macos"))]
fn place_badge<R: Runtime>(_tray: &TrayIcon<R>) {}

/// What VoiceOver reads for the status item: the app's name and the tooltip's state.
fn accessibility_label(tooltip: &str) -> String {
    if tooltip.is_empty() {
        return "Vigia".to_string();
    }
    format!("Vigia, {tooltip}")
}

/// Names the status bar button for VoiceOver, in step with the tooltip.
#[cfg(target_os = "macos")]
fn set_accessibility_label<R: Runtime>(tray: &TrayIcon<R>, tooltip: &str) {
    let label = accessibility_label(tooltip);
    with_button(tray, move |button, _| {
        use objc2_app_kit::NSAccessibility;
        button.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&label)));
    });
}

#[cfg(not(target_os = "macos"))]
fn set_accessibility_label<R: Runtime>(_tray: &TrayIcon<R>, _tooltip: &str) {}

/// Redraws the icon whenever the menu bar turns light or dark.
///
/// The status bar button's effective appearance is the menu bar's, so it changes with the
/// system appearance and with the wallpaper behind the menu bar. AppKit reports each change to
/// the button's subviews through `viewDidChangeEffectiveAppearance`, so an empty subview
/// observes every change and no polling is needed.
#[cfg(target_os = "macos")]
fn observe_appearance<R: Runtime>(tray: &TrayIcon<R>) {
    let app = tray.app_handle().clone();
    with_button(tray, move |button, mtm| {
        appearance::observe(button, mtm, move || {
            // The callback can run inside a tray call that holds the tray icon, so the redraw is
            // queued from another thread and runs on a later main loop turn.
            let app = app.clone();
            tauri::async_runtime::spawn(async move { refresh_appearance(&app) });
        });
    });
}

#[cfg(not(target_os = "macos"))]
fn observe_appearance<R: Runtime>(_tray: &TrayIcon<R>) {}

/// An empty, zero-sized view added to the status bar button that calls back when the button's
/// effective appearance changes.
#[cfg(target_os = "macos")]
mod appearance {
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSAccessibility, NSResponder, NSStatusBarButton, NSView};
    use objc2_foundation::NSRect;

    pub struct Ivars {
        on_change: Box<dyn Fn()>,
    }

    define_class!(
        // SAFETY: NSView has no subclassing requirements beyond calling super in overrides, and
        // this class does not implement Drop.
        #[unsafe(super(NSView, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "VigiaAppearanceObserver"]
        #[ivars = Ivars]
        struct Observer;

        impl Observer {
            #[unsafe(method(viewDidChangeEffectiveAppearance))]
            fn did_change_effective_appearance(&self) {
                // SAFETY: NSView implements the method and it takes no arguments.
                let _: () = unsafe { msg_send![super(self), viewDidChangeEffectiveAppearance] };
                (self.ivars().on_change)();
            }
        }
    );

    /// Adds the observer to `button`, which keeps it alive for the life of the status item.
    pub fn observe(
        button: &NSStatusBarButton,
        mtm: MainThreadMarker,
        on_change: impl Fn() + 'static,
    ) {
        let this = Observer::alloc(mtm).set_ivars(Ivars {
            on_change: Box::new(on_change),
        });
        // SAFETY: NSView's designated initializer takes a frame and returns an initialized view.
        let view: Retained<Observer> =
            unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        view.setAccessibilityElement(false);
        button.addSubview(&view);
    }
}

/// The status badge: an `NSImageView` added to the status bar button, below the transparent
/// view tray-icon uses to receive clicks, so clicks still reach the tray. Its image is not a
/// template, so it keeps its color while the porthole under it is tinted.
#[cfg(target_os = "macos")]
mod badge {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{
        NSAccessibility, NSImage, NSImageScaling, NSImageView, NSStatusBarButton,
        NSWindowOrderingMode,
    };
    use objc2_foundation::{NSData, NSPoint, NSRect, NSSize};

    /// Center of the badge on the 18 pt porthole, from its top left corner, and the badge's
    /// side. Both match `BADGE_CENTER` and `BADGE_SIZE` in scripts/gen-tray-icons.py.
    const BADGE_CENTER: (f64, f64) = (14.5, 14.5);
    const BADGE_SIZE: f64 = 7.0;
    const ICON_SIZE: f64 = 18.0;

    thread_local! {
        // AppKit objects live on the main thread, which is the only thread that touches this.
        static VIEW: RefCell<Option<Retained<NSImageView>>> = const { RefCell::new(None) };
    }

    /// Shows `png` as the badge, or hides the badge when `None`.
    pub fn set(button: &NSStatusBarButton, mtm: MainThreadMarker, png: Option<&'static [u8]>) {
        let Some(png) = png else {
            VIEW.with(|cell| {
                if let Some(view) = cell.borrow().as_ref() {
                    view.setHidden(true);
                }
            });
            return;
        };
        let data = NSData::with_bytes(png);
        let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
            log::warn!("tray badge image could not be decoded");
            return;
        };
        image.setSize(NSSize::new(BADGE_SIZE, BADGE_SIZE));
        image.setTemplate(false);
        VIEW.with(|cell| {
            let mut slot = cell.borrow_mut();
            let view = slot.get_or_insert_with(|| {
                let view = NSImageView::imageViewWithImage(&image, mtm);
                view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
                // The status item's tooltip carries the state in words.
                view.setAccessibilityElement(false);
                button.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, None);
                view
            });
            view.setImage(Some(&image));
            view.setFrame(frame(button));
            view.setHidden(false);
        });
    }

    /// Moves the badge to the porthole's lower right corner within the current button bounds.
    pub fn place(button: &NSStatusBarButton) {
        VIEW.with(|cell| {
            if let Some(view) = cell.borrow().as_ref() {
                view.setFrame(frame(button));
            }
        });
    }

    fn frame(button: &NSStatusBarButton) -> NSRect {
        let bounds = button.bounds();
        let image = button.cell().map(|cell| cell.imageRectForBounds(bounds));
        badge_frame(icon_rect(bounds, image), button.isFlipped())
    }

    /// The porthole's rect: the cell's image rect when it has an area, otherwise an 18 pt
    /// square centered in `bounds`.
    fn icon_rect(bounds: NSRect, image: Option<NSRect>) -> NSRect {
        image
            .filter(|rect| rect.size.width > 0.0 && rect.size.height > 0.0)
            .unwrap_or_else(|| centered_icon(bounds))
    }

    /// The badge's rect over the porthole at `image`, scaled with it. `flipped` is the button's
    /// coordinate system, with y growing down.
    fn badge_frame(image: NSRect, flipped: bool) -> NSRect {
        let scale = image.size.height / ICON_SIZE;
        let side = BADGE_SIZE * scale;
        let x = image.origin.x + (BADGE_CENTER.0 - BADGE_SIZE / 2.0) * scale;
        let from_top = (BADGE_CENTER.1 - BADGE_SIZE / 2.0) * scale;
        let y = if flipped {
            image.origin.y + from_top
        } else {
            image.origin.y + image.size.height - from_top - side
        };
        NSRect::new(NSPoint::new(x, y), NSSize::new(side, side))
    }

    fn centered_icon(bounds: NSRect) -> NSRect {
        NSRect::new(
            NSPoint::new(
                bounds.origin.x + (bounds.size.width - ICON_SIZE) / 2.0,
                bounds.origin.y + (bounds.size.height - ICON_SIZE) / 2.0,
            ),
            NSSize::new(ICON_SIZE, ICON_SIZE),
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
            NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
        }

        #[test]
        fn badge_sits_in_the_lower_right_of_the_porthole() {
            let icon = rect(10.0, 2.0, 18.0, 18.0);
            // Flipped: y grows down, so the badge is 11 pt below the icon's top edge.
            assert_eq!(badge_frame(icon, true), rect(21.0, 13.0, 7.0, 7.0));
            // Unflipped: the same spot measured from the bottom edge.
            assert_eq!(badge_frame(icon, false), rect(21.0, 2.0, 7.0, 7.0));
        }

        #[test]
        fn badge_scales_with_the_porthole() {
            let icon = rect(0.0, 0.0, 36.0, 36.0);
            assert_eq!(badge_frame(icon, true), rect(22.0, 22.0, 14.0, 14.0));
        }

        #[test]
        fn empty_image_rect_falls_back_to_a_centered_icon() {
            let bounds = rect(0.0, 0.0, 30.0, 22.0);
            let centered = rect(6.0, 2.0, 18.0, 18.0);
            assert_eq!(icon_rect(bounds, None), centered);
            assert_eq!(icon_rect(bounds, Some(rect(4.0, 4.0, 0.0, 18.0))), centered);
            let image = rect(5.0, 1.0, 20.0, 20.0);
            assert_eq!(icon_rect(bounds, Some(image)), image);
        }
    }
}

/// Redraws the icon when the menu bar switched between light and dark since the last update.
/// On the main thread it redraws at once; elsewhere the redraw is queued on the main loop.
pub fn refresh_appearance<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(tray) = handle.tray_by_id(TRAY_ID) {
            let _ = show(&tray, None);
        }
    });
}

/// Menu item handlers the app wires in; the tray itself only knows the IDs.
pub trait TrayActions: Send + Sync + 'static {
    fn refresh(&self);
    fn toggle_pause(&self);
    fn open_settings(&self);
    fn check_for_updates(&self);
}

/// The About panel's contents. On macOS `version` fills the application version, the empty
/// `short_version` hides the build number in parentheses, and `credits` shows the description.
fn about_metadata(version: String) -> AboutMetadata<'static> {
    AboutMetadata {
        name: Some("Vigia".into()),
        version: Some(version),
        short_version: Some(String::new()),
        copyright: Some("© 2026 Sérgio Santos".into()),
        credits: Some("CI status for GitHub Actions and GitLab CI, in the menu bar.".into()),
        ..Default::default()
    }
}

/// Builds the tray and returns it with the Pause item, whose label flips to Resume.
pub fn build<R: Runtime>(
    app: &AppHandle<R>,
    actions: Box<dyn TrayActions>,
) -> tauri::Result<(TrayIcon<R>, MenuItem<R>)> {
    let open = MenuItem::with_id(app, "open", "Open Vigia", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh Now", true, Some("CmdOrCtrl+R"))?;
    let pause = MenuItem::with_id(app, "pause", "Pause", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
    let version = app.package_info().version.to_string();
    let about = PredefinedMenuItem::about(app, Some("About Vigia"), Some(about_metadata(version)))?;
    let check_updates = MenuItem::with_id(
        app,
        "check-updates",
        "Check for Updates…",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit Vigia", true, Some("CmdOrCtrl+Q"))?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &PredefinedMenuItem::separator(app)?,
            &refresh,
            &pause,
            &PredefinedMenuItem::separator(app)?,
            &settings,
            &about,
            &check_updates,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    // Debug builds get a submenu that forces each icon, so every state can be checked by eye.
    #[cfg(debug_assertions)]
    {
        let debug = Submenu::with_id(app, "debug", "Debug", true)?;
        for look in TrayLook::ALL {
            debug.append(&MenuItem::with_id(
                app,
                look.menu_id(),
                look.label(),
                true,
                None::<&str>,
            )?)?;
        }
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&debug)?;
    }

    let initial = TrayLook::Idle;
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(initial.icon())
        .icon_as_template(true)
        .tooltip("Vigia")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| {
            let Some(action) = MenuAction::from_id(event.id().as_ref()) else {
                return;
            };
            match action {
                MenuAction::Open => show_popup(app),
                MenuAction::Quit => app.exit(0),
                MenuAction::Refresh => actions.refresh(),
                MenuAction::Pause => actions.toggle_pause(),
                MenuAction::Settings => actions.open_settings(),
                MenuAction::CheckForUpdates => actions.check_for_updates(),
                #[cfg(debug_assertions)]
                MenuAction::Look(look) => {
                    let _ = set_look(app, look, look.label());
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_popup(tray.app_handle());
            }
        })
        .build(app)?;
    state().shown = Some((initial, MenuBar::Light));
    set_accessibility_label(&tray, "");
    observe_appearance(&tray);
    Ok((tray, pause))
}

pub fn set_look<R: Runtime>(
    app: &AppHandle<R>,
    look: TrayLook,
    tooltip: &str,
) -> tauri::Result<()> {
    let handle = app.clone();
    let tooltip = tooltip.to_string();
    app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id(TRAY_ID) else {
            return;
        };
        if let Err(e) = show(&tray, Some(look)) {
            log::warn!("tray icon could not be updated: {e}");
        }
        set_accessibility_label(&tray, &tooltip);
        if let Err(e) = tray.set_tooltip(Some(tooltip)) {
            log::warn!("tray tooltip could not be updated: {e}");
        }
    })
}

pub fn set_pause_label<R: Runtime>(item: &MenuItem<R>, paused: bool) {
    let _ = item.set_text(pause_label(paused));
}

fn pause_label(paused: bool) -> &'static str {
    if paused {
        "Resume"
    } else {
        "Pause"
    }
}

/// Shows the status item pressed while the popup is open, like a menu bar menu.
pub fn set_highlight<R: Runtime>(app: &AppHandle<R>, on: bool) {
    #[cfg(target_os = "macos")]
    {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Some(tray) = handle.tray_by_id(TRAY_ID) {
                with_button(&tray, move |button, _| button.highlight(on));
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, on);
}

pub fn toggle_popup<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(POPUP_LABEL) else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    match popup_toggle(visible, within_blur_guard()) {
        PopupToggle::Hide => hide_popup(app),
        PopupToggle::Show => show_popup(app),
        PopupToggle::Ignore => {}
    }
}

/// Shows the popup under the tray icon and focuses it so that a later blur hides it.
pub fn show_popup<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(POPUP_LABEL) else {
        return;
    };
    let placed = window
        .as_ref()
        .window()
        .move_window(Position::TrayBottomCenter);
    if placed.is_err() {
        place_without_tray_rect(app, &window);
    }
    let _ = window.show();
    let _ = window.set_focus();
    set_highlight(app, true);
}

/// Positions the popup when the positioner has no tray rect yet: below the tray icon if the
/// icon reports its rect, otherwise in the top-right corner of the primary monitor's work area.
fn place_without_tray_rect<R: Runtime>(app: &AppHandle<R>, window: &tauri::WebviewWindow<R>) {
    let Ok(size) = window.outer_size() else {
        return;
    };
    let tray_rect = app
        .tray_by_id(TRAY_ID)
        .and_then(|tray| tray.rect().ok().flatten());
    if let Some(rect) = tray_rect {
        let scale = window.scale_factor().unwrap_or(1.0);
        let position = rect.position.to_physical::<f64>(scale);
        let icon = rect.size.to_physical::<f64>(scale);
        let _ = window.set_position(below_icon(position, icon, size.width));
        return;
    }
    if let Ok(Some(monitor)) = app.primary_monitor() {
        let _ = window.set_position(top_right(monitor.work_area(), size.width));
    }
}

/// Centers a window `width` wide under the tray icon.
fn below_icon(
    icon_position: PhysicalPosition<f64>,
    icon_size: PhysicalSize<f64>,
    width: u32,
) -> PhysicalPosition<i32> {
    let x = icon_position.x + icon_size.width / 2.0 - f64::from(width) / 2.0;
    let y = icon_position.y + icon_size.height;
    PhysicalPosition::new(x as i32, y as i32)
}

/// Places a window `width` wide in the top-right corner of a monitor's work area.
fn top_right(area: &PhysicalRect<i32, u32>, width: u32) -> PhysicalPosition<i32> {
    let x = area.position.x + area.size.width as i32 - width as i32;
    PhysicalPosition::new(x, area.position.y)
}

pub fn hide_popup<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(POPUP_LABEL) {
        let _ = window.hide();
    }
    set_highlight(app, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_round_trip() {
        for look in TrayLook::ALL {
            assert_eq!(TrayLook::from_menu_id(&look.menu_id()), Some(look));
        }
        assert_eq!(TrayLook::from_menu_id("quit"), None);
    }

    #[test]
    fn badged_looks_have_a_distinct_badge_per_menu_bar() {
        let badged = [
            TrayLook::Failed,
            TrayLook::Error,
            TrayLook::Running,
            TrayLook::Success,
        ];
        let mut seen = Vec::new();
        for look in badged {
            assert_eq!(look.icon_kind(), IconKind::Badged);
            for bar in [MenuBar::Light, MenuBar::Dark] {
                let png = look.badge(bar).expect("badged looks have a badge");
                assert!(png.starts_with(b"\x89PNG"));
                assert!(!seen.contains(&png), "{look:?} {bar:?} repeats a badge");
                seen.push(png);
            }
        }
        assert_eq!(TrayLook::Idle.icon_kind(), IconKind::Plain);
        assert_eq!(TrayLook::Paused.icon_kind(), IconKind::Paused);
    }

    #[test]
    fn paused_wins_and_idle_looks_have_no_badge() {
        assert_eq!(
            TrayLook::from_snapshot(TrayColor::Red, true),
            TrayLook::Paused
        );
        assert_eq!(
            TrayLook::from_snapshot(TrayColor::Green, false),
            TrayLook::Success
        );
        for bar in [MenuBar::Light, MenuBar::Dark] {
            assert!(TrayLook::Idle.badge(bar).is_none());
            assert!(TrayLook::Paused.badge(bar).is_none());
        }
    }

    #[test]
    fn every_tray_color_maps_to_its_look() {
        let cases = [
            (TrayColor::Red, TrayLook::Failed),
            (TrayColor::Orange, TrayLook::Error),
            (TrayColor::Yellow, TrayLook::Running),
            (TrayColor::Green, TrayLook::Success),
            (TrayColor::Gray, TrayLook::Idle),
        ];
        for (color, look) in cases {
            assert_eq!(TrayLook::from_snapshot(color, false), look);
            assert_eq!(TrayLook::from_snapshot(color, true), TrayLook::Paused);
        }
    }

    #[test]
    fn every_look_has_a_template_icon_and_a_label() {
        for look in TrayLook::ALL {
            let icon = look.icon();
            assert!(icon.width() > 0 && icon.height() > 0);
            assert!(!look.label().is_empty());
        }
        assert_eq!(TrayLook::Success.label(), "Passing");
    }

    #[test]
    fn about_panel_shows_the_version_once_and_the_description_as_credits() {
        let about = about_metadata("9.8.7".into());
        assert_eq!(about.name.as_deref(), Some("Vigia"));
        assert_eq!(about.version.as_deref(), Some("9.8.7"));
        assert_eq!(about.short_version.as_deref(), Some(""));
        assert_eq!(about.copyright.as_deref(), Some("© 2026 Sérgio Santos"));
        assert_eq!(
            about.credits.as_deref(),
            Some("CI status for GitHub Actions and GitLab CI, in the menu bar.")
        );
        assert_eq!(about.comments, None);
    }

    #[test]
    fn menu_ids_map_to_actions() {
        assert_eq!(MenuAction::from_id("open"), Some(MenuAction::Open));
        assert_eq!(MenuAction::from_id("quit"), Some(MenuAction::Quit));
        assert_eq!(MenuAction::from_id("refresh"), Some(MenuAction::Refresh));
        assert_eq!(MenuAction::from_id("pause"), Some(MenuAction::Pause));
        assert_eq!(MenuAction::from_id("settings"), Some(MenuAction::Settings));
        assert_eq!(
            MenuAction::from_id("check-updates"),
            Some(MenuAction::CheckForUpdates)
        );
        #[cfg(debug_assertions)]
        assert_eq!(
            MenuAction::from_id("debug-look-failed"),
            Some(MenuAction::Look(TrayLook::Failed))
        );
        assert_eq!(MenuAction::from_id("debug"), None);
    }

    #[test]
    fn blur_guard_covers_only_the_moments_after_a_blur_hide() {
        let at = Instant::now();
        assert!(!blur_guard_active(None, at));
        assert!(blur_guard_active(Some(at), at));
        let ms = Duration::from_millis;
        assert!(blur_guard_active(Some(at), at + ms(199)));
        assert!(!blur_guard_active(Some(at), at + ms(200)));
        // A clock reading from before the hide counts as no time passed.
        assert!(blur_guard_active(Some(at + ms(5)), at));
    }

    #[test]
    fn popup_toggle_hides_shows_or_ignores_the_click() {
        assert_eq!(popup_toggle(true, false), PopupToggle::Hide);
        assert_eq!(popup_toggle(true, true), PopupToggle::Hide);
        assert_eq!(popup_toggle(false, false), PopupToggle::Show);
        assert_eq!(popup_toggle(false, true), PopupToggle::Ignore);
    }

    #[test]
    fn redraw_does_only_the_work_the_change_needs() {
        use MenuBar::{Dark, Light};
        assert_eq!(redraw(None, None, Light), Redraw::Nothing);
        assert_eq!(
            redraw(None, Some(TrayLook::Idle), Light),
            Redraw::Draw {
                look: TrayLook::Idle,
                icon: true
            }
        );
        let shown = Some((TrayLook::Failed, Light));
        assert_eq!(redraw(shown, None, Light), Redraw::PlaceBadge);
        assert_eq!(
            redraw(shown, Some(TrayLook::Failed), Light),
            Redraw::PlaceBadge
        );
        // A menu bar appearance change redraws the badge of the shown look.
        assert_eq!(
            redraw(shown, None, Dark),
            Redraw::Draw {
                look: TrayLook::Failed,
                icon: false
            }
        );
        // Badged looks share one template image.
        assert_eq!(
            redraw(shown, Some(TrayLook::Running), Light),
            Redraw::Draw {
                look: TrayLook::Running,
                icon: false
            }
        );
        assert_eq!(
            redraw(shown, Some(TrayLook::Paused), Light),
            Redraw::Draw {
                look: TrayLook::Paused,
                icon: true
            }
        );
    }

    #[test]
    fn accessibility_label_names_the_app_and_the_state() {
        assert_eq!(accessibility_label(""), "Vigia");
        assert_eq!(accessibility_label("Paused"), "Vigia, Paused");
        assert_eq!(
            accessibility_label("2 failed, 3 passing"),
            "Vigia, 2 failed, 3 passing"
        );
    }

    #[test]
    fn pause_item_reads_resume_while_paused() {
        assert_eq!(pause_label(true), "Resume");
        assert_eq!(pause_label(false), "Pause");
    }

    #[test]
    fn popup_centers_under_the_icon() {
        let position = below_icon(
            PhysicalPosition::new(1000.0, 0.0),
            PhysicalSize::new(44.0, 48.0),
            400,
        );
        assert_eq!(position, PhysicalPosition::new(822, 48));
    }

    #[test]
    fn popup_without_an_icon_rect_goes_top_right() {
        let area = PhysicalRect {
            position: PhysicalPosition::new(0, 50),
            size: PhysicalSize::new(3024, 1900),
        };
        assert_eq!(top_right(&area, 400), PhysicalPosition::new(2624, 50));
        let second = PhysicalRect {
            position: PhysicalPosition::new(-1920, 0),
            size: PhysicalSize::new(1920, 1080),
        };
        assert_eq!(top_right(&second, 400), PhysicalPosition::new(-400, 0));
    }
}
