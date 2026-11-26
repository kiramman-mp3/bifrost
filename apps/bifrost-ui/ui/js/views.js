// Rendering of Home, Apps, Settings, and Developer views.

import { escapeHtml, formatGib, friendlyAppName, invoke, showToast, state } from "./core.js";
import { refreshApps } from "./data.js";
import { t } from "./i18n.js";

export function activateSection(sectionName) {
  document.querySelectorAll(".nav-item").forEach((b) => {
    const active = b.dataset.section === sectionName;
    b.classList.toggle("active", active);
    b.setAttribute("aria-selected", String(active));
  });
  document.querySelectorAll(".page").forEach((page) => {
    page.classList.toggle("active", page.id === `section-${sectionName}`);
  });
}

export function applyDeveloperMode(enabled) {
  const toggle = document.getElementById("toggle-dev-mode");
  if (toggle) toggle.checked = enabled;

  document.getElementById("nav-developer")?.classList.toggle("hidden", !enabled);
  document.querySelectorAll("[data-dev-only]").forEach((el) => {
    el.classList.toggle("hidden", !enabled);
  });

  if (!enabled && document.getElementById("section-developer")?.classList.contains("active")) {
    activateSection("settings");
  }
  if (state.lastApps.length) {
    renderAppsList();
  }
}

function localizedStateLabel(runtimeState) {
  return t(`status.${runtimeState}`) || runtimeState;
}

export function renderDashboard(data) {
  state.lastDashboard = data;
  state.lastDoctor = data.doctor;

  const runtimeState = data.runtime_state || "stopped";
  const isRunning = runtimeState === "running";
  const isTransition = runtimeState === "starting" || runtimeState === "booting" || runtimeState === "stopping";
  const stateText = localizedStateLabel(runtimeState);

  // Sidebar status pill
  const sidebarDot = document.getElementById("sidebar-runtime-dot");
  if (sidebarDot) {
    sidebarDot.className =
      "status-dot " + (isRunning ? "ok" : isTransition || data.heimdall_connected ? "warn" : "");
  }
  document.getElementById("sidebar-runtime-label").textContent = t("sidebar.state", { state: stateText });
  document.getElementById("sidebar-substatus").textContent = t("nav.subtitle");

  // Home Hero card
  const orb = document.getElementById("hero-orb");
  if (orb) {
    orb.className = "hero-orb " + (isRunning ? "running" : isTransition ? "booting" : "");
  }
  const badge = document.getElementById("hero-status-badge");
  if (badge) {
    badge.className = "status-pill-inline " + (isRunning ? "running" : isTransition ? "booting" : "");
    badge.textContent = stateText;
  }

  const headline = document.getElementById("hero-headline");
  const desc = document.getElementById("hero-description");
  const powerBtn = document.getElementById("btn-hero-power");
  const restartBtn = document.getElementById("btn-hero-restart");

  if (isRunning) {
    headline.textContent = t("home.heroRunningTitle");
    desc.textContent = t("home.heroRunningDesc");
    powerBtn.textContent = t("home.powerOff");
    powerBtn.classList.remove("primary");
    restartBtn.classList.remove("hidden");
  } else if (isTransition) {
    headline.textContent = t("home.heroBootingTitle");
    desc.textContent = t("home.heroBootingDesc");
    powerBtn.textContent = t("home.powerOff");
    powerBtn.classList.remove("primary");
    restartBtn.classList.add("hidden");
  } else {
    headline.textContent = t("home.heroStoppedTitle");
    desc.textContent = t("home.heroStoppedDesc");
    powerBtn.textContent = t("home.powerOn");
    powerBtn.classList.add("primary");
    restartBtn.classList.add("hidden");
  }

  const androidMeta = data.runtime.android_version
    ? `Android ${data.runtime.android_version}${data.runtime.image_id ? ` · ${data.runtime.image_id}` : ""}`
    : data.active_manifest_path
      ? data.runtime.image_id || "Android x86_64"
      : t("home.noImage");
  document.getElementById("dash-android").textContent = androidMeta;

  const adbWarn = document.getElementById("home-adb-warning");
  if (data.adb_access_hint) {
    adbWarn.textContent = data.adb_access_hint;
    adbWarn.classList.remove("hidden");
  } else {
    adbWarn.classList.add("hidden");
  }

  // Home summary cards
  const ramGb = (data.config.runtime.memory_mb / 1024).toFixed(1).replace(/\.0$/, "");
  document.getElementById("home-resources-summary").textContent = t("home.resourcesSummary", {
    cores: data.config.runtime.cpu_cores,
    ram: ramGb,
    audio: data.config.runtime.audio_enabled ? t("home.audioOn") : t("home.audioOff"),
  });

  const totalChecks = data.doctor.checks.length;
  const passedChecks = data.doctor.pass_count;
  const allReady = data.doctor.fail_count === 0;
  document.getElementById("home-readiness-summary").textContent = t(
    allReady ? "home.readinessOk" : "home.readinessWarn",
    { passed: passedChecks, total: totalChecks }
  );
  const readyBadge = document.getElementById("dash-readiness");
  readyBadge.textContent = allReady ? t("home.badgeReady") : t("home.badgeCheck");
  readyBadge.className = "badge " + (allReady ? "pass" : "warn");

  const shareCount = data.shared_file_count || 0;
  document.getElementById("home-share-subtitle").textContent = shareCount
    ? t("home.tileShareCount", { count: shareCount })
    : t("home.tileShareEmpty");

  // Developer supervisor info
  document.getElementById("dash-heimdall").textContent = data.heimdall_connected
    ? `Heimdall online (PID ${data.heimdall_health?.daemon_pid ?? "?"}) · ${data.heimdall_health?.pipe_name ?? ""}`
    : "Heimdall offline";
  const heimdallBtn = document.getElementById("btn-heimdall-toggle");
  if (heimdallBtn) {
    heimdallBtn.textContent = data.heimdall_connected ? "Stop Heimdall" : "Start Heimdall";
  }
  document.getElementById("dev-adb-summary").textContent =
    `Endpoint: ${data.config.network.adb_bind_address}:${data.config.network.adb_port} · ADB devices online: ${data.devices.length}`;

  const defaultManifest = data.runtime.manifest_path || data.active_manifest_path;
  const manifestInput = document.getElementById("manifest-input");
  if (defaultManifest && manifestInput && !manifestInput.value) {
    manifestInput.value = defaultManifest;
  }

  populateConfigControls(data.config, data.doctor);
  renderDoctor(data.doctor);
  renderSettings(data);
}

export function populateConfigControls(cfg, doctor) {
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
  document.getElementById("cfg-auto-sync-shortcuts").checked = cfg.desktop?.auto_sync_shortcuts ?? true;
  document.getElementById("cfg-forward-notifications").checked = cfg.desktop?.forward_notifications ?? true;
  document.getElementById("cfg-auto-sync-clipboard").checked = cfg.desktop?.auto_sync_clipboard ?? true;
  document.getElementById("cfg-close-to-tray").checked = cfg.desktop?.close_to_tray ?? true;

  if (doctor?.snapshot) {
    document.getElementById("settings-host-limits").textContent = t("settings.hostLimits", {
      cores: doctor.snapshot.cpu.logical_cores,
      ram: formatGib(doctor.snapshot.memory.total_bytes),
    });
  }
}

export function renderAppsList() {
  const container = document.getElementById("apps-table-container");
  if (!container) return;

  const query = (document.getElementById("apps-search-input")?.value || "").trim().toLowerCase();
  const shortcutMap = new Map(state.lastShortcuts.map((s) => [s.package_name, s.display_name]));

  if (!state.lastApps.length) {
    container.innerHTML = `<div class="setting-row"><span class="muted">${escapeHtml(t("apps.emptyNone"))}</span></div>`;
    return;
  }

  const filtered = state.lastApps.filter((app) => {
    if (!query) return true;
    const title = friendlyAppName(app.package_name, shortcutMap.get(app.package_name)).toLowerCase();
    return title.includes(query) || app.package_name.toLowerCase().includes(query);
  });

  if (!filtered.length) {
    container.innerHTML = `<div class="setting-row"><span class="muted">${escapeHtml(t("apps.emptySearch"))}</span></div>`;
    return;
  }

  container.innerHTML = filtered
    .map((app) => {
      const pinnedTitle = shortcutMap.get(app.package_name);
      const isPinned = shortcutMap.has(app.package_name);
      const title = friendlyAppName(app.package_name, pinnedTitle);
      const initial = (title[0] || "A").toUpperCase();
      const subtitleParts = [
        app.is_system ? t("apps.systemApp") : t("apps.userApp"),
        isPinned ? t("apps.pinnedTag") : null,
        state.devMode ? app.package_name : null,
      ].filter(Boolean);

      return `
      <div class="setting-row wrap-row">
        <div class="setting-left">
          <div class="app-avatar">${escapeHtml(initial)}</div>
          <div class="setting-text">
            <strong>${escapeHtml(title)}</strong>
            <span class="muted small-text">${escapeHtml(subtitleParts.join(" · "))}</span>
          </div>
        </div>
        <div class="actions">
          <button class="btn small primary" data-app-window="${escapeHtml(app.package_name)}">${escapeHtml(t("apps.openWindow"))}</button>
          <button class="btn small" data-app-shortcut="${escapeHtml(app.package_name)}" data-pinned="${isPinned}">${escapeHtml(isPinned ? t("apps.unpin") : t("apps.pin"))}</button>
          ${
            state.devMode
              ? `<button class="btn small" data-app-info="${escapeHtml(app.package_name)}">${escapeHtml(t("apps.info"))}</button>`
              : ""
          }
          <button class="btn small" data-app-stop="${escapeHtml(app.package_name)}">${escapeHtml(t("apps.stop"))}</button>
          ${
            !app.is_system || state.devMode
              ? `<button class="btn small danger" data-app-uninstall="${escapeHtml(app.package_name)}">${escapeHtml(t("apps.uninstall"))}</button>`
              : ""
          }
        </div>
      </div>`;
    })
    .join("");

  container.querySelectorAll("[data-app-window]").forEach((btn) =>
    btn.addEventListener("click", async () => {
      const pkg = btn.dataset.appWindow;
      btn.disabled = true;
      try {
        const handle = await invoke("launch_app_window", {
          package: pkg,
          width: null,
          height: null,
          dpi: null,
        });
        showToast(`${handle.window_title}`);
      } catch (err) {
        showToast(String(err), true);
      } finally {
        btn.disabled = false;
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
        } else {
          await invoke("create_app_shortcut", { package: pkg, name: null, apkPath: null });
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
        showToast(`${btn.dataset.appStop}`);
      } catch (err) {
        showToast(String(err), true);
      }
    })
  );

  container.querySelectorAll("[data-app-uninstall]").forEach((btn) =>
    btn.addEventListener("click", async () => {
      const pkg = btn.dataset.appUninstall;
      if (!window.confirm(t("apps.confirmUninstall", { pkg }))) return;
      try {
        await invoke("uninstall_app", { package: pkg, keepData: false });
        await refreshApps();
      } catch (err) {
        showToast(String(err), true);
      }
    })
  );
}

export function renderDoctor(report) {
  const container = document.getElementById("diag-checks");
  if (!container || !report) return;
  container.innerHTML = report.checks
    .map(
      (c) => `
      <div class="item-row">
        <div>
          <strong>${escapeHtml(c.name)}</strong> — ${escapeHtml(c.summary)}
          <div class="muted small-text">${escapeHtml(c.detail)}</div>
          ${c.remediation ? `<div class="small-text">${escapeHtml(c.remediation)}</div>` : ""}
        </div>
        <span class="badge ${escapeHtml(c.status)}">${escapeHtml(c.status)}</span>
      </div>`
    )
    .join("");
}

export function renderSettings(data) {
  document.getElementById("settings-version-title").textContent = `BIFRÖST v${data.version.version}`;
  document.getElementById("settings-share-count").textContent = data.shared_file_count
    ? t("settings.shareCount", { count: data.shared_file_count })
    : t("settings.shareEmpty");

  const autostartToggle = document.getElementById("toggle-heimdall-autostart");
  const autostartStatus = document.getElementById("settings-autostart-status");
  if (autostartToggle && autostartStatus) {
    autostartToggle.checked = Boolean(data.heimdall_autostart);
    autostartStatus.textContent = data.heimdall_autostart
      ? t("settings.autostartOn")
      : t("settings.autostartOff");
  }

  const pathsEl = document.getElementById("settings-paths");
  if (pathsEl) {
    pathsEl.innerHTML = `
      <dt>Config (bifrost.toml)</dt><dd>${escapeHtml(data.config_path)}</dd>
      <dt>Data directory</dt><dd>${escapeHtml(data.paths.data_dir)}</dd>
      <dt>Images directory</dt><dd>${escapeHtml(data.paths.images_dir)}</dd>
      <dt>Logs directory</dt><dd>${escapeHtml(data.paths.logs_dir)}</dd>
    `;
  }

  const capsEl = document.getElementById("settings-capabilities");
  if (capsEl) {
    capsEl.innerHTML = data.version.capabilities
      .map(
        (cap) => `
        <div class="item-row">
          <div>
            <strong>${escapeHtml(cap.name)}</strong>
            <div class="muted small-text">${escapeHtml(cap.summary)}</div>
          </div>
          <span class="badge ${escapeHtml(cap.status)}">${escapeHtml(cap.status.replaceAll("_", " "))}</span>
        </div>`
      )
      .join("");
  }
}
