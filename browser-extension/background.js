// Inspired by GenericAgent's GA Web / TMWebDriver real-browser bridge.
// Independent Wisp implementation; attribution: https://github.com/lsdefine/GenericAgent
// GenericAgent is MIT-licensed, Copyright (c) 2025 lsdefine. See NOTICE.md.

const BRIDGE_URL = "ws://127.0.0.1:18765";
const RECONNECT_ALARM = "wisp-browser-reconnect";

let socket = null;
let keepAliveTimer = null;
let lastError = "";

const isScriptable = (url) => /^https?:/i.test(url || "");

async function browserTabs() {
  return (await chrome.tabs.query({}))
    .filter((tab) => isScriptable(tab.url))
    .map((tab) => ({
      id: tab.id,
      url: tab.url || "",
      title: tab.title || "",
      active: !!tab.active,
      windowId: tab.windowId,
    }));
}

function send(message) {
  if (socket?.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify(message));
  }
}

async function sendTabs(type = "tabs_update") {
  send({ type, tabs: await browserTabs() });
}

function startKeepAlive() {
  clearInterval(keepAliveTimer);
  keepAliveTimer = setInterval(() => {
    if (socket?.readyState === WebSocket.OPEN) {
      send({ type: "ping" });
    }
  }, 20_000);
}

function connect() {
  if (socket && socket.readyState <= WebSocket.OPEN) return;
  try {
    socket = new WebSocket(BRIDGE_URL);
  } catch (error) {
    lastError = error.message;
    socket = null;
    return;
  }
  socket.onopen = async () => {
    lastError = "";
    startKeepAlive();
    await sendTabs("ext_ready");
  };
  socket.onmessage = async (event) => {
    try {
      const request = JSON.parse(event.data);
      if (request.id && request.code !== undefined) await handleRequest(request);
    } catch (error) {
      lastError = error.message;
    }
  };
  socket.onerror = () => {
    lastError = "Cannot connect to Wisp on 127.0.0.1:18765";
  };
  socket.onclose = () => {
    clearInterval(keepAliveTimer);
    keepAliveTimer = null;
    socket = null;
  };
}

async function runPageCode(code) {
  const clean = (value) => {
    if (value === undefined) return null;
    if (value === window) return `[Window: ${location.href}]`;
    if (value === document) return "[Document]";
    if (value instanceof Element) return value.outerHTML;
    if (value instanceof NodeList || value instanceof HTMLCollection) {
      return [...value].slice(0, 200).map((node) =>
        node instanceof Element ? node.outerHTML : String(node)
      );
    }
    try {
      const seen = new WeakSet();
      return JSON.parse(JSON.stringify(value, (_key, item) => {
        if (item instanceof Element) return item.outerHTML;
        if (typeof item === "object" && item !== null) {
          if (seen.has(item)) return "[Circular]";
          seen.add(item);
        }
        return item;
      }));
    } catch (error) {
      return `[Unserializable: ${error.message}]`;
    }
  };
  try {
    let value;
    try {
      value = (0, eval)(code);
      if (value instanceof Promise) value = await value;
    } catch (error) {
      if (!(error instanceof SyntaxError)) throw error;
      const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
      value = await new AsyncFunction(code)();
    }
    return { ok: true, data: clean(value) };
  } catch (error) {
    const message = error.message || String(error);
    return {
      ok: false,
      error: { name: error.name || "Error", message, stack: error.stack || "" },
      csp: /content security policy|unsafe-eval|refused to evaluate/i.test(message),
    };
  }
}

function cdpExpression(code) {
  return `(async () => {
    const code = ${JSON.stringify(code)};
    const clean = (value) => {
      if (value === undefined) return null;
      if (value instanceof Element) return value.outerHTML;
      if (value instanceof NodeList || value instanceof HTMLCollection) return [...value].slice(0, 200).map(x => x instanceof Element ? x.outerHTML : String(x));
      try { const seen = new WeakSet(); return JSON.parse(JSON.stringify(value, (_k, x) => { if (x instanceof Element) return x.outerHTML; if (typeof x === 'object' && x !== null) { if (seen.has(x)) return '[Circular]'; seen.add(x); } return x; })); }
      catch (e) { return '[Unserializable: ' + e.message + ']'; }
    };
    try {
      let value;
      try { value = (0, eval)(code); if (value instanceof Promise) value = await value; }
      catch (e) { if (!(e instanceof SyntaxError)) throw e; const AsyncFunction = Object.getPrototypeOf(async function(){}).constructor; value = await new AsyncFunction(code)(); }
      return { ok: true, data: clean(value) };
    } catch (e) { return { ok: false, error: { name: e.name || 'Error', message: e.message || String(e), stack: e.stack || '' } }; }
  })()`;
}

async function runWithCdp(tabId, method, params = {}) {
  await chrome.debugger.attach({ tabId }, "1.3");
  try {
    return await chrome.debugger.sendCommand({ tabId }, method, params);
  } finally {
    await chrome.debugger.detach({ tabId }).catch(() => {});
  }
}

async function executeJavaScript(tabId, code) {
  let result;
  try {
    const injected = await chrome.scripting.executeScript({
      target: { tabId },
      world: "MAIN",
      func: runPageCode,
      args: [code],
    });
    result = injected[0]?.result;
  } catch (error) {
    result = {
      ok: false,
      error: { name: error.name || "Error", message: error.message || String(error) },
      csp: true,
    };
  }
  if (result?.ok) return result.data;
  if (!result?.csp) throw result?.error || new Error("Page execution failed");

  const cdp = await runWithCdp(tabId, "Runtime.evaluate", {
    expression: cdpExpression(code),
    awaitPromise: true,
    returnByValue: true,
  });
  if (cdp.exceptionDetails) {
    throw new Error(cdp.exceptionDetails.exception?.description || "CDP execution failed");
  }
  const value = cdp.result?.value;
  if (!value?.ok) throw value?.error || new Error("CDP execution failed");
  return value.data;
}

async function runCommand(tabId, command) {
  switch (command.cmd) {
    case "cdp":
      return await runWithCdp(command.tabId || tabId, command.method, command.params || {});
    case "tabs": {
      if (command.method === "create") {
        const tab = await chrome.tabs.create({
          url: command.url,
          active: command.active ?? false,
        });
        return { id: tab.id, url: tab.url, title: tab.title };
      }
      if (command.method === "switch") {
        const tab = await chrome.tabs.update(command.tabId || tabId, { active: true });
        await chrome.windows.update(tab.windowId, { focused: true });
        return { ok: true };
      }
      return await browserTabs();
    }
    default:
      throw new Error(`Unknown browser command: ${command.cmd}`);
  }
}

async function handleRequest(request) {
  const created = new Set();
  const onCreated = (tab) => created.add(tab.id);
  chrome.tabs.onCreated.addListener(onCreated);
  try {
    let command = request.code;
    if (typeof command === "string") {
      try {
        const parsed = JSON.parse(command);
        if (parsed && typeof parsed === "object" && parsed.cmd) command = parsed;
      } catch (_) {}
    }
    const result = typeof command === "string"
      ? await executeJavaScript(request.tabId, command)
      : await runCommand(request.tabId, command);
    await new Promise((resolve) => setTimeout(resolve, 150));
    const newTabs = [];
    for (const id of created) {
      try {
        const tab = await chrome.tabs.get(id);
        newTabs.push({ id: tab.id, url: tab.url, title: tab.title });
      } catch (_) {}
    }
    send({ type: "result", id: request.id, result, newTabs });
  } catch (error) {
    send({
      type: "error",
      id: request.id,
      error: error?.message ? { name: error.name || "Error", message: error.message, stack: error.stack || "" } : error,
    });
  } finally {
    chrome.tabs.onCreated.removeListener(onCreated);
  }
}

chrome.alarms.create(RECONNECT_ALARM, { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === RECONNECT_ALARM) connect();
});
chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onInstalled.addListener(connect);
chrome.tabs.onCreated.addListener(() => sendTabs());
chrome.tabs.onRemoved.addListener(() => sendTabs());
chrome.tabs.onActivated.addListener(() => sendTabs());
chrome.tabs.onUpdated.addListener((_id, change) => {
  if (change.status === "complete" || change.url) sendTabs();
});
chrome.runtime.onMessage.addListener((message, _sender, reply) => {
  if (message?.type === "wisp_bridge_status") {
    reply({
      connected: socket?.readyState === WebSocket.OPEN,
      endpoint: BRIDGE_URL,
      error: lastError,
    });
  } else if (message?.type === "wisp_bridge_connect") {
    connect();
    reply({ ok: true });
  }
});

connect();
