"use strict";

const invoke = window.__TAURI__?.core?.invoke;

let lastDashboard = null;
let lastDoctor = null;

function showToast(message, isError = false) {
  const el = document.getElementById("toast-banner");
  el.textContent = message;
  el.classList.toggle("error", isError);
  el.classList.remove("hidden");
  clearTimeout(showToast._timer);
  showToast._timer = setTimeout(() => el.classList.add("hidden"), 5000);
}

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function formatGib(bytes) {
  if (!bytes || bytes <= 0) return "0.00 GB";
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}

function setupNavigation() {
  const buttons = document.querySelectorAll(".nav-item");
  buttons.forEach((btn) => {
    btn.addEventListener("click", () => {
      const target = btn.dataset.section;
      buttons.forEach((b) => {
        const active = b === btn;
        b.classList.toggle("active", active);
        b.setAttribute("aria-selected", String(active));
      });
      document.querySelectorAll(".panel").forEach((panel) => {
        panel.classList.toggle("active", panel.id === `section-${target}`);
      });
    });
  });
}

function setupTheme() {
  const saved = localStorage.getItem("bifrost-theme");
  const prefersLight = window.matchMedia?.("(prefers-color-scheme: light)")?.matches;
  const initial = saved || (prefersLight ? "light" : "dark");
  applyTheme(initial);

  document.getElementById("theme-toggle").addEventListener("click", () => {
    const current = document.documentElement.getAttribute("data-theme") || "dark";
    applyTheme(current === "dark" ? "light" : "dark");
  });
}

function applyTheme(theme) {
  document.documentElement.setAttribute("data-theme", theme);
  localStorage.setItem("bifrost-theme", theme);
  document.getElementById("theme-toggle").textContent =
    theme === "dark" ? "Theme: Dark" : "Theme: Light";
}

function renderDashboard(data) {
  lastDashboard = data;
  lastDoctor = data.doctor;

  const state = data.runtime_state || "stopped";
  document.getElementById("dash-state").textContent = state.toUpperCase();
  document.getElementById("dash-heimdall").textContent = data.heimdall_connected
    ? `Heimdall: connected (PID ${data.heimdall_health?.daemon_pid ?? "?"})`
    : "Heimdall: disconnected";

  const sidebarDot = document.getElementById("sidebar-runtime-dot");
  sidebarDot.className = "dot " + (state === "running" ? "ok" : data.heimdall_connected ? "warn" : "");
  document.getElementById("sidebar-runtime-label").textContent = `Midgard: ${state}`;

  document.getElementById("dash-android").textContent =
    data.runtime.android_version ? `Android ${data.runtime.android_version}` : "Not booted";
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

  if (data.runtime.manifest_path && !document.getElementById("manifest-input").value) {
    document.getElementById("manifest-input").value = data.runtime.manifest_path;
  }

  populateConfigForm(data.config, data.config_path);
  renderDoctor(data.doctor);
  renderSettings(data);
}

function populateConfigForm(cfg, configPath) {
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
  document.getElementById("cfg-path-label").textContent = `Target: ${configPath}`;
}

function renderDoctor(report) {
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

function renderSettings(data) {
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
}

async function refreshAll() {
  if (!invoke) {
    showToast("Tauri IPC bridge unavailable outside BIFRÖST desktop host.", true);
    return;
  }
  try {
    const snap = await invoke("get_dashboard");
    renderDashboard(snap);
    await Promise.allSettled([refreshLogs(), refreshSharedFiles(), refreshProcesses()]);
  } catch (err) {
    showToast(`Failed to refresh dashboard: ${err}`, true);
  }
}

async function refreshLogs() {
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

async function refreshSharedFiles() {
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

async function refreshProcesses() {
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

async function refreshApps() {
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

function wireEvents() {
  document.getElementById("btn-refresh-all").addEventListener("click", refreshAll);
  document.getElementById("btn-refresh-procs").addEventListener("click", refreshProcesses);
  document.getElementById("btn-refresh-logs").addEventListener("click", refreshLogs);
  document.getElementById("btn-refresh-share").addEventListener("click", refreshSharedFiles);
  document.getElementById("btn-refresh-apps").addEventListener("click", refreshApps);
  document.getElementById("btn-close-details").addEventListener("click", () => {
    document.getElementById("app-details-card").classList.add("hidden");
  });

  document.getElementById("btn-runtime-start").addEventListener("click", async () => {
    const manifestPath = document.getElementById("manifest-input").value.trim();
    const headless = document.getElementById("start-headless").checked;
    if (!manifestPath) {
      showToast("Provide a verified Midgard manifest.json path first.", true);
      return;
    }
    try {
      await invoke("start_runtime", { manifestPath, headless });
      showToast("Midgard VM start requested.");
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-runtime-restart").addEventListener("click", async () => {
    const manifestPath = document.getElementById("manifest-input").value.trim() || null;
    const headless = document.getElementById("start-headless").checked;
    try {
      await invoke("restart_runtime", { manifestPath, headless });
      showToast("Midgard VM restarted.");
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-runtime-stop").addEventListener("click", async () => {
    try {
      await invoke("stop_runtime", { force: false });
      showToast("Midgard VM stopped cleanly.");
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-inspect-apk").addEventListener("click", async () => {
    const apkPath = document.getElementById("apk-path-input").value.trim();
    if (!apkPath) {
      showToast("Enter a local .apk path first.", true);
      return;
    }
    try {
      const info = await invoke("inspect_apk_file", { apkPath });
      const box = document.getElementById("apk-inspect-output");
      box.textContent = JSON.stringify(info, null, 2);
      box.classList.remove("hidden");
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-install-apk").addEventListener("click", async () => {
    const apkPath = document.getElementById("apk-path-input").value.trim();
    if (!apkPath) {
      showToast("Enter a local .apk path first.", true);
      return;
    }
    try {
      const res = await invoke("install_apk_file", { apkPath });
      showToast(`Installed APK on ${res.serial}`);
      await refreshApps();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("config-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    if (!lastDashboard) return;
    const updated = {
      schema_version: lastDashboard.config.schema_version,
      runtime: {
        cpu_cores: Number(document.getElementById("cfg-cpu").value),
        memory_mb: Number(document.getElementById("cfg-ram").value),
        storage_gb: Number(document.getElementById("cfg-storage").value),
        graphics_mode: document.getElementById("cfg-graphics").value,
        audio_enabled: lastDashboard.config.runtime.audio_enabled ?? true,
        data_dir: lastDashboard.config.runtime.data_dir,
      },
      display: {
        width: Number(document.getElementById("cfg-width").value),
        height: Number(document.getElementById("cfg-height").value),
        dpi: Number(document.getElementById("cfg-dpi").value),
      },
      desktop: lastDashboard.config.desktop || {
        auto_sync_shortcuts: true,
        forward_notifications: true,
        auto_sync_clipboard: true,
      },
      network: {
        adb_bind_address: document.getElementById("cfg-adb-host").value.trim(),
        adb_port: Number(document.getElementById("cfg-adb-port").value),
        allow_non_loopback_adb: lastDashboard.config.network.allow_non_loopback_adb,
      },
      logging: {
        level: document.getElementById("cfg-log-level").value,
        json: lastDashboard.config.logging.json,
      },
    };
    try {
      const savedPath = await invoke("save_config", { config: updated });
      showToast(`Configuration validated and saved to ${savedPath}`);
      await refreshAll();
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-run-doctor").addEventListener("click", async () => {
    try {
      const report = await invoke("get_doctor_report");
      lastDoctor = report;
      renderDoctor(report);
      showToast(`Doctor complete: ${report.pass_count} passed, ${report.warn_count} warnings, ${report.fail_count} failed.`);
    } catch (err) {
      showToast(String(err), true);
    }
  });

  document.getElementById("btn-export-diag").addEventListener("click", async () => {
    if (!lastDoctor) return;
    try {
      await navigator.clipboard.writeText(JSON.stringify(lastDoctor, null, 2));
      showToast("Diagnostic JSON report copied to clipboard.");
    } catch (err) {
      showToast(`Failed to copy report: ${err}`, true);
    }
  });
}

document.addEventListener("DOMContentLoaded", () => {
  setupNavigation();
  setupTheme();
  wireEvents();
  refreshAll();
});
