//! `bifrost apps`, `install` and `uninstall` (Valhalla package management).

use super::*;

pub(super) async fn handle_apps(
    action: &AppsAction,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    let session = Session::open(explicit_config.map(PathBuf::as_path)).await?;
    let paths = session.paths().clone();

    match action {
        AppsAction::List {
            third_party,
            serial,
        } => {
            let packages = session.list_apps(serial.clone(), *third_party).await?;

            if as_json {
                return serde_json::to_string_pretty(&packages)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            if packages.is_empty() {
                return Ok("No installed Android packages found.".to_string());
            }
            let mut out = format!("Installed Android Packages ({}):\n", packages.len());
            for pkg in &packages {
                let kind = if pkg.is_system { "system" } else { "user" };
                out.push_str(&format!("  - {:<40} [{}]\n", pkg.package_name, kind));
            }
            Ok(out.trim_end().to_string())
        }
        AppsAction::Info { package, serial } => {
            valhalla::validate_package_name(package)?;
            let details = session.app_info(serial.clone(), package.clone()).await?;

            if as_json {
                return serde_json::to_string_pretty(&details)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Package:          {}\nVersion Name:     {}\nVersion Code:     {}\nPrimary CPU ABI:  {}\nLauncher:         {}\nCode Path:        {}",
                details.package_name,
                details.version_name.as_deref().unwrap_or("unknown"),
                details.version_code.as_deref().unwrap_or("unknown"),
                details.primary_cpu_abi.as_deref().unwrap_or("pure ART / none"),
                details.launcher_component.as_deref().unwrap_or("none"),
                details.code_path.as_deref().unwrap_or("unknown")
            ))
        }
        AppsAction::Launch {
            package,
            serial,
            window,
            width,
            height,
            dpi,
            no_autostart,
        } => {
            valhalla::validate_package_name(package)?;
            if *window {
                let target = session
                    .ensure_device_ready(serial.clone(), !*no_autostart, || {})
                    .await?;
                let ctrl = DisplayController::new();
                let handle = ctrl.launch_in_independent_window(
                    &target,
                    package,
                    AppWindowSpec {
                        width: *width,
                        height: *height,
                        dpi: *dpi,
                    },
                )?;
                if as_json {
                    return serde_json::to_string_pretty(&handle)
                        .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
                }
                return Ok(format!(
                    "Launched '{}' in independent Windows 11 window '{}' (PID {}, {}x{}@{}dpi).",
                    handle.package_name,
                    handle.window_title,
                    handle.pid,
                    handle.spec.width,
                    handle.spec.height,
                    handle.spec.dpi
                ));
            }

            let outcome = session.launch_app(serial.clone(), package.clone()).await?;

            if as_json {
                return serde_json::to_string_pretty(&outcome)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Launched '{}' on '{}' (component: {}).",
                outcome.package_name,
                outcome.serial,
                outcome.component.as_deref().unwrap_or("launcher intent")
            ))
        }
        AppsAction::Stop { package, serial } => {
            valhalla::validate_package_name(package)?;
            session.stop_app(serial.clone(), package.clone()).await?;

            if as_json {
                let payload = serde_json::json!({ "package": package, "stopped": true });
                return serde_json::to_string_pretty(&payload)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!("Force-stopped Android package '{package}'."))
        }
        AppsAction::Shortcut {
            package,
            name,
            apk,
            remove,
        } => {
            let mgr = valhalla::ShortcutManager::new(paths.data_dir.clone());
            if *remove {
                let removed = mgr.remove_for_app(package)?;
                if as_json {
                    return serde_json::to_string_pretty(&serde_json::json!({
                        "package": package,
                        "removed": removed.is_some(),
                    }))
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
                }
                return Ok(if removed.is_some() {
                    format!("Removed Windows Desktop and Start Menu shortcuts for '{package}'.")
                } else {
                    format!("No recorded Windows shortcuts found for '{package}'.")
                });
            }

            let record = mgr.create_for_app(package, name.as_deref(), apk.as_deref())?;
            if as_json {
                return serde_json::to_string_pretty(&record)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Created Windows shortcuts for '{}' ({}):\n  Desktop:    {}\n  Start Menu: {}",
                record.display_name,
                record.package_name,
                record.desktop_shortcut.display(),
                record.start_menu_shortcut.display()
            ))
        }
    }
}

pub(super) async fn handle_install(
    apk: &PathBuf,
    serial: Option<&str>,
    inspect_only: bool,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    // Reject unsafe paths (traversal / wrong extension) locally before touching IPC or ADB.
    let safe_apk = valhalla::validate_package_file_path(apk)?;

    if inspect_only {
        let info = inspect_apk(&safe_apk)?;
        if as_json {
            return serde_json::to_string_pretty(&info)
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
        }
        let abi_summary = match &info.abi_compatibility {
            AbiCompatibility::PureArt => "SUPPORTED (Pure ART / DEX — no native .so libraries)".to_string(),
            AbiCompatibility::NativeX86_64 { abis } => {
                format!("SUPPORTED (x86_64 native libs present; all ABIs: {})", abis.join(", "))
            }
            AbiCompatibility::UnsupportedNativeAbi { abis } => format!(
                "UNSUPPORTED (only [{}] native libs; ARM64 translation is unsupported per ADR-0004)",
                abis.join(", ")
            ),
        };
        let bundle_line = if info.is_split_bundle {
            format!(
                "\nSplit Bundle:      yes ({} split APKs)",
                info.split_apk_count
            )
        } else {
            String::new()
        };
        return Ok(format!(
            "Package Path:      {}\nPackage Name:      {}{}\nSize:              {} bytes\nSHA-256:           {}\nZIP Entries:       {}\nAndroidManifest:   {}\nDEX Bytecode:      {}\nABI Compatibility: {}",
            info.path.display(),
            info.package_name.as_deref().unwrap_or("unknown"),
            bundle_line,
            info.size_bytes,
            info.sha256,
            info.entry_count,
            info.has_android_manifest,
            info.has_dex,
            abi_summary
        ));
    }

    let session = Session::open(explicit_config.map(PathBuf::as_path)).await?;

    // Ensure Midgard is booted headlessly if installing via double-click while stopped.
    if serial.is_none()
        && blocking(|| ValhallaManager::new().resolve_target_serial(None))
            .await
            .is_err()
    {
        let inspection = inspect_apk(&safe_apk)?;
        if !inspection.abi_compatibility.is_supported() {
            return Err(BifrostError::UnsupportedAbi {
                path: inspection.path,
                detected_abis: inspection.detected_abis.join(", "),
            });
        }
        session.ensure_device_ready(None, true, || {}).await?;
    }

    let outcome = session
        .install_apk(serial.map(ToOwned::to_owned), safe_apk.clone())
        .await?;

    if as_json {
        return serde_json::to_string_pretty(&outcome)
            .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
    }
    let mut msg = format!(
        "Installed '{}' onto '{}' (SHA-256: {}).",
        outcome.inspection.path.display(),
        outcome.serial,
        outcome.inspection.sha256
    );
    if let Some(ref sc) = outcome.shortcuts {
        msg.push_str(&format!(
            "\nDesktop Shortcut:    {}\nStart Menu Shortcut: {}",
            sc.desktop_shortcut.display(),
            sc.start_menu_shortcut.display()
        ));
    }
    Ok(msg)
}

pub(super) async fn handle_uninstall(
    package: &str,
    serial: Option<&str>,
    keep_data: bool,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    // Validate Android package identifier locally before touching IPC or ADB.
    valhalla::validate_package_name(package)?;
    let session = Session::open(explicit_config.map(PathBuf::as_path)).await?;
    let output = session
        .uninstall_app(
            serial.map(ToOwned::to_owned),
            package.to_string(),
            keep_data,
        )
        .await?;

    if as_json {
        let payload = serde_json::json!({
            "package": package,
            "keep_data": keep_data,
            "output": output,
        });
        return serde_json::to_string_pretty(&payload)
            .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
    }
    Ok(format!("Uninstalled '{package}' ({output})."))
}
