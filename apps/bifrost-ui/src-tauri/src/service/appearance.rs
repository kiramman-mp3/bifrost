//! Native Windows 11 appearance and locale discovery (accent palette, theme, language).
//!
//! Reads the current user's Windows personalization settings from `HKCU` without any
//! `unsafe` code, deriving WinUI 3 light/dark accent variants and accessible foreground
//! contrast colors so `bifrost-ui` matches native Windows 11 tabs automatically.

use serde::Serialize;

/// Live Windows 11 personalization snapshot consumed by the Fluent/WinUI 3 shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WindowsAppearance {
    /// `"dark"` or `"light"` from `AppsUseLightTheme`.
    pub system_theme: String,
    /// Base Windows accent color (`#rrggbb`).
    pub accent: String,
    /// WinUI 3 `AccentLight2` — primary accent fill on Dark surfaces.
    pub accent_light: String,
    /// WinUI 3 `AccentLight3` — hover accent fill on Dark surfaces.
    pub accent_light_hover: String,
    /// WinUI 3 `AccentDark1` — primary accent fill on Light surfaces.
    pub accent_dark: String,
    /// WinUI 3 `AccentDark2` — hover accent fill on Light surfaces.
    pub accent_dark_hover: String,
    /// Accessible foreground (`"#000000"` or `"#ffffff"`) over `accent_light`.
    pub text_on_accent_light: String,
    /// Accessible foreground (`"#ffffff"` or `"#000000"`) over `accent_dark`.
    pub text_on_accent_dark: String,
    /// Raw OS locale tag (e.g. `"es-EC"`, `"en-US"`).
    pub system_locale: String,
    /// `"es"` when the OS locale is Spanish, `"en"` otherwise.
    pub preferred_language: String,
}

/// Decodes a Windows DWM `AccentColor` DWORD (`0xAABBGGRR` little-endian ABGR) into `(r, g, b)`.
#[must_use]
pub const fn decode_abgr_dword(dword: u32) -> (u8, u8, u8) {
    let r = (dword & 0xFF) as u8;
    let g = ((dword >> 8) & 0xFF) as u8;
    let b = ((dword >> 16) & 0xFF) as u8;
    (r, g, b)
}

/// Formats an RGB triple as lowercase `#rrggbb`.
#[must_use]
pub fn rgb_to_hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Blends `base` toward `target` by `amount` (`0.0..=1.0`).
#[must_use]
pub fn mix_rgb(base: (u8, u8, u8), target: (u8, u8, u8), amount: f32) -> (u8, u8, u8) {
    let t = amount.clamp(0.0, 1.0);
    let lerp = |a: u8, b: u8| -> u8 {
        (f32::from(a) + (f32::from(b) - f32::from(a)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    (
        lerp(base.0, target.0),
        lerp(base.1, target.1),
        lerp(base.2, target.2),
    )
}

/// Picks black or white text for readable contrast over an sRGB background.
#[must_use]
pub fn ideal_text_on_rgb((r, g, b): (u8, u8, u8)) -> &'static str {
    let lin = |c: u8| -> f32 {
        let s = f32::from(c) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
    if luminance > 0.34 {
        "#000000"
    } else {
        "#ffffff"
    }
}

/// Maps a BCP-47 locale tag (`"es-EC"`, `"es_ES"`, `"en-US"`) to a supported UI language.
#[must_use]
pub fn language_from_locale(locale: &str) -> &'static str {
    let normalized = locale.trim().to_ascii_lowercase();
    if normalized == "es" || normalized.starts_with("es-") || normalized.starts_with("es_") {
        "es"
    } else {
        "en"
    }
}

/// Builds a full WinUI 3 appearance snapshot from raw OS signals and an optional
/// 32-byte Windows `AccentPalette` (`8 × RGBA` shades: `Light3, Light2, Light1, Base, Dark1, Dark2, Dark3, ...`).
#[must_use]
pub fn build_appearance(
    apps_use_light_theme: bool,
    base_rgb: (u8, u8, u8),
    palette_bytes: Option<&[u8]>,
    locale: &str,
) -> WindowsAppearance {
    let (light3, light2, base, dark1, dark2) = match palette_bytes {
        Some(bytes) if bytes.len() >= 28 => (
            (bytes[0], bytes[1], bytes[2]),
            (bytes[4], bytes[5], bytes[6]),
            (bytes[12], bytes[13], bytes[14]),
            (bytes[16], bytes[17], bytes[18]),
            (bytes[20], bytes[21], bytes[22]),
        ),
        _ => (
            mix_rgb(base_rgb, (255, 255, 255), 0.56),
            mix_rgb(base_rgb, (255, 255, 255), 0.38),
            base_rgb,
            mix_rgb(base_rgb, (0, 0, 0), 0.16),
            mix_rgb(base_rgb, (0, 0, 0), 0.30),
        ),
    };

    let clean_locale = if locale.trim().is_empty() {
        "en-US".to_string()
    } else {
        locale.trim().to_string()
    };
    let preferred_language = language_from_locale(&clean_locale).to_string();

    WindowsAppearance {
        system_theme: if apps_use_light_theme {
            "light".to_string()
        } else {
            "dark".to_string()
        },
        accent: rgb_to_hex(base),
        accent_light: rgb_to_hex(light2),
        accent_light_hover: rgb_to_hex(light3),
        accent_dark: rgb_to_hex(dark1),
        accent_dark_hover: rgb_to_hex(dark2),
        text_on_accent_light: ideal_text_on_rgb(light2).to_string(),
        text_on_accent_dark: ideal_text_on_rgb(dark1).to_string(),
        system_locale: clean_locale,
        preferred_language,
    }
}

/// Queries the live Windows 11 personalization and locale keys under `HKCU`.
#[must_use]
pub fn get_windows_appearance() -> WindowsAppearance {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);

        let apps_use_light = hkcu
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
            .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
            .map(|v| v != 0)
            .unwrap_or(false);

        let base_rgb = hkcu
            .open_subkey("Software\\Microsoft\\Windows\\DWM")
            .and_then(|k| k.get_value::<u32, _>("AccentColor"))
            .map(decode_abgr_dword)
            .unwrap_or((0, 120, 212));

        let palette_bytes = hkcu
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent")
            .and_then(|k| k.get_raw_value("AccentPalette"))
            .ok()
            .map(|v| v.bytes);

        let locale = hkcu
            .open_subkey("Control Panel\\International")
            .and_then(|k| k.get_value::<String, _>("LocaleName"))
            .unwrap_or_else(|_| "en-US".to_string());

        build_appearance(apps_use_light, base_rgb, palette_bytes.as_deref(), &locale)
    }

    #[cfg(not(windows))]
    {
        let locale = std::env::var("LANG").unwrap_or_else(|_| "en-US".to_string());
        build_appearance(false, (0, 120, 212), None, &locale)
    }
}
