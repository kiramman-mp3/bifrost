// Event wiring for Home, Apps, Settings, Installer, and Developer views.

import { formatGib, invoke, showToast, state } from "./core.js";
import {
  refreshAll,
  refreshApps,
  refreshImages,
  refreshLogs,
  refreshProcesses,
  refreshTrustedKeys,
  saveCurrentConfig,
} from "./data.js";
import { t } from "./i18n.js";
import { closeInstallDialog, openInstallDialog, runConfirmedInstall } from "./installer.js";
import { openSetupManually } from "./setup.js";
import { activateSection, renderAppsList, renderDoctor } from "./views.js";

async function pickAndInstallApk() {
  try {
    const picked = await invoke("pick_file_dialog", {
      title: "Android Package (.apk / .apkm)",
      filter: "Android Package (*.apk;*.apkm)|*.apk;*.apkm|All files (*.*)|*.*",
    });
    if (picked) {
      await openInstallDialog(picked);
    }
  } catch (err) {
    showToast(String(err), true);
  }
}

export function wireEvents() {
  // 1. Home Hero & Quick Tiles
  document.getElementById("btn-refresh-all")?.addEventListener("click", refreshAll);

  document.getElementById("btn-hero-power")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    const rt = state.lastDashboard?.runtime_state || "stopped";
    const isActive = rt === "running" || rt === "starting" || rt === "booting";
    btn.disabled = true;
    try {
      if (isActive) {
        await invoke("stop_runtime", { force: false });
      } else {
        await invoke("start_active_runtime", { headless: true });
      }
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    } finally {
      btn.disabled = false;
    }
  });

  document.getElementById("btn-hero-restart")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    btn.disabled = true;
    try {
      await invoke("restart_runtime", { manifestPath: null, headless: true });
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    } finally {
      btn.disabled = false;
    }
  });

  document.getElementById("tile-install-apk")?.addEventListener("click", pickAndInstallApk);
  document.getElementById("tile-goto-apps")?.addEventListener("click", () => {
    activateSection("apps");
    refreshApps();
  });
  document.getElementById("tile-open-shared")?.addEventListener("click", async () => {
    try {
      await invoke("open_known_folder", { kind: "shared" });
    } catch (err) {
      showToast(String(err), true);
    }
  });
  document.getElementById("btn-home-goto-resources")?.addEventListener("click", () => {
    activateSection("settings");
    document.getElementById("settings-resources-anchor")?.scrollIntoView({ behavior: "smooth" });
  });

  // 2. Applications
  document.getElementById("btn-pick-install-apk")?.addEventListener("click", pickAndInstallApk);
  document.getElementById("btn-refresh-apps")?.addEventListener("click", refreshApps);
  document.getElementById("apps-search-input")?.addEventListener("input", renderAppsList);
  document.getElementById("apps-third-party")?.addEventListener("change", refreshApps);

  document.getElementById("btn-inspect-apk-pick")?.addEventListener("click", async () => {
    try {
      const apkPath = await invoke("pick_file_dialog", {
        title: "Inspect Android Package (.apk / .apkm)",
        filter: "Android Package (*.apk;*.apkm)|*.apk;*.apkm|All files (*.*)|*.*",
      });
      if (!apkPath) return;
      const info = await invoke("inspect_apk_file", { apkPath });
      document.getElementById("apk-inspect-output").textContent = JSON.stringify(info, null, 2);
      document.getElementById("apk-inspect-card").classList.remove("hidden");
    } catch (err) {
      showToast(String(err), true);
    }
  });
  document.getElementById("btn-close-inspect")?.addEventListener("click", () => {
    document.getElementById("apk-inspect-card").classList.add("hidden");
  });
  document.getElementById("btn-close-details")?.addEventListener("click", () => {
    document.getElementById("app-details-card").classList.add("hidden");
  });

  // 3. Installer modal
  document.getElementById("btn-installer-cancel")?.addEventListener("click", closeInstallDialog);
  document.getElementById("btn-installer-confirm")?.addEventListener("click", runConfirmedInstall);
  document.getElementById("btn-installer-launch")?.addEventListener("click", async () => {
    if (!state.currentInstalledPackage) return;
    try {
      await invoke("launch_app_window", {
        package: state.currentInstalledPackage,
        width: null,
        height: null,
        dpi: null,
      });
      closeInstallDialog();
    } catch (err) {
      const errEl = document.getElementById("installer-error");
      errEl.textContent = String(err);
      errEl.classList.remove("hidden");
    }
  });

  // 4. Settings (auto-saving switches, performance, backup/restore)
  document.querySelectorAll(".auto-save-cfg").forEach((ctrl) => {
    ctrl.addEventListener("change", saveCurrentConfig);
  });

  document.getElementById("toggle-heimdall-autostart")?.addEventListener("change", async (e) => {
    const enabled = e.target.checked;
    try {
      await invoke("set_heimdall_autostart", { enabled });
      await refreshAll();
    } catch (err) {
      e.target.checked = !enabled;
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-autotune-config")?.addEventListener("click", async () => {
    try {
      const tuned = await invoke("auto_tune_config");
      showToast(`${tuned.runtime.cpu_cores} cores · ${tuned.runtime.memory_mb} MB RAM`);
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-save-resources")?.addEventListener("click", saveCurrentConfig);
  document.getElementById("btn-save-display-adv")?.addEventListener("click", saveCurrentConfig);
  document.getElementById("btn-save-dev-config")?.addEventListener("click", saveCurrentConfig);

  document.getElementById("btn-graphics-advice")?.addEventListener("click", async (event) => {
    const button = event.currentTarget;
    const text = document.getElementById("graphics-advice-text");
    button.disabled = true;
    try {
      const advice = await invoke("graphics_advice");
      text.textContent = advice.summary;
    } catch (err) {
      text.textContent = `${err}`;
    } finally {
      button.disabled = false;
    }
  });

  document.getElementById("btn-open-shared-folder")?.addEventListener("click", async () => {
    try {
      await invoke("open_known_folder", { kind: "shared" });
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-backup-userdata")?.addEventListener("click", async () => {
    try {
      const outputDir = await invoke("pick_folder_dialog", {
        title: "Select Backup Destination Folder",
      });
      if (!outputDir) return;
      const meta = await invoke("backup_userdata", { outputDir });
      showToast(`${formatGib(meta.size_bytes)} · SHA-256 ${meta.sha256.slice(0, 12)}…`);
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-restore-userdata")?.addEventListener("click", async () => {
    try {
      const fromDir = await invoke("pick_folder_dialog", {
        title: "Select Backup Folder to Restore",
      });
      if (!fromDir) return;
      if (!window.confirm(t("settings.confirmRestore", { dir: fromDir }))) return;
      const meta = await invoke("restore_userdata", { fromDir });
      showToast(`SHA-256 ${meta.sha256.slice(0, 12)}…`);
    } catch (err) {
      showToast(String(err), true);
    }
  });

  // 5. Developer tab controls
  document.getElementById("btn-open-setup")?.addEventListener("click", openSetupManually);

  document.getElementById("btn-heimdall-toggle")?.addEventListener("click", async () => {
    try {
      if (state.lastDashboard?.heimdall_connected) {
        await invoke("stop_heimdall");
      } else {
        await invoke("start_heimdall");
      }
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-browse-manifest")?.addEventListener("click", async () => {
    try {
      const picked = await invoke("pick_file_dialog", {
        title: "Select Midgard Manifest (manifest.json)",
        filter: "Midgard Manifest (*.json)|*.json|All files (*.*)|*.*",
      });
      if (picked) document.getElementById("manifest-input").value = picked;
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-runtime-start")?.addEventListener("click", async () => {
    const manifestPath = document.getElementById("manifest-input").value.trim();
    const headless = document.getElementById("start-headless").checked;
    try {
      if (manifestPath) {
        await invoke("start_runtime", { manifestPath, headless });
      } else {
        await invoke("start_active_runtime", { headless });
      }
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-runtime-restart")?.addEventListener("click", async () => {
    const manifestPath = document.getElementById("manifest-input").value.trim() || null;
    const headless = document.getElementById("start-headless").checked;
    try {
      await invoke("restart_runtime", { manifestPath, headless });
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-runtime-stop")?.addEventListener("click", async () => {
    try {
      await invoke("stop_runtime", { force: false });
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-refresh-images")?.addEventListener("click", refreshImages);
  document.getElementById("btn-browse-img-manifest")?.addEventListener("click", async () => {
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

  document.getElementById("btn-register-image")?.addEventListener("click", async () => {
    const manifestPath = document.getElementById("img-manifest-input").value.trim();
    if (!manifestPath) return;
    try {
      const allowUnsigned = document.getElementById("img-allow-unsigned").checked;
      const rec = await invoke("register_image", { manifestPath, allowUnsigned });
      showToast(`${rec.id} (Android ${rec.android_version})`);
      await refreshAll();
      await refreshImages();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-add-trusted-key")?.addEventListener("click", async () => {
    const input = document.getElementById("trusted-key-input");
    const publicKey = input.value.trim();
    if (!publicKey) return;
    try {
      const keys = await invoke("add_trusted_key", { publicKey });
      input.value = "";
      await refreshTrustedKeys(keys);
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-refresh-procs")?.addEventListener("click", refreshProcesses);
  document.getElementById("btn-refresh-logs")?.addEventListener("click", refreshLogs);
  document.getElementById("btn-open-logs-folder")?.addEventListener("click", async () => {
    try {
      await invoke("open_known_folder", { kind: "logs" });
    } catch (err) {
      showToast(String(err), true);
    }
  });
  document.getElementById("btn-open-data-folder")?.addEventListener("click", async () => {
    try {
      await invoke("open_known_folder", { kind: "data" });
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-run-doctor")?.addEventListener("click", async () => {
    try {
      const report = await invoke("get_doctor_report");
      state.lastDoctor = report;
      renderDoctor(report);
      showToast(`${report.pass_count}/${report.checks.length} OK`);
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-export-diag")?.addEventListener("click", async () => {
    if (!state.lastDoctor) return;
    try {
      await navigator.clipboard.writeText(JSON.stringify(state.lastDoctor, null, 2));
      showToast("JSON copied");
    } catch (err) {
      showToast(String(err), true);
    }
  });
}
