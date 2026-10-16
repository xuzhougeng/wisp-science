import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(dir, "tab_ops.js"), "utf8");
const root = {};
vm.runInNewContext(source, { self: root, globalThis: root, setTimeout, clearTimeout, Promise });

test("tab drag errors are retryable TAB_BUSY", () => {
  assert.equal(root.isTransientTabError(new Error("Tabs cannot be edited right now (user may be dragging a tab).")), true);
  assert.equal(root.isTransientTabError(new Error("permission denied")), false);
});

test("withTabRetry eventually succeeds", async () => {
  let n = 0;
  const value = await root.withTabRetry(async () => {
    n += 1;
    if (n < 3) throw new Error("Tabs cannot be edited right now");
    return "ok";
  }, 4);
  assert.equal(value, "ok");
  assert.equal(n, 3);
});
// Chrome that may have no window open, like Windows background mode after
// the last window closed. Calls are copied out of the vm realm so deepEqual
// does not trip over a foreign Object.prototype.
function fakeChrome(openWindows) {
  let nextTab = 100;
  let nextWindow = 1;
  const windows = [];
  for (let i = 0; i < openWindows; i++) windows.push(nextWindow++);
  const calls = { tabsCreate: [], windowsCreate: [] };
  return {
    calls,
    tabs: {
      async create(props) {
        calls.tabsCreate.push({ ...props });
        const windowId = props.windowId ?? windows.at(-1);
        if (windowId === undefined) throw new Error("No current window");
        return { id: nextTab++, url: "", status: "loading", active: !!props.active, windowId };
      }
    },
    windows: {
      async create(props) {
        calls.windowsCreate.push({ ...props });
        await new Promise((resolve) => setTimeout(resolve, 5));
        const id = nextWindow++;
        windows.push(id);
        return { id, focused: !!props.focused, tabs: [{ id: nextTab++, url: "", status: "loading", active: true, windowId: id }] };
      }
    }
  };
}

test("create opens the tab in the current window when one exists", async () => {
  const chrome = fakeChrome(1);
  const ops = root.createTabOps(chrome);
  const tab = await ops.create("https://example.com", false);
  assert.deepEqual(chrome.calls.tabsCreate, [{ url: "https://example.com", active: false }]);
  assert.deepEqual(chrome.calls.windowsCreate, []);
  assert.equal(tab.windowId, 1);
  assert.equal(tab.managed, true);
  assert.equal(ops.isManaged(tab.id), true);
});

test("create opens a new window when Chrome has none", async () => {
  const chrome = fakeChrome(0);
  const ops = root.createTabOps(chrome);
  const tab = await ops.create("https://example.com", false);
  assert.deepEqual(chrome.calls.windowsCreate, [{ url: "https://example.com", focused: false }]);
  assert.equal(tab.windowId, 1);
  assert.equal(tab.managed, true);
  assert.equal(ops.isManaged(tab.id), true);
});

test("a new window takes focus when the tab is active", async () => {
  const chrome = fakeChrome(0);
  const ops = root.createTabOps(chrome);
  await ops.create("https://example.com", true);
  assert.deepEqual(chrome.calls.windowsCreate, [{ url: "https://example.com", focused: true }]);
});

test("parallel creates with no window share one new window", async () => {
  const chrome = fakeChrome(0);
  const ops = root.createTabOps(chrome);
  const [a, b] = await Promise.all([
    ops.create("https://a.example", false),
    ops.create("https://b.example", false)
  ]);
  assert.equal(chrome.calls.windowsCreate.length, 1);
  const joined = chrome.calls.tabsCreate.filter((call) => call.windowId !== undefined);
  assert.equal(joined.length, 1);
  assert.equal(joined[0].windowId, a.windowId);
  assert.equal(joined[0].active, false);
  assert.equal(b.windowId, a.windowId);
  assert.notEqual(b.id, a.id);
  assert.equal(ops.isManaged(a.id), true);
  assert.equal(ops.isManaged(b.id), true);
});

test("other tab errors do not open a window", async () => {
  const chrome = fakeChrome(1);
  chrome.tabs.create = async () => { throw new Error("permission denied"); };
  const ops = root.createTabOps(chrome);
  await assert.rejects(ops.create("https://example.com", false), (error) => error.code === "TAB_ERROR");
  assert.deepEqual(chrome.calls.windowsCreate, []);
});
