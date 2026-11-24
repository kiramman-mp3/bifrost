// Button and form event wiring for all views.

import { formatGib, invoke, showToast, state } from "./core.js";
import { refreshAll, refreshApps, refreshImages, refreshLogs, refreshProcesses, refreshSharedFiles } from "./data.js";
import { closeInstallDialog, openInstallDialog, runConfirmedInstall } from "./installer.js";
import { renderDoctor } from "./views.js";

export function wireEvents() {
  document.getElementById("btn-refresh-all").addEventListener("click", refreshAll);
  document.getElementById("btn-heimdall-toggle").addEventListener("click", async () => {
    try {
      if (state.lastDashboard?.heimdall_connected) {
        await invoke("stop_heimdall");
        showToast("Heimdall daemon stopped.");
      } else {
        const health = await invoke("start_heimdall");
        showToast(`Heimdall daemon started (PID ${health.daemon_pid}).`);
      }
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });
  document.getElementById("btn-heimdall-autostart").addEventListener("click", async () => {
    try {
      const next = !Boolean(state.lastDashboard?.heimdall_autostart);
      await invoke("set_heimdall_autostart", { enabled: next });
      showToast(
        next
          ? "Enabled silent Heimdall login autostart (HKCU\\...\\Run)."
          : "Disabled Heimdall login autostart."
      );
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });
  document.getElementById("btn-refresh-procs").addEventListener("click", refreshProcesses);
  document.getElementById("btn-refresh-logs").addEventListener("click", refreshLogs);
  document.getElementById("btn-refresh-share").addEventListener("click", refreshSharedFiles);
  document.getElementById("btn-refresh-apps").addEventListener("click", refreshApps);
  document.getElementById("btn-refresh-images").addEventListener("click", refreshImages);

  document.getElementById("btn-browse-manifest").addEventListener("click", async () => {
    try {
      const picked = await invoke("pick_file_dialog", {
        title: "Select Midgard Bundle Manifest (manifest.json)",
        filter: "Midgard Manifest (*.json)|*.json|All files (*.*)|*.*",
      });
      if (picked) document.getElementById("manifest-input").value = picked;
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-browse-apk").addEventListener("click", async () => {
    try {
      const picked = await invoke("pick_file_dialog", {
        title: "Select Android Package (.apk / .apkm)",
        filter: "Android Package (*.apk;*.apkm)|*.apk;*.apkm|All files (*.*)|*.*",
      });
      if (picked) document.getElementById("apk-path-input").value = picked;
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-browse-img-manifest").addEventListener("click", async () => {
    try {
      const picked = await invoke("pick_file_dialog", {
        title: "Select Midgard Image Manifest (manifest.json)",
        filter: "Midgard Manifest (*.json)|*.json|All files (*.*)|*.*",
      });
      if (picked) document.getElementById("img-manifest-input").value = picked;
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-browse-recovery-dir").addEventListener("click", async () => {
    try {
      const picked = await invoke("pick_folder_dialog", {
        title: "Select Folder for Userdata Backup / Restore",
      });
      if (picked) document.getElementById("recovery-dir-input").value = picked;
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-register-image").addEventListener("click", async () => {
    const manifestPath = document.getElementById("img-manifest-input").value.trim();
    if (!manifestPath) {
      showToast("Browse for or enter a Midgard manifest.json path first.", true);
      return;
    }
    try {
      const allowUnsigned = document.getElementById("img-allow-unsigned").checked;
      const rec = await invoke("register_image", { manifestPath, allowUnsigned });
      const trust = rec.signature_verified ? "trusted signature" : "UNSIGNED, sha256-only";
      showToast(`Registered image '${rec.id}' (Android ${rec.android_version}, ${trust}).`);
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-backup-userdata").addEventListener("click", async () => {
    const outputDir = document.getElementById("recovery-dir-input").value.trim();
    if (!outputDir) {
      showToast("Choose a backup destination folder first.", true);
      return;
    }
    try {
      const meta = await invoke("backup_userdata", { outputDir });
      showToast(`Backed up userdata (${formatGib(meta.size_bytes)}, SHA-256 ${meta.sha256.slice(0, 12)}…).`);
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-restore-userdata").addEventListener("click", async () => {
    const fromDir = document.getElementById("recovery-dir-input").value.trim();
    if (!fromDir) {
      showToast("Choose a verified backup folder to restore from first.", true);
      return;
    }
    if (
      !window.confirm(
        `Restore Android userdata.img from '${fromDir}'? This overwrites your current Android user data after verifying SHA-256.`
      )
    ) {
      return;
    }
    try {
      const meta = await invoke("restore_userdata", { fromDir });
      showToast(`Restored verified userdata (SHA-256 ${meta.sha256.slice(0, 12)}…).`);
    } catch (err) {
      showToast(String(err), true);
    }
  });
  document.getElementById("btn-close-details").addEventListener("click", () => {
    document.getElementById("app-details-card").classList.add("hidden");
  });

  document.getElementById("btn-runtime-start").addEventListener("click", async () => {
    const manifestPath = document.getElementById("manifest-input").value.trim();
    const headless = document.getElementById("start-headless").checked;
    if (!manifestPath) {
      showToast("Provide a verified Midgard manifest.json path first.", true);
      return;
    }
    try {
      await invoke("start_runtime", { manifestPath, headless });
      showToast("Midgard VM start requested.");
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-runtime-restart").addEventListener("click", async () => {
    const manifestPath = document.getElementById("manifest-input").value.trim() || null;
    const headless = document.getElementById("start-headless").checked;
    try {
      await invoke("restart_runtime", { manifestPath, headless });
      showToast("Midgard VM restarted.");
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-runtime-stop").addEventListener("click", async () => {
    try {
      await invoke("stop_runtime", { force: false });
      showToast("Midgard VM stopped cleanly.");
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-inspect-apk").addEventListener("click", async () => {
    const apkPath = document.getElementById("apk-path-input").value.trim();
    if (!apkPath) {
      showToast("Enter a local .apk path first.", true);
      return;
    }
    try {
      const info = await invoke("inspect_apk_file", { apkPath });
      const box = document.getElementById("apk-inspect-output");
      box.textContent = JSON.stringify(info, null, 2);
      box.classList.remove("hidden");
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-installer-cancel").addEventListener("click", closeInstallDialog);
  document.getElementById("btn-installer-confirm").addEventListener("click", runConfirmedInstall);
  document.getElementById("btn-installer-launch").addEventListener("click", async () => {
    if (!state.currentInstalledPackage) return;
    const pkg = state.currentInstalledPackage;
    try {
      await invoke("launch_app_window", { package: pkg, width: null, height: null, dpi: null });
      closeInstallDialog();
    } catch (err) {
      const errEl = document.getElementById("installer-error");
      errEl.textContent = String(err);
      errEl.classList.remove("hidden");
    }
  });

  document.getElementById("btn-install-apk").addEventListener("click", async () => {
    const apkPath = document.getElementById("apk-path-input").value.trim();
    if (!apkPath) {
      showToast("Enter a local .apk or .apkm path first.", true);
      return;
    }
    await openInstallDialog(apkPath);
  });

  document.getElementById("btn-autotune-config").addEventListener("click", async () => {
    try {
      const tuned = await invoke("auto_tune_config");
      showToast(
        `Auto-tuned for this PC: ${tuned.runtime.cpu_cores} vCPUs, ${tuned.runtime.memory_mb} MB RAM. Restart Midgard to apply.`
      );
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("config-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    if (!state.lastDashboard) return;
    const updated = {
      schema_version: state.lastDashboard.config.schema_version,
      runtime: {
        cpu_cores: Number(document.getElementById("cfg-cpu").value),
        memory_mb: Number(document.getElementById("cfg-ram").value),
        storage_gb: Number(document.getElementById("cfg-storage").value),
        graphics_mode: document.getElementById("cfg-graphics").value,
        audio_enabled: document.getElementById("cfg-audio-enabled").checked,
        data_dir: state.lastDashboard.config.runtime.data_dir,
      },
      display: {
        width: Number(document.getElementById("cfg-width").value),
        height: Number(document.getElementById("cfg-height").value),
        dpi: Number(document.getElementById("cfg-dpi").value),
      },
      desktop: {
        auto_sync_shortcuts: document.getElementById("cfg-auto-sync-shortcuts").checked,
        forward_notifications: document.getElementById("cfg-forward-notifications").checked,
        auto_sync_clipboard: document.getElementById("cfg-auto-sync-clipboard").checked,
      },
      network: {
        adb_bind_address: document.getElementById("cfg-adb-host").value.trim(),
        adb_port: Number(document.getElementById("cfg-adb-port").value),
        allow_non_loopback_adb: state.lastDashboard.config.network.allow_non_loopback_adb,
      },
      logging: {
        level: document.getElementById("cfg-log-level").value,
        json: state.lastDashboard.config.logging.json,
      },
    };
    try {
      const savedPath = await invoke("save_config", { config: updated });
      showToast(`Configuration validated and saved to ${savedPath}`);
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-run-doctor").addEventListener("click", async () => {
    try {
      const report = await invoke("get_doctor_report");
      state.lastDoctor = report;
      renderDoctor(report);
      showToast(`Doctor complete: ${report.pass_count} passed, ${report.warn_count} warnings, ${report.fail_count} failed.`);
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-export-diag").addEventListener("click", async () => {
    if (!state.lastDoctor) return;
    try {
      await navigator.clipboard.writeText(JSON.stringify(state.lastDoctor, null, 2));
      showToast("Diagnostic JSON report copied to clipboard.");
    } catch (err) {
      showToast(`Failed to copy report: ${err}`, true);
    }
  });
}
