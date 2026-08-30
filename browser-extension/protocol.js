// Wisp Real Browser Bridge protocol constants (Protocol v2).
var WISP_PROTOCOL = {
  version: 2,
  extensionVersion: "0.3.1",
  capabilities: [
    "article_scan",
    "asset_download",
    "selector_screenshot",
    "full_page_screenshot",
    "managed_tabs",
    "conditional_wait",
    "session",
    "pause_control",
    "runtime_reload",
    "chatgpt_turn",
    "chat_turn"
  ]
};

function readBridgeConfig() {
  var session = "shared";
  var endpoint = "ws://127.0.0.1:18765";
  try {
    if (typeof WISP_BRIDGE_CONFIG === "object" && WISP_BRIDGE_CONFIG) {
      if (WISP_BRIDGE_CONFIG.session) session = String(WISP_BRIDGE_CONFIG.session);
      if (WISP_BRIDGE_CONFIG.endpoint) endpoint = String(WISP_BRIDGE_CONFIG.endpoint);
    }
  } catch (_) {}
  return { session: session, endpoint: endpoint };
}

function handshakePayload(tabs, paused) {
  var cfg = readBridgeConfig();
  return {
    type: "ext_ready",
    protocol_version: WISP_PROTOCOL.version,
    extension_version: WISP_PROTOCOL.extensionVersion,
    capabilities: WISP_PROTOCOL.capabilities,
    session: cfg.session,
    endpoint: cfg.endpoint,
    paused: !!paused,
    tabs: tabs || []
  };
}

function parseIncomingRequest(request) {
  if (!request || typeof request !== "object") return { kind: "invalid" };
  if (request.op) {
    return {
      kind: "v2",
      id: request.id,
      op: request.op,
      tabId: request.tabId,
      timeoutMs: request.timeoutMs,
      payload: request.payload || {},
      wait: request.wait || null
    };
  }
  if (request.id && request.code !== undefined) {
    return {
      kind: "v1",
      id: request.id,
      code: request.code,
      tabId: request.tabId,
      timeoutMs: request.timeoutMs
    };
  }
  return { kind: "invalid", id: request.id };
}

if (typeof self !== "undefined") {
  self.WISP_PROTOCOL = WISP_PROTOCOL;
  self.readBridgeConfig = readBridgeConfig;
  self.handshakePayload = handshakePayload;
  self.parseIncomingRequest = parseIncomingRequest;
}
