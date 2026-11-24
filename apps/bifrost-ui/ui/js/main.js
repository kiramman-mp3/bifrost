// Entry point: wires the shell, then opens the installer (--install-apk) or the dashboard.

import { invoke, state } from "./core.js";
import { refreshAll } from "./data.js";
import { wireEvents } from "./events.js";
import { openInstallDialog, setupInstallProgressListener } from "./installer.js";
import { setupDragDrop, setupNavigation, setupTheme } from "./shell.js";

document.addEventListener("DOMContentLoaded", async () => {
  setupNavigation();
  setupTheme();
  setupDragDrop();
  setupInstallProgressListener();
  wireEvents();

  if (invoke) {
    try {
      const pendingPath = await invoke("get_pending_install");
      if (pendingPath) {
        state.standaloneInstallerMode = true;
        document.body.classList.add("installer-mode");
        await openInstallDialog(pendingPath);
        return;
      }
    } catch {
      // Fall through to full dashboard mode if not invoked via --install-apk.
    }
  }
  refreshAll();
});
