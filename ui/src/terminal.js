import { Terminal } from "/vendor/xterm.mjs";
import { FitAddon } from "/vendor/xterm-addon-fit.mjs";

const controllers = new Map();

function cssColor(name, fallback) {
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

function currentTheme() {
  return {
    background: cssColor("--bg-app", "#151614"),
    foreground: cssColor("--text", "#ebe8e2"),
    cursor: cssColor("--clay", "#4cc5b5"),
    selectionBackground: cssColor("--surface-active", "#315d57"),
  };
}

function decodeBase64(value) {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function messageText(value) {
  if (value instanceof Error) return value.message;
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function createController(container, sessionId) {
  const core = window.__TAURI__?.core;
  const terminal = new Terminal({
    cursorBlink: true,
    cursorStyle: "bar",
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
    fontSize: 13,
    lineHeight: 1.2,
    scrollback: 10_000,
    allowProposedApi: false,
    theme: currentTheme(),
  });
  const fit = new FitAddon();
  terminal.loadAddon(fit);
  terminal.open(container);

  let disposed = false;
  let active = false;
  let fitFrame;
  let resizeTimer;
  let pendingInput = "";
  let inputFlushScheduled = false;
  let inputChain = Promise.resolve();

  const showError = (value) => {
    const message = messageText(value);
    container.dataset.terminalError = message;
    terminal.write(`\r\n\x1b[31m[terminal error] ${message}\x1b[0m\r\n`);
  };

  const scheduleFit = (focus = false) => {
    cancelAnimationFrame(fitFrame);
    fitFrame = requestAnimationFrame(() => {
      fitFrame = requestAnimationFrame(() => {
        if (disposed || container.clientWidth === 0 || container.clientHeight === 0) return;
        try {
          fit.fit();
          if (focus && active) terminal.focus();
        } catch (error) {
          showError(error);
        }
      });
    });
  };

  const queueInput = (data) => {
    pendingInput += data;
    if (inputFlushScheduled) return;
    inputFlushScheduled = true;
    queueMicrotask(() => {
      inputFlushScheduled = false;
      const data = pendingInput;
      pendingInput = "";
      inputChain = inputChain
        .then(() => core.invoke("write_terminal", { sessionId, data }))
        .catch(showError);
    });
  };

  const resizePty = ({ rows, cols }) => {
    if (!rows || !cols || disposed) return;
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => {
      core.invoke("resize_terminal", { sessionId, rows, cols }).catch(showError);
    }, 30);
  };

  const inputDisposable = terminal.onData(queueInput);
  const resizeDisposable = terminal.onResize(resizePty);
  const resizeObserver = new ResizeObserver(() => scheduleFit(active));
  resizeObserver.observe(container);
  const onPointerDown = () => terminal.focus();
  container.addEventListener("pointerdown", onPointerDown);

  let channel;
  if (!core) {
    showError("Terminal session bridge is unavailable.");
  } else {
    channel = new core.Channel();
    channel.onmessage = (message) => {
      if (disposed) return;
      if (message.event === "output") {
        terminal.write(decodeBase64(message.data.base64));
      } else if (message.event === "exit") {
        terminal.write(`\r\n\x1b[90m[process exited with code ${message.data.exitCode}]\x1b[0m\r\n`);
      } else if (message.event === "error") {
        showError(message.data.message);
      }
    };
    core.invoke("attach_terminal", { sessionId, onEvent: channel })
      .then(() => {
        delete container.dataset.terminalError;
        scheduleFit(active);
      })
      .catch(showError);
  }

  return {
    setActive(value) {
      active = value;
      if (active) scheduleFit(true);
    },
    refreshTheme() {
      terminal.options.theme = currentTheme();
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      cancelAnimationFrame(fitFrame);
      clearTimeout(resizeTimer);
      resizeObserver.disconnect();
      container.removeEventListener("pointerdown", onPointerDown);
      inputDisposable.dispose();
      resizeDisposable.dispose();
      channel?.cleanupCallback?.();
      terminal.dispose();
    },
  };
}

export function mount_terminal(elementId, sessionId) {
  if (controllers.has(elementId)) return;
  const container = document.getElementById(elementId);
  if (!container) return;
  controllers.set(elementId, createController(container, sessionId));
}

export function set_terminal_active(elementId, active) {
  controllers.get(elementId)?.setActive(active);
}

export function unmount_terminal(elementId) {
  const controller = controllers.get(elementId);
  if (!controller) return;
  controllers.delete(elementId);
  controller.dispose();
}

new MutationObserver(() => {
  for (const controller of controllers.values()) controller.refreshTheme();
}).observe(document.documentElement, {
  attributes: true,
  attributeFilter: ["class", "data-theme", "data-light-palette", "data-dark-palette", "style"],
});
