// Backend data fetching & configuration persistence through Tauri `invoke`.

import { escapeHtml, flashSavedIndicator, invoke, showToast, state } from "./core.js";
import { t } from "./i18n.js";
import { renderAppsList, renderDashboard } from "./views.js";

export async function refreshAll() {
  if (!invoke) return;
  try {
    const snap = await invoke("get_dashboard");
    renderDashboard(snap);
    if (state.devMode) {
      await Promise.allSettled([
        refreshLogs(),
        refreshProcesses(),
        refreshNetworkForwards(),
        refreshImages(),
      ]);
    }
  } catch (err) {
    showToast(`${err}`, true);
  }
}

export async function saveCurrentConfig() {
  if (!invoke || !state.lastDashboard) return;
  const base = state.lastDashboard.config;
  const updated = {
    schema_version: base.schema_version,
    runtime: {
      cpu_cores: Number(document.getElementById("cfg-cpu").value) || base.runtime.cpu_cores,
      memory_mb: Number(document.getElementById("cfg-ram").value) || base.runtime.memory_mb,
      storage_gb: Number(document.getElementById("cfg-storage").value) || base.runtime.storage_gb,
      graphics_mode: document.getElementById("cfg-graphics").value || base.runtime.graphics_mode,
      audio_enabled: document.getElementById("cfg-audio-enabled").checked,
      data_dir: base.runtime.data_dir,
    },
    display: {
      width: Number(document.getElementById("cfg-width").value) || base.display.width,
      height: Number(document.getElementById("cfg-height").value) || base.display.height,
      dpi: Number(document.getElementById("cfg-dpi").value) || base.display.dpi,
    },
    desktop: {
      auto_sync_shortcuts: document.getElementById("cfg-auto-sync-shortcuts").checked,
      forward_notifications: document.getElementById("cfg-forward-notifications").checked,
      auto_sync_clipboard: document.getElementById("cfg-auto-sync-clipboard").checked,
      close_to_tray: document.getElementById("cfg-close-to-tray").checked,
    },
    network: {
      adb_bind_address:
        document.getElementById("cfg-adb-host").value.trim() || base.network.adb_bind_address,
      adb_port: Number(document.getElementById("cfg-adb-port").value) || base.network.adb_port,
      allow_non_loopback_adb: base.network.allow_non_loopback_adb,
    },
    logging: {
      level: document.getElementById("cfg-log-level").value || base.logging.level,
      json: base.logging.json,
    },
    images: base.images ?? { trusted_keys: [] },
  };

  try {
    await invoke("save_config", { config: updated });
    flashSavedIndicator();
    await refreshAll();
  } catch (err) {
    showToast(String(err), true);
  }
}

export async function refreshApps() {
  if (!invoke) return;
  const container = document.getElementById("apps-table-container");
  const thirdPartyOnly = document.getElementById("apps-third-party")?.checked ?? true;
  try {
    const [apps, shortcuts] = await Promise.all([
      invoke("list_apps", { thirdPartyOnly }),
      invoke("list_app_shortcuts").catch(() => []),
    ]);
    state.lastApps = apps || [];
    state.lastShortcuts = shortcuts || [];
    renderAppsList();
  } catch {
    state.lastApps = [];
    if (container) {
      container.innerHTML = `<div class="setting-row"><span class="muted">${escapeHtml(t("apps.emptyOffline"))}</span></div>`;
    }
  }
}

export async function refreshLogs() {
  if (!invoke) return;
  const el = document.getElementById("diag-logs");
  if (!el) return;
  try {
    const logs = await invoke("get_logs", { tail: 100 });
    const lines = [
      `=== Serial (${logs.serial_log_path}) ===`,
      ...(logs.serial_lines.length ? logs.serial_lines : ["(empty)"]),
    ];
    if (logs.vmm_stderr_lines.length) {
      lines.push("", `=== VMM stderr (${logs.vmm_stderr_path}) ===`, ...logs.vmm_stderr_lines);
    }
    el.textContent = lines.join("\n");
  } catch (err) {
    el.textContent = `${err}`;
  }
}

export async function refreshProcesses() {
  if (!invoke) return;
  const el = document.getElementById("dash-processes");
  if (!el) return;
  try {
    const procs = await invoke("list_processes", { filter: null });
    if (!procs.length) {
      el.textContent = "—";
      return;
    }
    el.innerHTML = procs
      .slice(0, 14)
      .map(
        (p) =>
          `<div class="item-row"><code>${escapeHtml(p.name)}</code><span class="muted small-text">PID ${p.pid} · ${p.rss_kb} KB</span></div>`
      )
      .join("");
  } catch {
    el.textContent = "—";
  }
}

export async function refreshNetworkForwards() {
  if (!invoke) return;
  const el = document.getElementById("dev-forwards-list");
  if (!el) return;
  try {
    const rules = await invoke("list_network_forwards");
    el.innerHTML = rules
      .map(
        (r) =>
          `<div class="item-row"><code>${escapeHtml(r.local)} → ${escapeHtml(r.remote)}</code><span class="muted small-text">${escapeHtml(r.serial)}</span></div>`
      )
      .join("");
  } catch {
    el.innerHTML = "";
  }
}

export async function refreshImages() {
  if (!invoke) return;
  const container = document.getElementById("images-list");
  if (!container) return;
  try {
    const reg = await invoke("list_images");
    const images = reg?.images || [];
    if (!images.length) {
      container.innerHTML = `<span class="muted small-text">—</span>`;
    } else {
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
                  : `<button class="btn small primary" data-activate-image="${escapeHtml(img.id)}">Use</button>`
              }
            </div>
          </div>`;
        })
        .join("");

      container.querySelectorAll("[data-activate-image]").forEach((btn) =>
        btn.addEventListener("click", async () => {
          try {
            const activated = await invoke("activate_image", { imageId: btn.dataset.activateImage });
            showToast(`${activated.id} (Android ${activated.android_version})`);
            await refreshAll();
          } catch (err) {
            showToast(String(err), true);
          }
        })
      );
    }
  } catch (err) {
    container.textContent = `${err}`;
  }
  await refreshTrustedKeys();
}

export async function refreshTrustedKeys(keys) {
  if (!invoke) return;
  const container = document.getElementById("trusted-keys-list");
  if (!container) return;
  try {
    const list = keys ?? (await invoke("list_trusted_keys"));
    if (!list.length) {
      container.innerHTML = `<span class="muted small-text">—</span>`;
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
          <button class="btn small danger" data-remove-key="${escapeHtml(k.public_key)}">×</button>
        </div>`
      )
      .join("");
    container.querySelectorAll("[data-remove-key]").forEach((btn) =>
      btn.addEventListener("click", () => removeTrustedKey(btn.dataset.removeKey))
    );
  } catch (err) {
    container.textContent = `${err}`;
  }
}

async function removeTrustedKey(publicKey) {
  try {
    const affected = await invoke("trusted_key_removal_impact", { publicKey });
    const note = affected.length ? `\n\nAffected images: ${affected.join(", ")}` : "";
    if (!window.confirm(`Remove publisher key ${publicKey.slice(0, 16)}…?${note}`)) return;
    const keys = await invoke("remove_trusted_key", { publicKey });
    await refreshTrustedKeys(keys);
  } catch (err) {
    showToast(String(err), true);
  }
}
