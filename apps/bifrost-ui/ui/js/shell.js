// Window chrome, native Mica + dynamic Windows accent/theme/language sync, navigation, and drag-and-drop.

import { invoke, showToast, state } from "./core.js";
import { refreshApps } from "./data.js";
import { applyTranslations, setLang, t } from "./i18n.js";
import { openInstallDialog } from "./installer.js";
import { renderSetup } from "./setup.js";
import { activateSection, applyDeveloperMode, renderDashboard } from "./views.js";

const THEME_KEY = "bifrost-theme-pref";
const LANG_KEY = "bifrost-lang-pref";
const DEV_KEY = "bifrost-dev-mode";

function safeGet(key, fallback) {
  try {
    return localStorage.getItem(key) || fallback;
  } catch {
    return fallback;
  }
}

function safeSet(key, val) {
  try {
    localStorage.setItem(key, val);
  } catch {
    // Ignore storage quota / private mode errors.
  }
}

export function setupNavigation() {
  document.querySelectorAll(".nav-item").forEach((btn) => {
    btn.addEventListener("click", () => {
      const section = btn.dataset.section;
      activateSection(section);
      if (section === "apps") {
        refreshApps();
      }
    });
  });
}

export function setupWindowControls() {
  const getCurrentWindow = window.__TAURI__?.window?.getCurrentWindow;
  if (!getCurrentWindow) return;

  document.documentElement.classList.add("tauri-mica");
  const appWin = getCurrentWindow();

  const minBtn = document.getElementById("win-minimize");
  const maxBtn = document.getElementById("win-maximize");
  const closeBtn = document.getElementById("win-close");
  const maxIcon = document.getElementById("win-max-icon");
  const restoreIcon = document.getElementById("win-restore-icon");

  const syncMaxIcon = async () => {
    try {
      const maximized = await appWin.isMaximized();
      maxIcon?.classList.toggle("hidden", maximized);
      restoreIcon?.classList.toggle("hidden", !maximized);
    } catch {
      // Ignore if window handle is closing.
    }
  };

  minBtn?.addEventListener("click", () => appWin.minimize());
  maxBtn?.addEventListener("click", async () => {
    await appWin.toggleMaximize();
    await syncMaxIcon();
  });
  closeBtn?.addEventListener("click", () => {
    if (state.standaloneInstallerMode) {
      invoke?.("close_installer_window");
    } else {
      appWin.close();
    }
  });

  syncMaxIcon();

  const listen = window.__TAURI__?.event?.listen;
  if (listen) {
    listen("tauri://resize", syncMaxIcon);
    listen("tauri://focus", () => {
      document.body.classList.remove("window-unfocused");
      syncWindowsAppearance();
    });
    listen("tauri://blur", () => {
      document.body.classList.add("window-unfocused");
    });
  }

  window.addEventListener("focus", () => {
    document.body.classList.remove("window-unfocused");
    syncWindowsAppearance();
  });
  window.addEventListener("blur", () => {
    document.body.classList.add("window-unfocused");
  });
}

export async function syncWindowsAppearance() {
  if (invoke) {
    try {
      state.appearance = await invoke("get_windows_appearance");
    } catch {
      // Keep CSS defaults when unavailable.
    }
  }

  const app = state.appearance;
  if (app) {
    const root = document.documentElement.style;
    root.setProperty("--win-accent-base", app.accent);
    root.setProperty("--win-accent-light", app.accent_light);
    root.setProperty("--win-accent-light-hover", app.accent_light_hover);
    root.setProperty("--win-accent-dark", app.accent_dark);
    root.setProperty("--win-accent-dark-hover", app.accent_dark_hover);
    root.setProperty("--win-text-on-accent-light", app.text_on_accent_light);
    root.setProperty("--win-text-on-accent-dark", app.text_on_accent_dark);
  }

  applyEffectiveTheme();
  applyEffectiveLanguage();
}

export function applyEffectiveTheme() {
  const systemDark = state.appearance
    ? state.appearance.system_theme === "dark"
    : !window.matchMedia?.("(prefers-color-scheme: light)")?.matches;

  const resolved =
    state.themePref === "dark"
      ? "dark"
      : state.themePref === "light"
        ? "light"
        : systemDark
          ? "dark"
          : "light";

  document.documentElement.setAttribute("data-theme", resolved);
  const select = document.getElementById("pref-theme");
  if (select && select.value !== state.themePref) {
    select.value = state.themePref;
  }
}

export function applyEffectiveLanguage() {
  const browserEs = (navigator.language || "").toLowerCase().startsWith("es");
  const systemLang = state.appearance?.preferred_language || (browserEs ? "es" : "en");
  const resolved = state.langPref === "auto" ? systemLang : state.langPref;

  setLang(resolved);
  applyTranslations();

  const select = document.getElementById("pref-lang");
  if (select && select.value !== state.langPref) {
    select.value = state.langPref;
  }
  const localeHint = document.getElementById("settings-locale-hint");
  if (localeHint) {
    const loc = state.appearance?.system_locale || navigator.language || "en-US";
    localeHint.textContent = t("settings.langDetected", { locale: loc });
  }
  if (state.standaloneInstallerMode) {
    document.getElementById("titlebar-title").textContent = t("installer.titleWindow");
  }
  if (state.lastDashboard) {
    renderDashboard(state.lastDashboard);
  }
  if (state.lastSetupPlan) {
    renderSetup(state.lastSetupPlan);
  }
}

export async function initShellAppearance() {
  state.themePref = safeGet(THEME_KEY, "system");
  state.langPref = safeGet(LANG_KEY, "auto");
  state.devMode = safeGet(DEV_KEY, "0") === "1";

  applyDeveloperMode(state.devMode);
  await syncWindowsAppearance();

  document.getElementById("pref-theme")?.addEventListener("change", (e) => {
    state.themePref = e.target.value;
    safeSet(THEME_KEY, state.themePref);
    applyEffectiveTheme();
  });

  document.getElementById("pref-lang")?.addEventListener("change", (e) => {
    state.langPref = e.target.value;
    safeSet(LANG_KEY, state.langPref);
    applyEffectiveLanguage();
  });

  document.getElementById("toggle-dev-mode")?.addEventListener("change", (e) => {
    state.devMode = e.target.checked;
    safeSet(DEV_KEY, state.devMode ? "1" : "0");
    applyDeveloperMode(state.devMode);
  });
}

export function setupDragDrop() {
  const listen = window.__TAURI__?.event?.listen;
  const overlay = document.getElementById("drop-overlay");
  if (!listen || !overlay) return;

  listen("tauri://drag-enter", () => overlay.classList.remove("hidden"));
  listen("tauri://drag-leave", () => overlay.classList.add("hidden"));
  listen("tauri://drag-drop", async (event) => {
    overlay.classList.add("hidden");
    const paths = event?.payload?.paths || [];
    if (!paths.length) return;

    const apkFile = paths.find((p) => {
      const lower = p.toLowerCase();
      return lower.endsWith(".apk") || lower.endsWith(".apkm");
    });
    if (apkFile) {
      await openInstallDialog(apkFile);
      return;
    }

    const jsonFile = paths.find((p) => p.toLowerCase().endsWith(".json"));
    if (jsonFile) {
      const m1 = document.getElementById("manifest-input");
      const m2 = document.getElementById("img-manifest-input");
      if (m1) m1.value = jsonFile;
      if (m2) m2.value = jsonFile;
      if (!state.devMode) {
        state.devMode = true;
        safeSet(DEV_KEY, "1");
        applyDeveloperMode(true);
      }
      activateSection("developer");
      showToast(`manifest.json: ${jsonFile}`);
      return;
    }

    showToast(t("drop.invalid"), true);
  });
}
