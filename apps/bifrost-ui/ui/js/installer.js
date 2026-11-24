// Windows 11 App Installer dialog for .apk/.apkm packages.

import { escapeHtml, formatBytes, invoke, showToast, state } from "./core.js";
import { refreshApps } from "./data.js";

export function setupInstallProgressListener() {
  const listen = window.__TAURI__?.event?.listen;
  if (!listen) return;
  listen("install-progress", (event) => {
    const p = event?.payload;
    if (!p) return;
    setInstallerProgress(p.percent ?? 0, p.stage || "Installing…", Boolean(p.indeterminate));
  });
}

export function setInstallerProgress(percent, stage, indeterminate) {
  const wrap = document.getElementById("installer-progress-wrap");
  const fill = document.getElementById("installer-progress-fill");
  const stageEl = document.getElementById("installer-stage-label");
  const pctEl = document.getElementById("installer-percent-label");
  wrap.classList.remove("hidden");
  stageEl.textContent = stage;
  pctEl.textContent = indeterminate ? "Working…" : `${percent}%`;
  fill.classList.toggle("indeterminate", indeterminate);
  if (!indeterminate) {
    fill.style.width = `${Math.max(0, Math.min(100, percent))}%`;
  }
}

export function closeInstallDialog() {
  if (state.installerBusy) return;
  if (state.standaloneInstallerMode) {
    invoke?.("close_installer_window");
    return;
  }
  document.getElementById("installer-modal").classList.add("hidden");
  state.currentInstallerPreview = null;
  state.currentInstalledPackage = null;
}

export async function openInstallDialog(apkPath) {
  const modal = document.getElementById("installer-modal");
  const questionEl = document.getElementById("installer-question");
  const titleEl = document.getElementById("installer-app-title");
  const verEl = document.getElementById("installer-version-line");
  const metaEl = document.getElementById("installer-meta");
  const iconImg = document.getElementById("installer-icon-img");
  const iconFallback = document.getElementById("installer-icon-fallback");
  const progressWrap = document.getElementById("installer-progress-wrap");
  const errorEl = document.getElementById("installer-error");
  const cancelBtn = document.getElementById("btn-installer-cancel");
  const confirmBtn = document.getElementById("btn-installer-confirm");
  const launchBtn = document.getElementById("btn-installer-launch");

  state.currentInstallerPreview = null;
  state.currentInstalledPackage = null;
  state.installerBusy = false;

  questionEl.textContent = "Inspecting Android package…";
  titleEl.textContent = "Please wait";
  verEl.textContent = apkPath;
  metaEl.innerHTML = "";
  iconImg.classList.add("hidden");
  iconFallback.classList.remove("hidden");
  progressWrap.classList.add("hidden");
  errorEl.classList.add("hidden");
  cancelBtn.disabled = false;
  cancelBtn.textContent = "Cancel";
  confirmBtn.disabled = true;
  confirmBtn.classList.remove("hidden");
  confirmBtn.textContent = "Install";
  launchBtn.classList.add("hidden");
  modal.classList.remove("hidden");

  try {
    const preview = await invoke("preview_package_install", { apkPath });
    state.currentInstallerPreview = preview;

    const verSuffix = preview.new_version ? ` v${preview.new_version}` : "";
    questionEl.textContent = preview.is_update
      ? "Do you want to update this application?"
      : "Do you want to install this application?";
    titleEl.textContent = `${preview.display_name}${verSuffix}`;

    if (preview.is_update) {
      const fromVer = preview.installed_version ? `v${preview.installed_version}` : "installed";
      const toVer = preview.new_version ? `v${preview.new_version}` : "new build";
      verEl.textContent = `Update: ${fromVer} → ${toVer} (${preview.package_name || "unknown package"})`;
    } else {
      verEl.textContent = preview.package_name || preview.path;
    }

    if (preview.icon_data_url) {
      iconImg.src = preview.icon_data_url;
      iconImg.classList.remove("hidden");
      iconFallback.classList.add("hidden");
    }

    const abiLabel = !preview.abi_supported
      ? `Unsupported ABI (${preview.detected_abis.join(", ")})`
      : preview.detected_abis.length
        ? "x86_64 Native"
        : "Pure ART";
    const chips = [
      `<span class="meta-chip ${preview.abi_supported ? "ok" : "err"}">${escapeHtml(abiLabel)}</span>`,
      `<span class="meta-chip">${escapeHtml(formatBytes(preview.size_bytes))}</span>`,
      `<span class="meta-chip">${preview.is_split_bundle ? `Split Bundle (${preview.split_apk_count} APKs)` : "Single APK"}</span>`,
    ];
    metaEl.innerHTML = chips.join("");

    if (!preview.abi_supported) {
      errorEl.textContent =
        "This package only contains non-x86_64 native libraries (ARM64 translation is unsupported per ADR-0004).";
      errorEl.classList.remove("hidden");
      confirmBtn.disabled = true;
    } else {
      confirmBtn.textContent = preview.is_update ? "Update" : "Install";
      confirmBtn.disabled = false;
    }
  } catch (err) {
    questionEl.textContent = "Unable to inspect package";
    titleEl.textContent = "Invalid package";
    errorEl.textContent = String(err);
    errorEl.classList.remove("hidden");
    confirmBtn.disabled = true;
  }
}

export async function runConfirmedInstall() {
  if (!state.currentInstallerPreview || state.installerBusy) return;
  state.installerBusy = true;
  const cancelBtn = document.getElementById("btn-installer-cancel");
  const confirmBtn = document.getElementById("btn-installer-confirm");
  const launchBtn = document.getElementById("btn-installer-launch");
  const errorEl = document.getElementById("installer-error");

  cancelBtn.disabled = true;
  confirmBtn.disabled = true;
  errorEl.classList.add("hidden");
  setInstallerProgress(8, "Starting installation…", false);

  try {
    const res = await invoke("install_apk_file", { apkPath: state.currentInstallerPreview.path });
    state.currentInstalledPackage = res.package_name || state.currentInstallerPreview.package_name;
    setInstallerProgress(
      100,
      state.currentInstallerPreview.is_update
        ? `${state.currentInstallerPreview.display_name} updated successfully!`
        : `${state.currentInstallerPreview.display_name} installed successfully!`,
      false
    );
    confirmBtn.classList.add("hidden");
    cancelBtn.disabled = false;
    cancelBtn.textContent = "Close";
    if (state.currentInstalledPackage) {
      launchBtn.classList.remove("hidden");
    }
    if (!state.standaloneInstallerMode) {
      showToast(
        `${state.currentInstallerPreview.is_update ? "Updated" : "Installed"} ${state.currentInstallerPreview.display_name} on ${res.serial}`
      );
      await refreshApps();
    }
  } catch (err) {
    document.getElementById("installer-progress-wrap").classList.add("hidden");
    errorEl.textContent = String(err);
    errorEl.classList.remove("hidden");
    cancelBtn.disabled = false;
    confirmBtn.disabled = false;
  } finally {
    state.installerBusy = false;
  }
}
