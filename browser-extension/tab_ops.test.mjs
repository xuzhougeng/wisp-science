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
function fakeChrome(openWindows, options = {}) {
  let nextTab = 100;
  let nextWindow = 1;
  let nextGroup = 500;
  const windows = [];
  const groups = new Map();
  for (let i = 0; i < openWindows; i++) windows.push(nextWindow++);
  const calls = { tabsCreate: [], windowsCreate: [], tabsGroup: [], groupsUpdate: [] };
  const chrome = {
    calls,
    groups,
    tabs: {
      async create(props) {
        calls.tabsCreate.push({ ...props });
        if (props.windowId !== undefined && !windows.includes(props.windowId)) {
          throw new Error(`No window with id: ${props.windowId}.`);
        }
        const windowId = props.windowId ?? windows.at(-1);
        if (windowId === undefined) throw new Error("No current window");
        return { id: nextTab++, url: "", status: "loading", active: !!props.active, windowId, groupId: -1 };
      }
    },
    windows: {
      async create(props) {
        calls.windowsCreate.push({ ...props });
        await new Promise((resolve) => setTimeout(resolve, 5));
        const id = nextWindow++;
        windows.push(id);
        return { id, focused: !!props.focused, tabs: [{ id: nextTab++, url: "", status: "loading", active: true, windowId: id, groupId: -1 }] };
      }
    }
  };
  if (options.groups !== false) {
    chrome.tabs.group = async (props) => {
      calls.tabsGroup.push(JSON.parse(JSON.stringify(props)));
      if (props.groupId !== undefined) {
        if (!groups.has(props.groupId)) throw new Error(`No group with id: ${props.groupId}.`);
        return props.groupId;
      }
      const id = nextGroup++;
      groups.set(id, { id, windowId: props.createProperties.windowId, title: "", color: "grey" });
      return id;
    };
    chrome.tabGroups = {
      async get(id) {
        if (!groups.has(id)) throw new Error(`No group with id: ${id}.`);
        return { ...groups.get(id) };
      },
      async update(id, props) {
        calls.groupsUpdate.push({ id, ...props });
        Object.assign(groups.get(id), props);
        return { ...groups.get(id) };
      }
    };
  }
  return chrome;
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

test("create pins the tab to the owner's window instead of the focused one", async () => {
  const chrome = fakeChrome(2);
  const ops = root.createTabOps(chrome);
  const tab = await ops.create("https://example.com", false, { windowId: 1 });
  assert.deepEqual(chrome.calls.tabsCreate, [{ url: "https://example.com", active: false, windowId: 1 }]);
  assert.equal(tab.windowId, 1);
  assert.deepEqual(chrome.calls.windowsCreate, []);
});

test("a closed owner window is rebuilt rather than borrowing another window", async () => {
  const chrome = fakeChrome(1);
  const ops = root.createTabOps(chrome);
  const tab = await ops.create("https://example.com", false, { windowId: 42 });
  assert.equal(chrome.calls.tabsCreate.length, 1);
  assert.deepEqual(chrome.calls.windowsCreate, [{ url: "https://example.com", focused: false }]);
  assert.equal(tab.windowId, 2, "new window, not the pre-existing window 1");
  assert.equal(ops.isManaged(tab.id), true);
});

test("missing-window errors are only recovered when a window was pinned", async () => {
  const chrome = fakeChrome(1);
  chrome.tabs.create = async () => { throw new Error("No window with id: 9."); };
  const ops = root.createTabOps(chrome);
  await assert.rejects(ops.create("https://example.com", false), (error) => error.code === "TAB_ERROR");
  assert.deepEqual(chrome.calls.windowsCreate, []);
  assert.equal(root.isMissingWindowError(new Error("No window with id: 9.")), true);
  assert.equal(root.isMissingWindowError(new Error("No current window")), false);
});

test("create labels the tab with a new project group when none exists", async () => {
  const chrome = fakeChrome(1);
  const ops = root.createTabOps(chrome);
  const tab = await ops.create("https://example.com", false, {
    group: { title: "Wisp · Project A", color: "cyan" }
  });
  assert.equal(chrome.calls.tabsGroup.length, 1);
  assert.deepEqual(chrome.calls.tabsGroup[0], { tabIds: [tab.id], createProperties: { windowId: 1 } });
  assert.deepEqual(chrome.calls.groupsUpdate, [{ id: 500, title: "Wisp · Project A", color: "cyan" }]);
  assert.equal(tab.groupId, 500);
});

test("create reuses the remembered group while it lives in the tab's window", async () => {
  const chrome = fakeChrome(1);
  const ops = root.createTabOps(chrome);
  const first = await ops.create("https://a.example", false, { group: { title: "Wisp · P", color: "blue" } });
  const second = await ops.create("https://b.example", false, {
    windowId: 1,
    group: { id: first.groupId, title: "Wisp · P", color: "blue" }
  });
  assert.equal(second.groupId, first.groupId);
  assert.deepEqual(chrome.calls.tabsGroup[1], { tabIds: [second.id], groupId: first.groupId });
  assert.equal(chrome.calls.groupsUpdate.length, 1, "an existing group is not renamed");
});

test("a remembered group from another window or a dead group is replaced", async () => {
  const chrome = fakeChrome(1);
  const ops = root.createTabOps(chrome);
  const rebuilt = await ops.create("https://a.example", false, {
    windowId: 77,
    group: { id: 999, title: "Wisp · P", color: "blue" }
  });
  assert.equal(rebuilt.windowId, 2);
  assert.equal(rebuilt.groupId, 500);
  assert.deepEqual(chrome.calls.tabsGroup[0], { tabIds: [rebuilt.id], createProperties: { windowId: 2 } });
  chrome.groups.get(500).windowId = 1;
  const moved = await ops.create("https://b.example", false, {
    windowId: 2,
    group: { id: 500, title: "Wisp · P", color: "blue" }
  });
  assert.equal(moved.groupId, 501, "the group now lives in window 1, so window 2 gets its own");
});

test("grouping failures and browsers without tab groups never fail the open", async () => {
  const noGroups = fakeChrome(1, { groups: false });
  const plain = await root.createTabOps(noGroups).create("https://a.example", false, { group: { title: "Wisp" } });
  assert.equal(plain.groupId, undefined);
  assert.equal(plain.managed, true);
  const failing = fakeChrome(1);
  failing.tabs.group = async () => { throw new Error("Tabs cannot be edited right now"); };
  const tab = await root.createTabOps(failing).create("https://a.example", false, { group: { title: "Wisp" } });
  assert.equal(tab.groupId, undefined);
  assert.equal(tab.managed, true);
});
