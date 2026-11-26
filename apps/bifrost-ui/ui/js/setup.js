// First-run setup wizard: checklist from the Rust setup plan, visible winget installs,
// and the first visible Android boot (polled until ADB access is ready).

import { escapeHtml, invoke, showToast } from "./core.js";
import { activateSection } from "./views.js";

const DISMISS_KEY = "bifrost-setup-dismissed";
const POLL_MS = 5000;
const POLL_LIMIT_MS = 20 * 60 * 1000;
const STATUS_LABEL = { done: "Done", pending: "To do", blocked: "Waiting" };
let pollTimer = null;
let busy = false;

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
    // Storage unavailable: the wizard simply opens again next time.
  }
}

function actionButton(step) {
  if (step.status !== "pending") return "";
  switch (step.action.kind) {
    case "winget":
      return `<button class="btn primary small" data-setup-install="${escapeHtml(step.id)}" type="button">Install with winget</button>`;
    case "register_image":
      return `<button class="btn small" data-setup-goto="images" type="button">Register an image</button>`;
    case "first_boot":
      return `<button class="btn primary small" data-setup-boot type="button">Boot Android with a window</button>`;
    default:
      return "";
  }
}

export function renderSetup(plan) {
  document.getElementById("setup-steps").innerHTML = plan.steps
    .map(
      (s) => `
      <div class="check-row setup-step ${escapeHtml(s.status)}">
        <span class="badge setup-${escapeHtml(s.status)}">${STATUS_LABEL[s.status] ?? escapeHtml(s.status)}</span>
        <div>
          <strong>${escapeHtml(s.title)}</strong>${s.required ? "" : ' <span class="muted">(optional)</span>'}
          <div class="muted small-text">${escapeHtml(s.detail)}</div>
          <div class="setup-step-action">${actionButton(s)}</div>
        </div>
      </div>`
    )
    .join("");
  document.getElementById("setup-summary").textContent = plan.complete
    ? "Setup is complete. Android apps are ready to run."
    : "Finish the steps below once. BIFRÖST never installs anything without asking.";
  document.getElementById("nav-setup").classList.toggle("needs-attention", !plan.complete);
}

export async function refreshSetup() {
  if (!invoke) return null;
  try {
    const plan = await invoke("get_setup_plan");
    renderSetup(plan);
    return plan;
  } catch (err) {
    showToast(`Could not check setup: ${err}`, true);
    return null;
  }
}

// Opens the wizard at start-up while required steps remain, unless the user hid it.
export async function maybeOpenSetup() {
  const plan = await refreshSetup();
  if (plan && !plan.complete && !readDismissed()) activateSection("setup");
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
      showToast("Setup complete: Android is ready.");
    }
  }, POLL_MS);
}

async function runBusy(button, label, task) {
  if (busy) return;
  busy = true;
  const original = button.textContent;
  button.disabled = true;
  button.textContent = label;
  try {
    await task();
  } finally {
    busy = false;
    button.disabled = false;
    button.textContent = original;
  }
}

async function installTool(button, stepId) {
  const ok = window.confirm(
    `BIFRÖST will open a winget window to install "${stepId}".\n\n` +
      "Review the license prompt there. Windows may ask for administrator approval."
  );
  if (!ok) return;
  await runBusy(button, "Installing…", async () => {
    try {
      renderSetup(await invoke("install_setup_tool", { stepId }));
      showToast(`Installed ${stepId}.`);
    } catch (err) {
      showToast(`${err}`, true);
    }
  });
}

async function firstBoot(button) {
  const ok = window.confirm(
    "Midgard will boot with a visible window (a running VM restarts).\n\n" +
      "In that window, finish Android's welcome screens and, when asked, allow USB " +
      'debugging with "Always allow from this computer".'
  );
  if (!ok) return;
  await runBusy(button, "Booting…", async () => {
    try {
      await invoke("start_first_boot");
      showToast("Midgard is booting. Finish Android's setup in its window.");
      pollUntilReady();
    } catch (err) {
      showToast(`Could not boot Midgard: ${err}`, true);
    }
  });
}

async function checkForUpdates(button) {
  const status = document.getElementById("update-status");
  await runBusy(button, "Checking…", async () => {
    try {
      const info = await invoke("check_for_updates");
      if (!info.update_available) {
        status.textContent = `BIFRÖST ${info.current_version} is up to date (latest release: ${info.latest_version}).`;
        return;
      }
      const bundle = info.bundle_asset ? ` (${info.bundle_asset})` : "";
      status.innerHTML =
        `BIFRÖST <strong>${escapeHtml(info.latest_version)}</strong> is available (installed: ${escapeHtml(info.current_version)}). ` +
        `Download the release bundle${escapeHtml(bundle)} from <code>${escapeHtml(info.release_url)}</code> and run ` +
        "<code>install-bifrost.ps1</code>; it verifies every file's SHA-256 and keeps your Android data.";
    } catch (err) {
      status.textContent = `Could not check for updates: ${err}`;
    }
  });
}

export function wireSetup() {
  document.getElementById("btn-check-updates").addEventListener("click", (event) =>
    checkForUpdates(event.currentTarget)
  );
  document.getElementById("setup-steps").addEventListener("click", (event) => {
    const target = event.target.closest("button");
    if (!target) return;
    if (target.dataset.setupInstall) installTool(target, target.dataset.setupInstall);
    else if (target.dataset.setupGoto) activateSection(target.dataset.setupGoto);
    else if ("setupBoot" in target.dataset) firstBoot(target);
  });
  document.getElementById("btn-setup-refresh").addEventListener("click", refreshSetup);
  document.getElementById("btn-setup-hide").addEventListener("click", () => {
    writeDismissed(true);
    activateSection("dashboard");
  });
  document.getElementById("nav-setup").addEventListener("click", () => {
    writeDismissed(false);
    refreshSetup();
  });
}
