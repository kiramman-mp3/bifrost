// Navigation, theme and drag-and-drop wiring.

import { showToast } from "./core.js";
import { openInstallDialog } from "./installer.js";
import { activateSection } from "./views.js";

export function setupNavigation() {
  document.querySelectorAll(".nav-item").forEach((btn) => {
    btn.addEventListener("click", () => activateSection(btn.dataset.section));
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
      activateSection("apps");
      document.getElementById("apk-path-input").value = apkFile;
      await openInstallDialog(apkFile);
      return;
    }

    const jsonFile = paths.find((p) => p.toLowerCase().endsWith(".json"));
    if (jsonFile) {
      document.getElementById("manifest-input").value = jsonFile;
      const imgInput = document.getElementById("img-manifest-input");
      if (imgInput) imgInput.value = jsonFile;
      showToast(`Loaded manifest path: ${jsonFile}`);
      return;
    }

    showToast("Drop an .apk package or a Midgard manifest.json file.", true);
  });
}

export function setupTheme() {
  const saved = localStorage.getItem("bifrost-theme");
  const prefersLight = window.matchMedia?.("(prefers-color-scheme: light)")?.matches;
  const initial = saved || (prefersLight ? "light" : "dark");
  applyTheme(initial);

  document.getElementById("theme-toggle").addEventListener("click", () => {
    const current = document.documentElement.getAttribute("data-theme") || "dark";
    applyTheme(current === "dark" ? "light" : "dark");
  });
}

export function applyTheme(theme) {
  document.documentElement.setAttribute("data-theme", theme);
  localStorage.setItem("bifrost-theme", theme);
  document.getElementById("theme-toggle").textContent =
    theme === "dark" ? "Theme: Dark" : "Theme: Light";
}
