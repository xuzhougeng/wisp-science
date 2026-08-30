// Inspired by GenericAgent's GA Web / TMWebDriver real-browser bridge.
// Independent Wisp implementation; attribution: https://github.com/lsdefine/GenericAgent
// GenericAgent is MIT-licensed, Copyright (c) 2025 lsdefine. See NOTICE.md.

importScripts(
  "session_config.js",
  "protocol.js",
  "wait_tab.js",
  "wait_engine.js",
  "tab_ops.js",
  "scan_page.js",
  "downloads.js",
  "capture.js",
  "chat_adapter.js"
);

var cfg = readBridgeConfig();
var BRIDGE_URL = cfg.endpoint || "ws://127.0.0.1:18765";
var SESSION = cfg.session || "shared";
var RECONNECT_ALARM = "wisp-browser-reconnect";
var DEFAULT_TIMEOUT_MS = 15000;
var WAIT_GUARD_MS = 500;
var tabWaiter = createTabWaiter(chrome);
var waitEngine = createWaitEngine(chrome);
var tabOps = createTabOps(chrome, new Set());
var scans = pageScanFunctions();

var socket = null;
var keepAliveTimer = null;
var lastError = "";
var paused = false;

chrome.storage.local.get(["wisp_paused"], function (stored) {
  paused = !!stored.wisp_paused;
});

var isScriptable = function (url) {
  return /^https?:/i.test(url || "");
};

async function browserTabs() {
  return (await chrome.tabs.query({}))
    .filter(function (tab) { return isScriptable(tab.url); })
    .map(function (tab) {
      return {
        id: tab.id,
        url: tab.url || "",
        title: tab.title || "",
        active: !!tab.active,
        windowId: tab.windowId,
        managed: tabOps.isManaged(tab.id)
      };
    });
}

function send(message) {
  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify(message));
  }
}

async function sendHandshake() {
  send(handshakePayload(await browserTabs(), paused));
}

function startKeepAlive() {
  clearInterval(keepAliveTimer);
  keepAliveTimer = setInterval(function () {
    if (socket && socket.readyState === WebSocket.OPEN) {
      send({ type: "ping", session: SESSION });
    }
  }, 20000);
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
  socket.onopen = async function () {
    lastError = "";
    startKeepAlive();
    await sendHandshake();
  };
  socket.onmessage = async function (event) {
    try {
      var request = JSON.parse(event.data);
      if (request.id) await handleRequest(request);
    } catch (error) {
      lastError = error.message;
    }
  };
  socket.onerror = function () {
    lastError = "Cannot connect to Wisp on " + BRIDGE_URL.replace("ws://", "");
  };
  socket.onclose = function () {
    clearInterval(keepAliveTimer);
    keepAliveTimer = null;
    socket = null;
  };
}

async function runPageCode(code) {
  var clean = function (value) {
    if (value === undefined) return null;
    if (value === window) return "[Window: " + location.href + "]";
    if (value === document) return "[Document]";
    if (value instanceof Element) return value.outerHTML;
    if (value instanceof NodeList || value instanceof HTMLCollection) {
      return Array.prototype.slice.call(value, 0, 200).map(function (node) {
        return node instanceof Element ? node.outerHTML : String(node);
      });
    }
    try {
      var seen = new WeakSet();
      return JSON.parse(JSON.stringify(value, function (_key, item) {
        if (item instanceof Element) return item.outerHTML;
        if (typeof item === "object" && item !== null) {
          if (seen.has(item)) return "[Circular]";
          seen.add(item);
        }
        return item;
      }));
    } catch (error) {
      return "[Unserializable: " + error.message + "]";
    }
  };
  try {
    var value;
    try {
      value = (0, eval)(code);
      if (value instanceof Promise) value = await value;
    } catch (error) {
      if (!(error instanceof SyntaxError)) throw error;
      var AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
      value = await new AsyncFunction(code)();
    }
    return { ok: true, data: clean(value) };
  } catch (error) {
    var message = error.message || String(error);
    return {
      ok: false,
      error: { name: error.name || "Error", message: message, stack: error.stack || "" },
      csp: /content security policy|unsafe-eval|refused to evaluate/i.test(message)
    };
  }
}

function cdpExpression(code) {
  return "(async () => {\n    const code = " + JSON.stringify(code) + ";\n    const clean = (value) => {\n      if (value === undefined) return null;\n      if (value instanceof Element) return value.outerHTML;\n      if (value instanceof NodeList || value instanceof HTMLCollection) return [...value].slice(0, 200).map(x => x instanceof Element ? x.outerHTML : String(x));\n      try { const seen = new WeakSet(); return JSON.parse(JSON.stringify(value, (_k, x) => { if (x instanceof Element) return x.outerHTML; if (typeof x === 'object' && x !== null) { if (seen.has(x)) return '[Circular]'; seen.add(x); } return x; })); }\n      catch (e) { return '[Unserializable: ' + e.message + ']'; }\n    };\n    try {\n      let value;\n      try { value = (0, eval)(code); if (value instanceof Promise) value = await value; }\n      catch (e) { if (!(e instanceof SyntaxError)) throw e; const AsyncFunction = Object.getPrototypeOf(async function(){}).constructor; value = await new AsyncFunction(code)(); }\n      return { ok: true, data: clean(value) };\n    } catch (e) { return { ok: false, error: { name: e.name || 'Error', message: e.message || String(e), stack: e.stack || '' } }; }\n  })()";
}

async function runWithCdp(tabId, method, params) {
  await chrome.debugger.attach({ tabId: tabId }, "1.3");
  try {
    return await chrome.debugger.sendCommand({ tabId: tabId }, method, params || {});
  } finally {
    await chrome.debugger.detach({ tabId: tabId }).catch(function () {});
  }
}

async function executeJavaScript(tabId, code) {
  var result;
  try {
    var injected = await chrome.scripting.executeScript({
      target: { tabId: tabId },
      world: "MAIN",
      func: runPageCode,
      args: [code]
    });
    result = injected[0] && injected[0].result;
  } catch (error) {
    result = {
      ok: false,
      error: { name: error.name || "Error", message: error.message || String(error) },
      csp: true
    };
  }
  if (result && result.ok) return result.data;
  if (!(result && result.csp)) throw (result && result.error) || new Error("Page execution failed");

  var cdp = await runWithCdp(tabId, "Runtime.evaluate", {
    expression: cdpExpression(code),
    awaitPromise: true,
    returnByValue: true
  });
  if (cdp.exceptionDetails) {
    throw new Error((cdp.exceptionDetails.exception && cdp.exceptionDetails.exception.description) || "CDP execution failed");
  }
  var value = cdp.result && cdp.result.value;
  if (!(value && value.ok)) throw (value && value.error) || new Error("CDP execution failed");
  return value.data;
}

async function runNamedScan(tabId, mode) {
  var fnName = mode === "article" ? "articleScan" : mode === "text" ? "textScan" : "defaultScan";
  await chrome.scripting.executeScript({
    target: { tabId: tabId },
    world: "ISOLATED",
    files: ["scan_page.js"]
  });
  var injected = await chrome.scripting.executeScript({
    target: { tabId: tabId },
    world: "ISOLATED",
    func: function (which) {
      var api = pageScanFunctions();
      return api[which]();
    },
    args: [fnName]
  });
  return injected[0] && injected[0].result;
}

function parseCommand(code) {
  if (typeof code !== "string") return code;
  try {
    var parsed = JSON.parse(code);
    if (parsed && typeof parsed === "object" && parsed.cmd) return parsed;
  } catch (_) {}
  return code;
}

function isTabCloseCommand(command) {
  return typeof command === "object" && command.cmd === "tabs" && command.method === "close";
}
function isTabCreateCommand(command) {
  return typeof command === "object" && command.cmd === "tabs" && command.method === "create";
}
function isNavigationCommand(command) {
  if (isTabCreateCommand(command)) return true;
  if (typeof command === "object" && command.cmd === "cdp") {
    return command.method === "Page.navigate" || command.method === "Target.createTarget";
  }
  return false;
}

function requestDeadline(request) {
  var timeoutMs = Number(request.timeoutMs);
  var budget = Number.isFinite(timeoutMs) && timeoutMs > 0 ? timeoutMs : DEFAULT_TIMEOUT_MS;
  return Date.now() + Math.max(0, budget - WAIT_GUARD_MS);
}

async function tabSnapshot(tabId) {
  try { return await chrome.tabs.get(tabId); } catch (_) { return null; }
}

async function runCommand(tabId, command) {
  switch (command.cmd) {
    case "cdp":
      return await runWithCdp(command.tabId || tabId, command.method, command.params || {});
    case "tabs": {
      if (command.method === "create") {
        return await tabOps.create(command.url, command.active);
      }
      if (command.method === "close") {
        var ids = (command.tabIds || [command.tabId || tabId]).filter(function (id) { return Number.isInteger(id); });
        var closed = await tabOps.close(ids);
        return { closed: closed, remaining: await browserTabs() };
      }
      if (command.method === "switch") {
        return await tabOps.switchTo(command.tabId || tabId);
      }
      return await browserTabs();
    }
    case "scan":
      return await runNamedScan(tabId, command.mode || "default");
    case "wait":
      return await waitEngine.waitFor(tabId, command.spec || command, requestDeadline({ timeoutMs: command.timeoutMs }));
    case "download":
      return await downloadAsset(command);
    case "capture":
      return await capturePage(tabId, command);
    case "control":
      if (command.method === "pause") {
        paused = !!command.paused;
        await chrome.storage.local.set({ wisp_paused: paused });
        return { paused: paused };
      }
      return { paused: paused };
    case "runtime":
      if (command.method === "reload") {
        // Return the acknowledgement first; reloading the service worker
        // immediately would tear down the socket before Wisp receives it.
        setTimeout(function () { chrome.runtime.reload(); }, 250);
        return { reloading: true };
      }
      throw new Error("Unknown runtime method: " + command.method);
    default:
      throw new Error("Unknown browser command: " + command.cmd);
  }
}

async function runChatSite(tabId, method, payload) {
  await chrome.scripting.executeScript({
    target: { tabId: tabId },
    world: "MAIN",
    files: ["chat_adapter.js"]
  });
  var injected = await chrome.scripting.executeScript({
    target: { tabId: tabId },
    world: "MAIN",
    func: function (which, prompt) {
      var api = chatSiteDomFns();
      if (which === "fill") return api.fill(prompt);
      if (which === "send") return api.send();
      if (which === "read") return api.read();
      return api.ready();
    },
    args: [method, (payload && payload.prompt) || ""]
  });
  return injected[0] && injected[0].result;
}

function isControlCommand(command) {
  return typeof command === "object" && command !== null && command.cmd === "control";
}

async function handleRequest(request) {
  var parsed = parseIncomingRequest(request);
  var command;
  if (parsed.kind === "v2") {
    if (parsed.op === "eval") command = parsed.payload && parsed.payload.code != null ? parsed.payload.code : parsed.payload;
    else command = Object.assign({ cmd: parsed.op }, parsed.payload || {});
    if (parsed.wait) command.wait = parsed.wait;
  } else {
    command = parseCommand(request.code);
  }

  if (paused && !isControlCommand(command)) {
    send({
      type: "error",
      id: request.id,
      error: { code: "USER_CONTROLLING", message: "user paused browser control from the extension popup", retryable: false }
    });
    return;
  }

  var created = new Set();
  var onCreated = function (tab) { created.add(tab.id); };
  chrome.tabs.onCreated.addListener(onCreated);
  var deadline = requestDeadline(request);
  var waitMeta = null;
  try {
    if (request.tabId && !isTabCloseCommand(command) && !(typeof command === "object" && command.cmd === "wait")) {
      var spec = (typeof command === "object" && command.wait) || { until: "complete" };
      waitMeta = await waitEngine.waitFor(request.tabId, spec, deadline);
    }

    var result;
    if (typeof command === "object" && command && (command.cmd === "chat" || command.cmd === "chatgpt")) {
      result = await runChatSite(request.tabId, command.method || "ready", command);
    } else if (typeof command === "string") {
      result = await executeJavaScript(request.tabId, command);
    } else {
      result = await runCommand(request.tabId, command);
    }

    var afterTargets = new Set(created);
    if (request.tabId) {
      var after = await tabSnapshot(request.tabId);
      if ((after && after.status === "loading") || isNavigationCommand(command)) {
        afterTargets.add(request.tabId);
      }
    }
    for (var id of afterTargets) {
      waitMeta = await waitEngine.waitFor(id, { until: "complete" }, deadline);
    }

    if (isTabCreateCommand(command) && result && result.id) {
      var tab = await tabSnapshot(result.id);
      if (tab) {
        waitMeta = await waitEngine.waitFor(result.id, {
          until: "complete",
          settle_ms: 250
        }, deadline);
        var readyTab = await tabSnapshot(result.id);
        if (readyTab) result = tabOps.publicTab(readyTab);
      }
    }

    var newTabs = [];
    for (var createdId of created) {
      var createdTab = await tabSnapshot(createdId);
      if (createdTab) newTabs.push({ id: createdTab.id, url: createdTab.url, title: createdTab.title });
    }
    send({
      type: "result",
      id: request.id,
      result: result,
      newTabs: newTabs,
      session: SESSION,
      ready: waitMeta ? waitMeta.ready : true,
      wait: waitMeta ? waitMeta.wait : { until: "complete", waited_ms: 0 }
    });
  } catch (error) {
    send({
      type: "error",
      id: request.id,
      error: error && error.message
        ? { name: error.name || "Error", message: error.message, stack: error.stack || "", code: error.code, retryable: !!error.retryable }
        : error
    });
  } finally {
    chrome.tabs.onCreated.removeListener(onCreated);
  }
}

chrome.alarms.create(RECONNECT_ALARM, { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener(function (alarm) {
  if (alarm.name === RECONNECT_ALARM) connect();
});
chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onInstalled.addListener(connect);
chrome.tabs.onCreated.addListener(function () { sendHandshake(); });
chrome.tabs.onRemoved.addListener(function () { sendHandshake(); });
chrome.tabs.onActivated.addListener(function () { sendHandshake(); });
chrome.tabs.onUpdated.addListener(function (_id, change) {
  if (change.status === "complete" || change.url) sendHandshake();
});
chrome.runtime.onMessage.addListener(function (message, _sender, reply) {
  if (message && message.type === "wisp_bridge_status") {
    browserTabs().then(function (tabs) {
      reply({
        connected: !!(socket && socket.readyState === WebSocket.OPEN),
        endpoint: BRIDGE_URL,
        session: SESSION,
        paused: paused,
        extension_version: WISP_PROTOCOL.extensionVersion,
        protocol_version: WISP_PROTOCOL.version,
        error: lastError,
        tabs: tabs.slice(0, 12)
      });
    }).catch(function () {
      reply({
        connected: !!(socket && socket.readyState === WebSocket.OPEN),
        endpoint: BRIDGE_URL,
        session: SESSION,
        paused: paused,
        extension_version: WISP_PROTOCOL.extensionVersion,
        protocol_version: WISP_PROTOCOL.version,
        error: lastError,
        tabs: []
      });
    });
    return true;
  } else if (message && message.type === "wisp_bridge_connect") {
    connect();
    reply({ ok: true });
  } else if (message && message.type === "wisp_bridge_pause") {
    paused = !!message.paused;
    chrome.storage.local.set({ wisp_paused: paused });
    sendHandshake();
    reply({ ok: true, paused: paused });
  } else if (message && message.type === "wisp_bridge_release") {
    tabOps.releaseAll();
    sendHandshake();
    reply({ ok: true, released: true });
  } else if (message && message.type === "wisp_bridge_disconnect") {
    if (socket) {
      try { socket.close(); } catch (_) {}
    }
    reply({ ok: true, disconnected: true });
  }
});

connect();
