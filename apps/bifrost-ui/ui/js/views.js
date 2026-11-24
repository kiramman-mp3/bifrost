// Rendering of the Dashboard, Runtime Configuration, Diagnostics and Settings views.

import { escapeHtml, formatGib, state } from "./core.js";

export function activateSection(sectionName) {
  const buttons = document.querySelectorAll(".nav-item");
  buttons.forEach((b) => {
    const active = b.dataset.section === sectionName;
    b.classList.toggle("active", active);
    b.setAttribute("aria-selected", String(active));
  });
  document.querySelectorAll(".panel").forEach((panel) => {
    panel.classList.toggle("active", panel.id === `section-${sectionName}`);
  });
}

export function renderDashboard(data) {
  state.lastDashboard = data;
  state.lastDoctor = data.doctor;

  const runtimeState = data.runtime_state || "stopped";
  document.getElementById("dash-state").textContent = runtimeState.toUpperCase();
  document.getElementById("dash-heimdall").textContent = data.heimdall_connected
    ? `Heimdall: connected (PID ${data.heimdall_health?.daemon_pid ?? "?"})`
    : "Heimdall: disconnected";

  const sidebarDot = document.getElementById("sidebar-runtime-dot");
  sidebarDot.className = "dot " + (runtimeState === "running" ? "ok" : data.heimdall_connected ? "warn" : "");
  document.getElementById("sidebar-runtime-label").textContent = `Midgard: ${runtimeState}`;

  document.getElementById("dash-android").textContent = data.adb_access_hint
    ? `ADB blocked: ${data.adb_access_hint}`
    : data.runtime.android_version
      ? `Android ${data.runtime.android_version}`
      : "Not booted";
  document.getElementById("dash-image").textContent =
    data.runtime.image_id ? `Image: ${data.runtime.image_id}` : "Image: none loaded";

  document.getElementById("dash-cpu").textContent = `${data.config.runtime.cpu_cores} vCPUs`;
  document.getElementById("dash-cpu-host").textContent =
    `${data.doctor.snapshot.cpu.logical_cores} logical host cores`;

  document.getElementById("dash-ram").textContent = `${data.config.runtime.memory_mb} MB`;
  document.getElementById("dash-ram-host").textContent =
    `Host: ${formatGib(data.doctor.snapshot.memory.total_bytes)}`;

  document.getElementById("dash-storage").textContent = `${data.config.runtime.storage_gb} GB`;
  document.getElementById("dash-storage-host").textContent =
    `Free: ${formatGib(data.doctor.snapshot.storage.available_bytes)}`;

  document.getElementById("dash-readiness").textContent = String(data.doctor.overall)
    .replaceAll("_", " ")
    .toUpperCase();
  document.getElementById("dash-adb").textContent = `ADB devices: ${data.devices.length}`;

  const heimdallBtn = document.getElementById("btn-heimdall-toggle");
  if (heimdallBtn) {
    heimdallBtn.textContent = data.heimdall_connected ? "Stop Heimdall" : "Start Heimdall";
  }

  const defaultManifest = data.runtime.manifest_path || data.active_manifest_path;
  if (defaultManifest && !document.getElementById("manifest-input").value) {
    document.getElementById("manifest-input").value = defaultManifest;
  }

  populateConfigForm(data.config, data.config_path);
  renderDoctor(data.doctor);
  renderSettings(data);
}

export function populateConfigForm(cfg, configPath) {
  document.getElementById("cfg-cpu").value = cfg.runtime.cpu_cores;
  document.getElementById("cfg-ram").value = cfg.runtime.memory_mb;
  document.getElementById("cfg-storage").value = cfg.runtime.storage_gb;
  document.getElementById("cfg-graphics").value = cfg.runtime.graphics_mode;
  document.getElementById("cfg-width").value = cfg.display.width;
  document.getElementById("cfg-height").value = cfg.display.height;
  document.getElementById("cfg-dpi").value = cfg.display.dpi;
  document.getElementById("cfg-adb-host").value = cfg.network.adb_bind_address;
  document.getElementById("cfg-adb-port").value = cfg.network.adb_port;
  document.getElementById("cfg-log-level").value = cfg.logging.level;
  document.getElementById("cfg-audio-enabled").checked = cfg.runtime.audio_enabled ?? true;
  document.getElementById("cfg-auto-sync-shortcuts").checked =
    cfg.desktop?.auto_sync_shortcuts ?? true;
  document.getElementById("cfg-forward-notifications").checked =
    cfg.desktop?.forward_notifications ?? true;
  document.getElementById("cfg-auto-sync-clipboard").checked =
    cfg.desktop?.auto_sync_clipboard ?? true;
  document.getElementById("cfg-path-label").textContent = `Target: ${configPath}`;
}

export function renderDoctor(report) {
  const container = document.getElementById("diag-checks");
  container.innerHTML = report.checks
    .map(
      (c) => `
      <div class="check-row">
        <span class="badge ${escapeHtml(c.status)}">${escapeHtml(c.status)}</span>
        <div>
          <strong>${escapeHtml(c.name)}</strong> — ${escapeHtml(c.summary)}
          <div class="muted small-text">${escapeHtml(c.detail)}</div>
          ${
            c.remediation
              ? `<div class="small-text">Action: ${escapeHtml(c.remediation)}</div>`
              : ""
          }
        </div>
      </div>`
    )
    .join("");
}

export function renderSettings(data) {
  const pathsEl = document.getElementById("settings-paths");
  pathsEl.innerHTML = `
    <dt>Config File</dt><dd>${escapeHtml(data.config_path)}</dd>
    <dt>Data Directory</dt><dd>${escapeHtml(data.paths.data_dir)}</dd>
    <dt>Images Directory</dt><dd>${escapeHtml(data.paths.images_dir)}</dd>
    <dt>Logs Directory</dt><dd>${escapeHtml(data.paths.logs_dir)}</dd>
    <dt>Win32 Clipboard</dt><dd>${data.clipboard_has_text ? "Unicode text present" : "Empty / non-text"}</dd>
  `;

  const capsEl = document.getElementById("settings-capabilities");
  capsEl.innerHTML = data.version.capabilities
    .map(
      (cap) => `
      <div class="check-row">
        <span class="badge ${escapeHtml(cap.status)}">${escapeHtml(cap.status.replaceAll("_", " "))}</span>
        <div>
          <strong>${escapeHtml(cap.name)}</strong>
          <div class="muted small-text">${escapeHtml(cap.summary)}</div>
        </div>
      </div>`
    )
    .join("");

  const autostartBtn = document.getElementById("btn-heimdall-autostart");
  const autostartStatus = document.getElementById("settings-autostart-status");
  if (autostartBtn && autostartStatus) {
    autostartBtn.textContent = data.heimdall_autostart ? "Disable Autostart" : "Enable Autostart";
    autostartStatus.textContent = data.heimdall_autostart
      ? "Enabled: Heimdall starts silently at Windows login via HKCU\\...\\Run\\BifrostHeimdall (zero admin)."
      : "Disabled: Start Heimdall manually or enable silent Windows login autostart (HKCU\\...\\Run, no admin required).";
  }
}
