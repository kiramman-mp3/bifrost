// First-run setup checklist: only shown in the sidebar when required steps remain (or opened from Developer).

import { escapeHtml, invoke, showToast, state } from "./core.js";
import { t } from "./i18n.js";
import { activateSection, applyDeveloperMode } from "./views.js";

const DISMISS_KEY = "bifrost-setup-dismissed";
const POLL_MS = 5000;
const POLL_LIMIT_MS = 20 * 60 * 1000;
let pollTimer = null;
let busy = false;
let forceVisible = false;

function readDismissed() {
  try {
    return localStorage.getItem(DISMISS_KEY) === "1";
  } catch {
    return false;
  }
}

function writeDismissed(value) {
  try {
    if (value) localStorage.setItem(DISMISS_KEY, "1");
    else localStorage.removeItem(DISMISS_KEY);
  } catch {
    // Ignore storage errors.
  }
}

function actionButton(step) {
  if (step.status !== "pending") return "";
  switch (step.action.kind) {
    case "winget":
      return `<button class="btn primary small" data-setup-install="${escapeHtml(step.id)}" type="button">${escapeHtml(t("setup.installWinget"))}</button>`;
    case "register_image":
      return `<button class="btn small" data-setup-goto="developer" type="button">${escapeHtml(t("setup.registerImg"))}</button>`;
    case "first_boot":
      return `<button class="btn primary small" data-setup-boot type="button">${escapeHtml(t("setup.firstBoot"))}</button>`;
    default:
      return "";
  }
}

export function renderSetup(plan) {
  if (!plan) return;
  state.lastSetupPlan = plan;

  const statusText = {
    done: t("setup.done"),
    pending: t("setup.pending"),
    blocked: t("setup.blocked"),
  };

  document.getElementById("setup-steps").innerHTML = plan.steps
    .map(
      (s) => `
      <div class="setting-row wrap-row">
        <div class="setting-left">
          <div class="setting-text">
            <strong>${escapeHtml(s.title)} ${s.required ? "" : `<span class="muted small-text">${escapeHtml(t("setup.optional"))}</span>`}</strong>
            <span class="muted small-text">${escapeHtml(s.detail)}</span>
          </div>
        </div>
        <div class="actions">
          ${actionButton(s)}
          <span class="badge setup-${escapeHtml(s.status)}">${escapeHtml(statusText[s.status] ?? s.status)}</span>
        </div>
      </div>`
    )
    .join("");

  document.getElementById("setup-summary").textContent = plan.complete
    ? t("setup.complete")
    : t("setup.incomplete");

  const navSetup = document.getElementById("nav-setup");
  if (navSetup) {
    const showNav = !plan.complete || forceVisible;
    navSetup.classList.toggle("hidden", !showNav);
    navSetup.classList.toggle("needs-attention", !plan.complete);
  }
}

export async function refreshSetup() {
  if (!invoke) return null;
  try {
    const plan = await invoke("get_setup_plan");
    renderSetup(plan);
    return plan;
  } catch (err) {
    showToast(`${err}`, true);
    return null;
  }
}

export async function openSetupManually() {
  forceVisible = true;
  await refreshSetup();
  activateSection("setup");
}

export async function maybeOpenSetup() {
  const plan = await refreshSetup();
  if (plan && !plan.complete && !readDismissed()) {
    activateSection("setup");
  }
}

function stopPolling() {
  if (pollTimer) clearInterval(pollTimer);
  pollTimer = null;
}

function pollUntilReady() {
  stopPolling();
  const startedAt = Date.now();
  pollTimer = setInterval(async () => {
    if (Date.now() - startedAt > POLL_LIMIT_MS) {
      stopPolling();
      return;
    }
    const plan = await refreshSetup();
    if (plan?.complete) {
      stopPolling();
      showToast(t("setup.complete"));
    }
  }, POLL_MS);
}

async function runBusy(button, task) {
  if (busy) return;
  busy = true;
  const original = button.textContent;
  button.disabled = true;
  button.textContent = t("common.working");
  try {
    await task();
  } finally {
    busy = false;
    button.disabled = false;
    button.textContent = original;
  }
}

async function installTool(button, stepId) {
  if (!window.confirm(t("setup.confirmWinget", { step: stepId }))) return;
  await runBusy(button, async () => {
    try {
      renderSetup(await invoke("install_setup_tool", { stepId }));
    } catch (err) {
      showToast(`${err}`, true);
    }
  });
}

async function firstBoot(button) {
  if (!window.confirm(t("setup.confirmBoot"))) return;
  await runBusy(button, async () => {
    try {
      await invoke("start_first_boot");
      pollUntilReady();
    } catch (err) {
      showToast(`${err}`, true);
    }
  });
}

export async function checkForUpdates(button) {
  const status = document.getElementById("update-status");
  await runBusy(button, async () => {
    try {
      const info = await invoke("check_for_updates");
      status.textContent = info.update_available
        ? t("settings.updateAvailable", {
            latest: info.latest_version,
            current: info.current_version,
            url: info.release_url,
          })
        : t("settings.upToDate", { current: info.current_version });
    } catch (err) {
      status.textContent = `${err}`;
    }
  });
}

export function wireSetup() {
  document.getElementById("btn-check-updates")?.addEventListener("click", (e) =>
    checkForUpdates(e.currentTarget)
  );
  document.getElementById("setup-steps")?.addEventListener("click", (event) => {
    const target = event.target.closest("button");
    if (!target) return;
    if (target.dataset.setupInstall) {
      installTool(target, target.dataset.setupInstall);
    } else if (target.dataset.setupGoto) {
      state.devMode = true;
      applyDeveloperMode(true);
      activateSection(target.dataset.setupGoto);
    } else if ("setupBoot" in target.dataset) {
      firstBoot(target);
    }
  });
  document.getElementById("btn-setup-refresh")?.addEventListener("click", refreshSetup);
  document.getElementById("btn-setup-hide")?.addEventListener("click", () => {
    writeDismissed(true);
    forceVisible = false;
    if (state.lastSetupPlan?.complete) {
      document.getElementById("nav-setup")?.classList.add("hidden");
    }
    activateSection("home");
  });
  document.getElementById("nav-setup")?.addEventListener("click", () => {
    writeDismissed(false);
    refreshSetup();
  });
}
