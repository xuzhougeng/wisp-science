import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(dir, "side_relay.js"), "utf8");
const root = {};
vm.runInNewContext(source, { self: root, globalThis: root });

const tab = { id: 7, url: "https://example.com/paper", title: "Tab title" };
const scan = { title: "Paper", text: "Body of the paper" };

test("the first question about a page carries its text", () => {
  const payload = root.sideAskPayload({ id: "a1", question: "  Any RNA-seq data? " }, tab, scan, "");
  assert.equal(payload.type, "side_ask");
  assert.equal(payload.id, "a1");
  assert.equal(payload.session_id, null);
  assert.equal(payload.question, "Any RNA-seq data?");
  assert.equal(payload.page.url, tab.url);
  assert.equal(payload.page.title, "Paper");
  assert.equal(payload.page.text, "Body of the paper");
  assert.equal(payload.page.tab_id, 7);
  assert.equal("selection" in payload.page, false);
  assert.equal("unreadable" in payload.page, false);
});

test("a follow-up on the same URL does not resend the page text", () => {
  const payload = root.sideAskPayload(
    { id: "a2", question: "And the sample size?", sessionId: "s1", sentUrl: tab.url },
    tab,
    scan,
    ""
  );
  assert.equal(payload.session_id, "s1");
  assert.equal("text" in payload.page, false);
  assert.equal("unreadable" in payload.page, false);
});

test("navigating to another URL sends the new page text", () => {
  const payload = root.sideAskPayload(
    { id: "a3", question: "q", sessionId: "s1", sentUrl: "https://example.com/other" },
    tab,
    scan,
    ""
  );
  assert.equal(payload.page.text, "Body of the paper");
});

test("a selection is always sent and capped", () => {
  const payload = root.sideAskPayload(
    { id: "a4", question: "q", sessionId: "s1", sentUrl: tab.url },
    tab,
    scan,
    "x".repeat(30000)
  );
  assert.equal(payload.page.selection.length, 20000);
  const blank = root.sideAskPayload({ id: "a5", question: "q" }, tab, scan, "  \n ");
  assert.equal("selection" in blank.page, false);
});

test("an unreadable tab still sends its URL and title", () => {
  const payload = root.sideAskPayload({ id: "a6", question: "q" }, tab, undefined, undefined);
  assert.equal(payload.page.url, tab.url);
  assert.equal(payload.page.title, "Tab title");
  assert.equal("text" in payload.page, false);
  assert.equal(payload.page.unreadable, true);
});

test("only side_ messages are routed to the panel", () => {
  assert.equal(root.isSideMessage({ type: "side_delta", id: "a1" }), true);
  assert.equal(root.isSideMessage({ id: "1", code: "1+1" }), false);
  assert.equal(root.isSideMessage({ type: "result", id: "1" }), false);
  assert.equal(root.isSideMessage(null), false);
});
