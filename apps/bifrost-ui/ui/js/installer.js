// Windows 11 App Installer dialog for .apk / .apkm packages (localized ES / EN).

import { escapeHtml, formatBytes, invoke, showToast, state } from "./core.js";
import { refreshApps } from "./data.js";
import { t } from "./i18n.js";

export function setupInstallProgressListener() {
  const listen = window.__TAURI__?.event?.listen;
  if (!listen) return;
  listen("install-progress", (event) => {
    const p = event?.payload;
    if (!p) return;
    setInstallerProgress(p.percent ?? 0, p.stage || t("common.working"), Boolean(p.indeterminate));
  });
}

export function setInstallerProgress(percent, stage, indeterminate) {
  const wrap = document.getElementById("installer-progress-wrap");
  const fill = document.getElementById("installer-progress-fill");
  const stageEl = document.getElementById("installer-stage-label");
  const pctEl = document.getElementById("installer-percent-label");
  wrap.classList.remove("hidden");
  stageEl.textContent = stage;
  pctEl.textContent = indeterminate ? t("common.working") : `${percent}%`;
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
  const warningEl = document.getElementById("installer-warning");
  const errorEl = document.getElementById("installer-error");
  const cancelBtn = document.getElementById("btn-installer-cancel");
  const confirmBtn = document.getElementById("btn-installer-confirm");
  const launchBtn = document.getElementById("btn-installer-launch");

  state.currentInstallerPreview = null;
  state.currentInstalledPackage = null;
  state.installerBusy = false;

  questionEl.textContent = t("installer.inspecting");
  titleEl.textContent = t("installer.wait");
  verEl.textContent = apkPath;
  metaEl.innerHTML = "";
  iconImg.classList.add("hidden");
  iconFallback.classList.remove("hidden");
  progressWrap.classList.add("hidden");
  warningEl.classList.add("hidden");
  errorEl.classList.add("hidden");
  cancelBtn.disabled = false;
  cancelBtn.textContent = t("common.cancel");
  confirmBtn.disabled = true;
  confirmBtn.classList.remove("hidden");
  confirmBtn.textContent = t("installer.install");
  launchBtn.textContent = t("installer.openApp");
  launchBtn.classList.add("hidden");
  modal.classList.remove("hidden");

  try {
    const preview = await invoke("preview_package_install", { apkPath });
    state.currentInstallerPreview = preview;

    const verSuffix = preview.new_version ? ` v${preview.new_version}` : "";
    questionEl.textContent = preview.is_update
      ? t("installer.askUpdate")
      : t("installer.askInstall");
    titleEl.textContent = `${preview.display_name}${verSuffix}`;

    if (preview.is_update) {
      const fromVer = preview.installed_version ? `v${preview.installed_version}` : "—";
      const toVer = preview.new_version ? `v${preview.new_version}` : "—";
      verEl.textContent = t("installer.updateLine", { from: fromVer, to: toVer });
    } else {
      verEl.textContent = preview.package_name || preview.path;
    }

    if (preview.icon_data_url) {
      iconImg.src = preview.icon_data_url;
      iconImg.classList.remove("hidden");
      iconFallback.classList.add("hidden");
    }

    const abiLabel = !preview.abi_supported
      ? t("installer.abiUnsupported", { abis: preview.detected_abis.join(", ") })
      : preview.detected_abis.length
        ? t("installer.abiNative")
        : t("installer.abiUniversal");

    const chips = [
      `<span class="meta-chip ${preview.abi_supported ? "ok" : "err"}">${escapeHtml(abiLabel)}</span>`,
      `<span class="meta-chip">${escapeHtml(formatBytes(preview.size_bytes))}</span>`,
      `<span class="meta-chip">${escapeHtml(
        preview.is_split_bundle
          ? t("installer.splitBundle", { count: preview.split_apk_count })
          : t("installer.singleApk")
      )}</span>`,
    ];
    if (preview.required_gles) {
      chips.push(
        `<span class="meta-chip ${preview.gles_warning ? "warn" : ""}">OpenGL ES ${escapeHtml(preview.required_gles)}</span>`
      );
    }
    metaEl.innerHTML = chips.join("");

    if (preview.gles_warning) {
      warningEl.textContent = preview.gles_warning;
      warningEl.classList.remove("hidden");
    }

    if (!preview.abi_supported) {
      errorEl.textContent = t("installer.errAbi");
      errorEl.classList.remove("hidden");
      confirmBtn.disabled = true;
    } else {
      confirmBtn.textContent = preview.is_update ? t("installer.update") : t("installer.install");
      confirmBtn.disabled = false;
    }
  } catch (err) {
    questionEl.textContent = "";
    titleEl.textContent = t("installer.invalidTitle");
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
  setInstallerProgress(8, t("installer.starting"), false);

  try {
    const res = await invoke("install_apk_file", { apkPath: state.currentInstallerPreview.path });
    state.currentInstalledPackage = res.package_name || state.currentInstallerPreview.package_name;
    setInstallerProgress(
      100,
      t(state.currentInstallerPreview.is_update ? "installer.doneUpdate" : "installer.doneInstall", {
        name: state.currentInstallerPreview.display_name,
      }),
      false
    );
    confirmBtn.classList.add("hidden");
    cancelBtn.disabled = false;
    cancelBtn.textContent = t("common.close");
    if (state.currentInstalledPackage) {
      launchBtn.classList.remove("hidden");
    }
    if (!state.standaloneInstallerMode) {
      showToast(state.currentInstallerPreview.display_name);
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
