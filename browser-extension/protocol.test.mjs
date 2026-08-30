import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(dir, "protocol.js"), "utf8");
const root = {};
vm.runInNewContext(source, { self: root, globalThis: root });

test("handshake reports protocol 2 and extension 0.3.1", () => {
  const payload = root.handshakePayload([{ id: 1, url: "https://example.com" }], false);
  assert.equal(payload.protocol_version, 2);
  assert.equal(payload.extension_version, "0.3.1");
  assert.ok(payload.capabilities.includes("article_scan"));
  assert.ok(payload.capabilities.includes("asset_download"));
  assert.ok(payload.capabilities.includes("chatgpt_turn"));
  assert.ok(payload.capabilities.includes("chat_turn"));
  assert.ok(payload.capabilities.includes("runtime_reload"));
  assert.equal(payload.session, "shared");
  assert.equal(payload.tabs.length, 1);
});

test("parseIncomingRequest keeps v1 code requests", () => {
  const parsed = root.parseIncomingRequest({ id: "abc", code: "document.title", tabId: 3 });
  assert.equal(parsed.kind, "v1");
  assert.equal(parsed.code, "document.title");
  assert.equal(parsed.tabId, 3);
});

test("parseIncomingRequest accepts v2 ops", () => {
  const parsed = root.parseIncomingRequest({
    id: "x",
    op: "scan",
    payload: { mode: "article" },
    tabId: 9,
  });
  assert.equal(parsed.kind, "v2");
  assert.equal(parsed.op, "scan");
  assert.equal(parsed.payload.mode, "article");
});
