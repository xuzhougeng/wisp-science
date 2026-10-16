// Tab operations with retry for transient Chrome errors (tab drag / moving).

function isTransientTabError(error) {
  var message = (error && error.message) || String(error || "");
  return /Tabs cannot be edited right now|user may be dragging|being dragged|No tab with id/i.test(message);
}

// Chrome can outlive its last window (background mode on Windows, the app on
// macOS). tabs.create without a windowId then has nowhere to go.
function isNoWindowError(error) {
  var message = (error && error.message) || String(error || "");
  return /No current window/i.test(message);
}

async function withTabRetry(operation, attempts) {
  var max = attempts || 4;
  var lastError = null;
  for (var i = 0; i < max; i++) {
    try {
      return await operation();
    } catch (error) {
      lastError = error;
      if (!isTransientTabError(error) || i === max - 1) {
        var wrapped = new Error(error.message || String(error));
        wrapped.code = isTransientTabError(error) ? "TAB_BUSY" : "TAB_ERROR";
        wrapped.retryable = isTransientTabError(error);
        wrapped.operation = "tab";
        wrapped.attempts = i + 1;
        throw wrapped;
      }
      await new Promise(function (resolve) { setTimeout(resolve, 80 * Math.pow(2, i)); });
    }
  }
  throw lastError;
}

function createTabOps(chromeLike, managedIds) {
  var managed = managedIds || new Set();
  var pendingWindow = null;

  // Parallel opens that all find no window share the first new window
  // instead of each opening their own.
  var openInNewWindow = async function (url, active) {
    if (pendingWindow) {
      var shared = await pendingWindow;
      return await withTabRetry(function () {
        return chromeLike.tabs.create({ windowId: shared.id, url: url, active: active ?? false });
      });
    }
    pendingWindow = withTabRetry(function () {
      return chromeLike.windows.create({ url: url, focused: !!active });
    });
    try {
      return (await pendingWindow).tabs[0];
    } finally {
      pendingWindow = null;
    }
  };

  var publicTab = function (tab) {
    if (!tab) return null;
    return {
      id: tab.id,
      url: tab.url || "",
      title: tab.title || "",
      status: tab.status || "",
      active: !!tab.active,
      windowId: tab.windowId,
      managed: managed.has(tab.id)
    };
  };

  return {
    markManaged: function (id) {
      if (Number.isInteger(id)) managed.add(id);
    },
    unmark: function (id) {
      managed.delete(id);
    },
    isManaged: function (id) {
      return managed.has(id);
    },
    releaseAll: function () {
      managed.clear();
      return true;
    },
    create: async function (url, active) {
      var tab;
      try {
        tab = await withTabRetry(function () {
          return chromeLike.tabs.create({ url: url, active: active ?? false });
        });
      } catch (error) {
        if (!isNoWindowError(error)) throw error;
        tab = await openInNewWindow(url, active);
      }
      managed.add(tab.id);
      return publicTab(tab);
    },
    close: async function (ids) {
      var closed = [];
      for (var i = 0; i < ids.length; i++) {
        var id = ids[i];
        try {
          await withTabRetry(function () { return chromeLike.tabs.remove(id); });
          closed.push(id);
          managed.delete(id);
        } catch (_) {}
      }
      return closed;
    },
    switchTo: async function (tabId) {
      var tab = await withTabRetry(function () {
        return chromeLike.tabs.update(tabId, { active: true });
      });
      await withTabRetry(function () {
        return chromeLike.windows.update(tab.windowId, { focused: true });
      });
      return { ok: true, tab: publicTab(tab) };
    },
    get: async function (tabId) {
      try {
        return publicTab(await chromeLike.tabs.get(tabId));
      } catch (_) {
        return null;
      }
    },
    publicTab: publicTab
  };
}

if (typeof self !== "undefined") {
  self.createTabOps = createTabOps;
  self.isTransientTabError = isTransientTabError;
  self.isNoWindowError = isNoWindowError;
  self.withTabRetry = withTabRetry;
}
