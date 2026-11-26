// Entry point: initializes Windows 11 Mica/accent shell, then opens the App Installer (--install-apk) or Home.

import { invoke, state } from "./core.js";
import { refreshAll, refreshApps } from "./data.js";
import { wireEvents } from "./events.js";
import { t } from "./i18n.js";
import { openInstallDialog, setupInstallProgressListener } from "./installer.js";
import { maybeOpenSetup, wireSetup } from "./setup.js";
import {
  initShellAppearance,
  setupDragDrop,
  setupNavigation,
  setupWindowControls,
} from "./shell.js";

document.addEventListener("DOMContentLoaded", async () => {
  setupWindowControls();
  setupNavigation();
  await initShellAppearance();
  setupDragDrop();
  setupInstallProgressListener();
  wireEvents();
  wireSetup();

  if (invoke) {
    try {
      const pendingPath = await invoke("get_pending_install");
      if (pendingPath) {
        state.standaloneInstallerMode = true;
        document.body.classList.add("installer-mode");
        document.getElementById("titlebar-title").textContent = t("installer.titleWindow");
        await openInstallDialog(pendingPath);
        return;
      }
    } catch {
      // Fall through to full app mode.
    }
  }

  refreshAll();
  refreshApps();
  maybeOpenSetup();
});
