//! Per-window zoom for windows with app content. The popup has no zoom.
//!
//! Tauri has no zoom getter, so the level each window shows is kept here, by window label.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Mutex, PoisonError};

use tauri::{Runtime, WebviewWindow};

use crate::windows::UnknownId;

/// The zoom factors a window steps through.
pub const LEVELS: [f64; 8] = [0.75, 0.85, 1.0, 1.15, 1.3, 1.5, 1.75, 2.0];
/// The index of 100% in `LEVELS`.
const ACTUAL_SIZE: usize = 2;

/// One zoom command: View → Actual Size, Zoom In or Zoom Out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomStep {
    In,
    Out,
    Reset,
}

impl FromStr for ZoomStep {
    type Err = UnknownId;

    fn from_str(id: &str) -> Result<ZoomStep, UnknownId> {
        match id {
            "in" => Ok(ZoomStep::In),
            "out" => Ok(ZoomStep::Out),
            "reset" => Ok(ZoomStep::Reset),
            _ => Err(UnknownId),
        }
    }
}

/// The level index after `step` from `index`, held within `LEVELS`.
pub fn next_level(index: usize, step: ZoomStep) -> usize {
    match step {
        ZoomStep::In => (index + 1).min(LEVELS.len() - 1),
        ZoomStep::Out => index.saturating_sub(1),
        ZoomStep::Reset => ACTUAL_SIZE,
    }
}

/// The zoom level of each window, by label.
#[derive(Debug, Default)]
pub struct ZoomLevels(Mutex<HashMap<String, usize>>);

impl ZoomLevels {
    /// Applies `step` to the stored level for `label` and returns the new factor.
    pub fn step(&self, label: &str, step: ZoomStep) -> f64 {
        let mut levels = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let current = levels.get(label).copied().unwrap_or(ACTUAL_SIZE);
        let next = next_level(current, step);
        levels.insert(label.to_string(), next);
        LEVELS[next]
    }

    /// Forgets a closed window, so its next instance opens at actual size.
    pub fn forget(&self, label: &str) {
        let mut levels = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        levels.remove(label);
    }
}

/// Zooms `window` one step, unless it is the popup.
pub fn apply<R: Runtime>(levels: &ZoomLevels, window: &WebviewWindow<R>, step: ZoomStep) {
    let label = window.label();
    if label == crate::tray::POPUP_LABEL {
        return;
    }
    let factor = levels.step(label, step);
    if let Err(e) = window.set_zoom(factor) {
        log::warn!("zoom could not be set: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_size_is_one() {
        assert_eq!(LEVELS[ACTUAL_SIZE], 1.0);
    }

    #[test]
    fn steps_stay_within_the_levels() {
        assert_eq!(next_level(ACTUAL_SIZE, ZoomStep::In), ACTUAL_SIZE + 1);
        assert_eq!(next_level(ACTUAL_SIZE, ZoomStep::Out), ACTUAL_SIZE - 1);
        assert_eq!(next_level(0, ZoomStep::Out), 0);
        assert_eq!(next_level(LEVELS.len() - 1, ZoomStep::In), LEVELS.len() - 1);
        assert_eq!(next_level(5, ZoomStep::Reset), ACTUAL_SIZE);
    }

    #[test]
    fn levels_are_kept_per_window() {
        let levels = ZoomLevels::default();
        assert_eq!(levels.step("settings", ZoomStep::In), 1.15);
        assert_eq!(levels.step("settings", ZoomStep::In), 1.3);
        assert_eq!(levels.step("other", ZoomStep::Out), 0.85);
        assert_eq!(levels.step("settings", ZoomStep::Reset), 1.0);
        levels.step("settings", ZoomStep::In);
        levels.forget("settings");
        assert_eq!(levels.step("settings", ZoomStep::Out), 0.85);
    }

    #[test]
    fn step_ids_parse() {
        assert_eq!("in".parse(), Ok(ZoomStep::In));
        assert_eq!("out".parse(), Ok(ZoomStep::Out));
        assert_eq!("reset".parse(), Ok(ZoomStep::Reset));
        assert_eq!("In".parse::<ZoomStep>(), Err(UnknownId));
    }
}
