// Data refresh: every call goes to the Rust backend through Tauri `invoke`.

import { escapeHtml, invoke, showToast } from "./core.js";
import { renderDashboard } from "./views.js";

export async function refreshAll() {
  if (!invoke) {
    showToast("Tauri IPC bridge unavailable outside BIFRÖST desktop host.", true);
    return;
  }
  try {
    const snap = await invoke("get_dashboard");
    renderDashboard(snap);
    await Promise.allSettled([
      refreshLogs(),
      refreshSharedFiles(),
      refreshProcesses(),
      refreshImages(),
    ]);
  } catch (err) {
    showToast(`Failed to refresh dashboard: ${err}`, true);
  }
}

export async function refreshLogs() {
  try {
    const logs = await invoke("get_logs", { tail: 100 });
    const lines = [
      `=== Serial Log (${logs.serial_log_path}) ===`,
      ...(logs.serial_lines.length ? logs.serial_lines : ["(no serial output recorded yet)"]),
    ];
    if (logs.vmm_stderr_lines.length) {
      lines.push("", `=== VMM Stderr (${logs.vmm_stderr_path}) ===`, ...logs.vmm_stderr_lines);
    }
    document.getElementById("diag-logs").textContent = lines.join("\n");
  } catch (err) {
    document.getElementById("diag-logs").textContent = `Unable to load logs: ${err}`;
  }
}

export async function refreshSharedFiles() {
  const el = document.getElementById("settings-shared-files");
  try {
    const files = await invoke("list_shared_files");
    if (!files.length) {
      el.innerHTML = `<span class="muted">Host shared directory is empty.</span>`;
      return;
    }
    el.innerHTML = files
      .map(
        (f) =>
          `<div class="item-row"><span>${escapeHtml(f.relative_path)}</span><span class="muted small-text">${f.is_dir ? "dir" : `${f.size_bytes} bytes`}</span></div>`
      )
      .join("");
  } catch (err) {
    el.textContent = `Failed to list shared folder: ${err}`;
  }
}

export async function refreshImages() {
  const container = document.getElementById("images-list");
  if (!container) return;
  try {
    const reg = await invoke("list_images");
    const images = reg?.images || [];
    if (!images.length) {
      container.innerHTML = `<span class="muted">No Midgard images registered yet. Browse for a verified manifest.json below.</span>`;
      return;
    }
    container.innerHTML = images
      .map((img) => {
        const isActive = reg.active_image_id === img.id;
        return `
        <div class="item-row">
          <div>
            <strong>${escapeHtml(img.name)}</strong> <code>${escapeHtml(img.id)}</code>
            <div class="muted small-text">Android ${escapeHtml(img.android_version)} · ${escapeHtml(img.arch)} · ${escapeHtml(img.manifest_path)}</div>
          </div>
          <div class="actions">
            ${
              isActive
                ? `<span class="badge pass">Active</span>`
                : `<button class="btn small primary" data-activate-image="${escapeHtml(img.id)}">Set Active</button>`
            }
          </div>
        </div>`;
      })
      .join("");

    container.querySelectorAll("[data-activate-image]").forEach((btn) =>
      btn.addEventListener("click", async () => {
        const imageId = btn.dataset.activateImage;
        try {
          const activated = await invoke("activate_image", { imageId });
          showToast(`Active Midgard image set to '${activated.id}' (Android ${activated.android_version}).`);
          await refreshAll();
        } catch (err) {
          showToast(String(err), true);
        }
      })
    );
  } catch (err) {
    container.textContent = `Failed to load image registry: ${err}`;
  }
  await refreshTrustedKeys();
}

export async function refreshProcesses() {
  const el = document.getElementById("dash-processes");
  try {
    const procs = await invoke("list_processes", { filter: null });
    if (!procs.length) {
      el.textContent = "No running Android processes reported.";
      return;
    }
    el.innerHTML = procs
      .slice(0, 12)
      .map(
        (p) =>
          `<div class="item-row"><code>${escapeHtml(p.name)}</code><span class="muted small-text">PID ${p.pid} · ${p.rss_kb} KB</span></div>`
      )
      .join("");
  } catch {
    el.textContent = "No online Android device connected.";
  }
}

export async function refreshApps() {
  const container = document.getElementById("apps-table-container");
  const thirdPartyOnly = document.getElementById("apps-third-party").checked;
  try {
    const [apps, shortcuts] = await Promise.all([
      invoke("list_apps", { thirdPartyOnly }),
      invoke("list_app_shortcuts").catch(() => []),
    ]);
    const pinnedSet = new Set(shortcuts.map((s) => s.package_name));
    if (!apps.length) {
      container.textContent = "No installed packages found.";
      return;
    }
    container.innerHTML = apps
      .map((app) => {
        const isPinned = pinnedSet.has(app.package_name);
        return `
        <div class="item-row">
          <div>
            <strong>${escapeHtml(app.package_name)}</strong>
            <div class="muted small-text">${app.is_system ? "system" : "user"} package${isPinned ? " · pinned to Desktop & Start Menu" : ""}</div>
          </div>
          <div class="actions">
            <button class="btn small primary" data-app-window="${escapeHtml(app.package_name)}">Open Window</button>
            <button class="btn small" data-app-shortcut="${escapeHtml(app.package_name)}" data-pinned="${isPinned}">${isPinned ? "Unpin" : "Pin"}</button>
            <button class="btn small" data-app-info="${escapeHtml(app.package_name)}">Info</button>
            <button class="btn small" data-app-stop="${escapeHtml(app.package_name)}">Stop</button>
            <button class="btn small danger" data-app-uninstall="${escapeHtml(app.package_name)}">Uninstall</button>
          </div>
        </div>`;
      })
      .join("");

    container.querySelectorAll("[data-app-window]").forEach((btn) =>
      btn.addEventListener("click", async () => {
        const pkg = btn.dataset.appWindow;
        try {
          const handle = await invoke("launch_app_window", {
            package: pkg,
            width: null,
            height: null,
            dpi: null,
          });
          showToast(`Opened '${handle.window_title}' (PID ${handle.pid})`);
        } catch (err) {
          showToast(String(err), true);
        }
      })
    );
    container.querySelectorAll("[data-app-shortcut]").forEach((btn) =>
      btn.addEventListener("click", async () => {
        const pkg = btn.dataset.appShortcut;
        const pinned = btn.dataset.pinned === "true";
        try {
          if (pinned) {
            await invoke("remove_app_shortcut", { package: pkg });
            showToast(`Removed Windows shortcuts for ${pkg}`);
          } else {
            const rec = await invoke("create_app_shortcut", {
              package: pkg,
              name: null,
              apkPath: null,
            });
            showToast(`Pinned '${rec.display_name}' to Desktop & Start Menu`);
          }
          await refreshApps();
        } catch (err) {
          showToast(String(err), true);
        }
      })
    );
    container.querySelectorAll("[data-app-info]").forEach((btn) =>
      btn.addEventListener("click", async () => {
        try {
          const info = await invoke("get_app_info", { package: btn.dataset.appInfo });
          document.getElementById("app-details-title").textContent = info.package_name;
          document.getElementById("app-details-body").textContent = JSON.stringify(info, null, 2);
          document.getElementById("app-details-card").classList.remove("hidden");
        } catch (err) {
          showToast(String(err), true);
        }
      })
    );
    container.querySelectorAll("[data-app-stop]").forEach((btn) =>
      btn.addEventListener("click", async () => {
        try {
          await invoke("stop_app", { package: btn.dataset.appStop });
          showToast(`Force-stopped ${btn.dataset.appStop}`);
        } catch (err) {
          showToast(String(err), true);
        }
      })
    );
    container.querySelectorAll("[data-app-uninstall]").forEach((btn) =>
      btn.addEventListener("click", async () => {
        const pkg = btn.dataset.appUninstall;
        if (!window.confirm(`Uninstall Android package '${pkg}'?`)) return;
        try {
          await invoke("uninstall_app", { package: pkg, keepData: false });
          showToast(`Uninstalled ${pkg}`);
          await refreshApps();
        } catch (err) {
          showToast(String(err), true);
        }
      })
    );
  } catch (err) {
    container.textContent = String(err);
  }
}

/** Renders `[images] trusted_keys`; `keys` skips the backend call when already known. */
export async function refreshTrustedKeys(keys) {
  const container = document.getElementById("trusted-keys-list");
  if (!container) return;
  try {
    const list = keys ?? (await invoke("list_trusted_keys"));
    if (!list.length) {
      container.innerHTML = `<span class="muted">No trusted publishers. Signed bundles cannot be registered as signed until you add the publisher's key.</span>`;
      return;
    }
    container.innerHTML = list
      .map(
        (k) => `
        <div class="item-row">
          <div>
            <strong>${escapeHtml(k.fingerprint)}</strong>
            <div class="muted small-text"><code>${escapeHtml(k.public_key)}</code></div>
          </div>
          <div class="actions">
            <button class="btn small danger" data-remove-key="${escapeHtml(k.public_key)}">Remove</button>
          </div>
        </div>`
      )
      .join("");
    container.querySelectorAll("[data-remove-key]").forEach((btn) =>
      btn.addEventListener("click", () => removeTrustedKey(btn.dataset.removeKey))
    );
  } catch (err) {
    container.textContent = `Failed to load trusted publishers: ${err}`;
  }
}

async function removeTrustedKey(publicKey) {
  try {
    const affected = await invoke("trusted_key_removal_impact", { publicKey });
    const impact = affected.length
      ? `

These registered images verify only with this key and could not be re-registered as signed: ${affected.join(", ")}. Their current registrations are kept.`
      : "";
    if (!window.confirm(`Stop trusting publisher key ${publicKey.slice(0, 16)}…?${impact}`)) return;
    const keys = await invoke("remove_trusted_key", { publicKey });
    showToast("Publisher key removed from [images] trusted_keys.");
    await refreshTrustedKeys(keys);
  } catch (err) {
    showToast(String(err), true);
  }
}
