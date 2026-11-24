//! Windows desktop integration and bridge commands: `desktop`, `share`, `clipboard`,
//! `notifications`, `processes` and `network`.

use super::*;

pub(super) fn handle_desktop(action: &DesktopAction, as_json: bool) -> Result<String> {
    match action {
        DesktopAction::RegisterApkHandler { context_menu_only } => {
            let status = valhalla::register_apk_handler(!*context_menu_only)?;
            if as_json {
                return serde_json::to_string_pretty(&status)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Registered Windows .apk/.apkm handler (ProgID: {}, context_menu: {}, default_app: {}).\nCommand: {}",
                status.prog_id,
                status.context_menu_registered,
                status.default_handler_registered,
                status.command
            ))
        }
        DesktopAction::UnregisterApkHandler => {
            valhalla::unregister_apk_handler()?;
            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "unregistered": true,
                    "prog_id": valhalla::BIFROST_APK_PROG_ID,
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(
                "Removed BIFRÖST .apk/.apkm file association and Explorer context menu from HKCU."
                    .to_string(),
            )
        }
    }
}

pub(super) fn handle_share(
    action: &ShareAction,
    explicit_config: Option<&PathBuf>,
    as_json: bool,
) -> Result<String> {
    let (_, _, _, paths) = load_effective_config_and_paths(explicit_config)?;
    let share_root = paths.data_dir.join("shared");
    let adb = AdbClient::new();
    let bridge = SharedDirectoryBridge::new(&share_root, adb);

    match action {
        ShareAction::List => {
            let files = bridge.list_host_files()?;
            if as_json {
                let payload = serde_json::json!({
                    "host_root": share_root,
                    "entries": files,
                });
                return serde_json::to_string_pretty(&payload)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            if files.is_empty() {
                return Ok(format!(
                    "Host shared directory '{}' is empty.",
                    share_root.display()
                ));
            }
            let mut out = format!("Shared Files ({}):\n", share_root.display());
            for entry in &files {
                let kind = if entry.is_dir { "dir" } else { "file" };
                out.push_str(&format!(
                    "  - {:<36} [{}] {} bytes\n",
                    entry.relative_path, kind, entry.size_bytes
                ));
            }
            Ok(out.trim_end().to_string())
        }
        ShareAction::Push { path, serial } => {
            let target = ValhallaManager::new().resolve_target_serial(serial.as_deref())?;
            let out = bridge.push_to_guest(&target, path)?;
            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "pushed": path,
                    "serial": target,
                    "output": out.trim(),
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Pushed '{path}' to '{target}' (/sdcard/BifrostShare)."
            ))
        }
        ShareAction::Pull { path, serial } => {
            let target = ValhallaManager::new().resolve_target_serial(serial.as_deref())?;
            let dest = bridge.pull_from_guest(&target, path)?;
            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "pulled": path,
                    "serial": target,
                    "host_path": dest,
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Pulled '{path}' from '{target}' into '{}'.",
                dest.display()
            ))
        }
    }
}

pub(super) fn handle_clipboard(action: &ClipboardAction, as_json: bool) -> Result<String> {
    match action {
        ClipboardAction::Get => {
            let text = read_host_clipboard_text()?;
            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "has_text": text.is_some(),
                    "length": text.as_ref().map_or(0, String::len),
                    "text": text,
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            match text {
                Some(t) => Ok(t),
                None => Ok("(Windows clipboard does not currently contain text)".to_string()),
            }
        }
        ClipboardAction::SyncToAndroid { serial } => {
            let target = ValhallaManager::new().resolve_target_serial(serial.as_deref())?;
            let bytes = sync_clipboard_to_android(&AdbClient::new(), &target)?;
            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "synced": true,
                    "serial": target,
                    "bytes": bytes,
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Synced {bytes} bytes of clipboard text to '{target}'."
            ))
        }
    }
}

pub(super) fn handle_notifications(action: &NotificationsAction, as_json: bool) -> Result<String> {
    match action {
        NotificationsAction::List { serial } => {
            let target = ValhallaManager::new().resolve_target_serial(serial.as_deref())?;
            let list = list_active_notifications(&AdbClient::new(), &target)?;
            if as_json {
                return serde_json::to_string_pretty(&list)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            if list.is_empty() {
                return Ok(format!("No active notifications on '{target}'."));
            }
            let mut out = format!(
                "Active Android Notifications on '{target}' ({}):\n",
                list.len()
            );
            for n in &list {
                let title = n.title.as_deref().unwrap_or("(no title)");
                let text = n.text.as_deref().unwrap_or("");
                out.push_str(&format!("  - [{}] {}: {}\n", n.package_name, title, text));
            }
            Ok(out.trim_end().to_string())
        }
    }
}

pub(super) fn handle_processes(action: &ProcessesAction, as_json: bool) -> Result<String> {
    let mgr = ProcessManager::new();
    match action {
        ProcessesAction::List { filter, serial } => {
            let target = mgr.resolve_serial(serial.as_deref())?;
            let list = mgr.list_processes(&target, filter.as_deref())?;
            if as_json {
                return serde_json::to_string_pretty(&list)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            if list.is_empty() {
                return Ok(format!("No matching processes on '{target}'."));
            }
            let mut out = format!("Android Processes on '{target}' ({}):\n", list.len());
            for p in list.iter().take(50) {
                out.push_str(&format!(
                    "  PID {:<7} {:<12} {:>8} KB  {}\n",
                    p.pid, p.user, p.rss_kb, p.name
                ));
            }
            Ok(out.trim_end().to_string())
        }
    }
}

pub(super) fn handle_network(action: &NetworkAction, as_json: bool) -> Result<String> {
    let mgr = NetworkManager::new();
    match action {
        NetworkAction::Forwards { serial } => {
            let rules = mgr.list_forwards(serial.as_deref())?;
            if as_json {
                return serde_json::to_string_pretty(&rules)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            if rules.is_empty() {
                return Ok("No active ADB loopback TCP port forwards.".to_string());
            }
            let mut out = format!("Active Loopback TCP Port Forwards ({}):\n", rules.len());
            for r in &rules {
                out.push_str(&format!(
                    "  - {} : {} -> {}\n",
                    r.serial, r.local_spec, r.remote_spec
                ));
            }
            Ok(out.trim_end().to_string())
        }
        NetworkAction::Forward {
            local_port,
            guest_port,
            serial,
        } => {
            let target = mgr.resolve_serial(serial.as_deref())?;
            let rule = mgr.add_forward(&target, *local_port, *guest_port)?;
            if as_json {
                return serde_json::to_string_pretty(&rule)
                    .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Forwarded 127.0.0.1:{} -> {}:{} on '{}'.",
                local_port, rule.remote_spec, guest_port, rule.serial
            ))
        }
        NetworkAction::Unforward { local_port, serial } => {
            let target = mgr.resolve_serial(serial.as_deref())?;
            mgr.remove_forward(&target, *local_port)?;
            if as_json {
                return serde_json::to_string_pretty(&serde_json::json!({
                    "removed": true,
                    "serial": target,
                    "local_port": local_port,
                }))
                .map_err(|err| BifrostError::ConfigSerialize(err.to_string()));
            }
            Ok(format!(
                "Removed loopback forward on port {local_port} for '{target}'."
            ))
        }
    }
}
