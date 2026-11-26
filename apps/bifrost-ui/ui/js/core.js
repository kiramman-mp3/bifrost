// Shared helpers and mutable UI state for the BIFRÖST Windows 11 desktop shell.

export const invoke = window.__TAURI__?.core?.invoke;

/** Mutable state shared across ES modules. */
export const state = {
  lastDashboard: null,
  lastDoctor: null,
  lastApps: [],
  lastShortcuts: [],
  lastSetupPlan: null,
  appearance: null,
  themePref: "system",
  langPref: "auto",
  devMode: false,
  standaloneInstallerMode: false,
  currentInstallerPreview: null,
  currentInstalledPackage: null,
  installerBusy: false,
};

export function formatBytes(bytes) {
  if (!bytes || bytes <= 0) return "0 MB";
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
  return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
}

export function formatGib(bytes) {
  if (!bytes || bytes <= 0) return "0.0 GB";
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}

/** Derives a human-friendly display title from an Android package identifier when no shortcut label is recorded. */
export function friendlyAppName(packageName, shortcutTitle = null) {
  if (shortcutTitle && shortcutTitle.trim()) return shortcutTitle.trim();
  if (!packageName) return "App";
  const parts = String(packageName).split(".").filter(Boolean);
  const raw = parts[parts.length - 1] || packageName;
  return raw
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (ch) => ch.toUpperCase());
}

export function showToast(message, isError = false) {
  const el = document.getElementById("toast-banner");
  if (!el) return;
  el.textContent = message;
  el.classList.toggle("error", isError);
  el.classList.remove("hidden");
  clearTimeout(showToast._timer);
  showToast._timer = setTimeout(() => el.classList.add("hidden"), 4500);
}

export function flashSavedIndicator() {
  const pill = document.getElementById("settings-saved-pill");
  if (!pill) return;
  pill.classList.remove("hidden");
  clearTimeout(flashSavedIndicator._timer);
  flashSavedIndicator._timer = setTimeout(() => pill.classList.add("hidden"), 2200);
}

export function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}
