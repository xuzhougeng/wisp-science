// Side panel relay. A question about the tab the user is looking at travels
// over the bridge socket to Wisp's agent loop; the turn's events come back the
// same way and are fanned out to every open side panel.

var SIDE_SELECTION_MAX = 20000;

function isSideMessage(message) {
  return !!message && typeof message.type === "string" && message.type.indexOf("side_") === 0;
}

function sideAskPayload(ask, tab, scan, selection) {
  var url = (tab && tab.url) || "";
  var page = { url: url, title: (scan && scan.title) || (tab && tab.title) || "", tab_id: tab && tab.id };
  // PDF viewers expose no text; Wisp then reads the URL with its own tools.
  if (!(scan && scan.text)) page.unreadable = true;
  // After the first question about a URL its text is already in the conversation.
  else if (ask.sentUrl !== url) page.text = scan.text;
  if (selection && String(selection).trim()) page.selection = String(selection).slice(0, SIDE_SELECTION_MAX);
  return {
    type: "side_ask",
    id: ask.id,
    session_id: ask.sessionId || null,
    question: String(ask.question || "").trim(),
    page: page
  };
}

function createSideRelay(chrome, deps) {
  var ports = new Set();

  var post = function (port, message) {
    try { port.postMessage(message); } catch (_) {}
  };

  var broadcast = function (message) {
    ports.forEach(function (port) { post(port, message); });
  };

  async function readTab(tabId) {
    try {
      await chrome.scripting.executeScript({
        target: { tabId: tabId },
        world: "ISOLATED",
        files: ["scan_page.js"]
      });
      var injected = await chrome.scripting.executeScript({
        target: { tabId: tabId },
        world: "ISOLATED",
        func: function () {
          return { scan: pageScanFunctions().textScan(), selection: String(window.getSelection() || "") };
        }
      });
      return (injected[0] && injected[0].result) || {};
    } catch (_) {
      // PDF viewers and browser pages refuse injection. Wisp still gets the
      // URL and reads the document with its own tools.
      return {};
    }
  }

  async function ask(port, message) {
    var tab = (await chrome.tabs.query({ active: true, windowId: message.windowId }))[0];
    if (!tab || !deps.isScriptable(tab.url)) {
      post(port, { type: "side_error", id: message.id, code: "NO_PAGE" });
      return;
    }
    if (!(await deps.ready())) {
      post(port, { type: "side_error", id: message.id, code: "WISP_OFFLINE" });
      return;
    }
    var read = await readTab(tab.id);
    var payload = sideAskPayload(message, tab, read.scan, read.selection);
    post(port, {
      type: "side_sent",
      id: message.id,
      url: payload.page.url,
      title: payload.page.title,
      text_chars: (payload.page.text || "").length,
      selection_chars: (payload.page.selection || "").length
    });
    deps.send(payload);
  }

  function attach(port) {
    ports.add(port);
    port.onDisconnect.addListener(function () { ports.delete(port); });
    port.onMessage.addListener(function (message) {
      if (!message) return;
      if (message.type === "ask") {
        ask(port, message).catch(function (error) {
          post(port, { type: "side_error", id: message.id, error: (error && error.message) || String(error) });
        });
      } else if (message.type === "stop") {
        deps.send({ type: "side_stop", session_id: message.sessionId });
      } else if (message.type === "approval") {
        deps.send({
          type: "side_approval_response",
          id: message.id,
          approval_id: message.approvalId,
          approved: !!message.approved
        });
      }
    });
  }

  return { attach: attach, broadcast: broadcast };
}

if (typeof self !== "undefined") {
  self.isSideMessage = isSideMessage;
  self.sideAskPayload = sideAskPayload;
  self.createSideRelay = createSideRelay;
}
