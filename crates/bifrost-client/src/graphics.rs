//! Graphics mode advice from the OpenGL ES versions installed apps require (ADR-0007).
//!
//! On the default QEMU-WHPX backend the two practical modes trade compatibility for
//! speed: `auto` renders on the CPU (ANGLE on SwiftShader) and exposes OpenGL ES 3.1;
//! `gfxstream` renders through virgl on the host GPU, about twice as smooth on the
//! reference machine, but exposes only OpenGL ES 2.0. Apps declare the version they
//! need with `<uses-feature android:glEsVersion>`, so the choice can be made from data.

use bifrost_core::GraphicsMode;
use serde::Serialize;
use valhalla::{format_gles_version, AppGlesRequirement};

/// Highest OpenGL ES version the virgl GPU mode exposes (`major << 16 | minor`).
pub const GPU_MODE_MAX_GLES: u32 = 0x0002_0000;
/// Highest OpenGL ES version the default CPU mode (ANGLE on SwiftShader) exposes.
pub const CPU_MODE_MAX_GLES: u32 = 0x0003_0001;

/// One installed app and the OpenGL ES version it requires, formatted (`3.0`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppGlesNeed {
    pub package_name: String,
    pub required_gles: String,
}

/// Recommendation derived from the installed third-party apps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphicsAdvice {
    pub current_mode: GraphicsMode,
    pub recommended_mode: GraphicsMode,
    pub apps_checked: usize,
    /// Apps that need more than the GPU mode offers (OpenGL ES 3.0 or newer).
    pub apps_needing_cpu_mode: Vec<AppGlesNeed>,
    /// Apps that need more than either mode offers (newer than OpenGL ES 3.1).
    pub apps_unsupported: Vec<AppGlesNeed>,
    /// Apps whose manifest could not be read; they are not counted either way.
    pub apps_unreadable: Vec<String>,
    pub summary: String,
}

/// Builds the advice for `current` from the apps' declared requirements.
#[must_use]
pub fn advise(current: GraphicsMode, apps: &[AppGlesRequirement]) -> GraphicsAdvice {
    let need = |app: &AppGlesRequirement, v: u32| AppGlesNeed {
        package_name: app.package_name.clone(),
        required_gles: format_gles_version(v),
    };
    let mut apps_needing_cpu_mode = Vec::new();
    let mut apps_unsupported = Vec::new();
    let mut apps_unreadable = Vec::new();
    for app in apps {
        match app.required_gles_version {
            _ if !app.readable => apps_unreadable.push(app.package_name.clone()),
            Some(v) if v > CPU_MODE_MAX_GLES => apps_unsupported.push(need(app, v)),
            Some(v) if v > GPU_MODE_MAX_GLES => apps_needing_cpu_mode.push(need(app, v)),
            _ => {}
        }
    }

    let recommended_mode = if apps_needing_cpu_mode.is_empty() {
        GraphicsMode::Gfxstream
    } else {
        GraphicsMode::Auto
    };

    let list = |apps: &[AppGlesNeed]| {
        apps.iter()
            .map(|a| format!("{} (OpenGL ES {})", a.package_name, a.required_gles))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut summary = if apps.is_empty() {
        "No third-party apps are installed yet. GPU acceleration (Experimental) renders on the \
         host GPU but offers only OpenGL ES 2.0; check again after installing apps."
            .to_string()
    } else if apps_needing_cpu_mode.is_empty() {
        "No installed app requires more than OpenGL ES 2.0, so GPU acceleration (Experimental) \
         can render them on the host GPU. Apps that use OpenGL ES 3 only when available will \
         fall back to 2.0."
            .to_string()
    } else {
        format!(
            "Keep Auto: GPU acceleration offers only OpenGL ES 2.0, and these apps require more: {}.",
            list(&apps_needing_cpu_mode)
        )
    };
    if !apps_unsupported.is_empty() {
        summary.push_str(&format!(
            " These apps require a newer OpenGL ES than either mode offers (3.1 at most) and may \
             not start: {}.",
            list(&apps_unsupported)
        ));
    }
    if !apps_unreadable.is_empty() {
        summary.push_str(&format!(
            " Could not read the manifest of: {}.",
            apps_unreadable.join(", ")
        ));
    }

    GraphicsAdvice {
        current_mode: current,
        recommended_mode,
        apps_checked: apps.len(),
        apps_needing_cpu_mode,
        apps_unsupported,
        apps_unreadable,
        summary,
    }
}

/// Warning for installing an app that requires `required` OpenGL ES in `mode`, if any.
#[must_use]
pub fn install_warning(mode: GraphicsMode, required: Option<u32>) -> Option<String> {
    let required = required?;
    let version = format_gles_version(required);
    if required > CPU_MODE_MAX_GLES {
        Some(format!(
            "This app requires OpenGL ES {version}; BIFRÖST offers at most 3.1, so it may not start."
        ))
    } else if mode == GraphicsMode::Gfxstream && required > GPU_MODE_MAX_GLES {
        Some(format!(
            "This app requires OpenGL ES {version}, but GPU acceleration offers only 2.0. \
             Switch Graphics to Auto in Settings (or set graphics_mode = \"auto\") and restart Midgard."
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, gles: Option<u32>) -> AppGlesRequirement {
        AppGlesRequirement {
            package_name: name.to_string(),
            required_gles_version: gles,
            readable: true,
        }
    }

    #[test]
    fn recommends_the_gpu_mode_when_every_app_fits_in_es_2() {
        let advice = advise(
            GraphicsMode::Auto,
            &[
                app("org.fdroid.fdroid", None),
                app("com.example.dream", Some(0x0002_0000)),
            ],
        );
        assert_eq!(advice.recommended_mode, GraphicsMode::Gfxstream);
        assert_eq!(advice.apps_checked, 2);
        assert!(advice.apps_needing_cpu_mode.is_empty());
    }

    #[test]
    fn keeps_auto_when_an_app_requires_es_3() {
        let mut unreadable = app("com.example.broken", None);
        unreadable.readable = false;
        let advice = advise(
            GraphicsMode::Gfxstream,
            &[
                app("com.example.game", Some(0x0003_0000)),
                app("com.example.vr", Some(0x0003_0002)),
                unreadable,
            ],
        );
        assert_eq!(advice.recommended_mode, GraphicsMode::Auto);
        assert_eq!(
            advice.apps_needing_cpu_mode,
            vec![AppGlesNeed {
                package_name: "com.example.game".into(),
                required_gles: "3.0".into()
            }]
        );
        assert_eq!(advice.apps_unsupported[0].required_gles, "3.2");
        assert_eq!(
            advice.apps_unreadable,
            vec!["com.example.broken".to_string()]
        );
        assert!(advice.summary.contains("com.example.game (OpenGL ES 3.0)"));
    }

    #[test]
    fn warns_only_when_the_mode_cannot_serve_the_app() {
        assert!(install_warning(GraphicsMode::Auto, Some(0x0003_0000)).is_none());
        assert!(install_warning(GraphicsMode::Gfxstream, Some(0x0002_0000)).is_none());
        assert!(install_warning(GraphicsMode::Gfxstream, None).is_none());
        assert!(install_warning(GraphicsMode::Gfxstream, Some(0x0003_0000))
            .unwrap()
            .contains("only 2.0"));
        assert!(install_warning(GraphicsMode::Auto, Some(0x0003_0002))
            .unwrap()
            .contains("at most 3.1"));
    }
}
