// Shared helpers and mutable UI state for the BIFRÖST desktop shell.

export const invoke = window.__TAURI__?.core?.invoke;

/** Mutable state shared across modules (imported bindings are read-only). */
export const state = {
  lastDashboard: null,
  lastDoctor: null,
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

export function showToast(message, isError = false) {
  const el = document.getElementById("toast-banner");
  el.textContent = message;
  el.classList.toggle("error", isError);
  el.classList.remove("hidden");
  clearTimeout(showToast._timer);
  showToast._timer = setTimeout(() => el.classList.add("hidden"), 5000);
}

export function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

export function formatGib(bytes) {
  if (!bytes || bytes <= 0) return "0.00 GB";
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}
