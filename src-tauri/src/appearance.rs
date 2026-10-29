//! System appearance settings the webviews cannot read themselves: the user's accent colour and
//! Reduce Transparency. On macOS both come from AppKit; elsewhere neither is reported.

use serde::Serialize;

/// What the frontend applies to its root element.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SystemAppearance {
    /// The accent colour as `#rrggbb` in sRGB, or `None` where the platform reports none.
    pub accent: Option<String>,
    /// Whether the user turned on Reduce Transparency.
    pub reduce_transparency: bool,
}

/// Formats sRGB components in the 0 to 1 range as `#rrggbb`; values outside the range are clamped.
pub fn hex_color(red: f64, green: f64, blue: f64) -> String {
    let byte = |c: f64| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(red), byte(green), byte(blue))
}

/// Reads the accent colour and Reduce Transparency from AppKit.
#[cfg(target_os = "macos")]
pub fn read() -> SystemAppearance {
    use objc2_app_kit::{NSColor, NSColorSpace, NSWorkspace};

    let accent = NSColor::controlAccentColor()
        .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
        .map(|c| hex_color(c.redComponent(), c.greenComponent(), c.blueComponent()));
    let reduce_transparency =
        NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency();
    SystemAppearance {
        accent,
        reduce_transparency,
    }
}

/// Linux and Windows report no accent and no Reduce Transparency.
#[cfg(not(target_os = "macos"))]
pub fn read() -> SystemAppearance {
    SystemAppearance::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_color_rounds_each_component() {
        assert_eq!(hex_color(0.0, 0.478, 1.0), "#007aff");
        assert_eq!(hex_color(1.0, 1.0, 1.0), "#ffffff");
        assert_eq!(hex_color(0.0, 0.0, 0.0), "#000000");
    }

    #[test]
    fn hex_color_clamps_out_of_range_components() {
        assert_eq!(hex_color(-0.2, 1.4, 0.5), "#00ff80");
    }

    #[test]
    fn payload_uses_null_for_a_missing_accent() {
        let json = serde_json::to_value(SystemAppearance::default()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "accent": null, "reduce_transparency": false })
        );
    }
}
