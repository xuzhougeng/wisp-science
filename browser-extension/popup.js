const $ = (id) => document.getElementById(id);
const status = $("status");
const meta = $("meta");
const tabs = $("tabs");

async function refresh() {
  const result = await chrome.runtime.sendMessage({ type: "wisp_bridge_status" });
  const connected = Boolean(result?.connected);
  status.className = connected ? (result.paused ? "warn" : "ok") : "bad";
  status.textContent = connected ? (result.paused ? "Paused" : "Connected") : "Not connected";
  $("error").hidden = connected || !result?.error;
  $("error").textContent = result?.error || "";
  meta.textContent = [
    "v" + (result?.extension_version || "?"),
    "protocol " + (result?.protocol_version || "?"),
    result?.session || "shared",
    result?.endpoint || ""
  ].filter(Boolean).join(" · ");
  $("pause").hidden = Boolean(result?.paused);
  $("resume").hidden = !result?.paused;
  $("disconnect").disabled = !connected;

  tabs.innerHTML = "";
  const list = (result?.tabs || []).slice(0, 8);
  if (!list.length) {
    const li = document.createElement("li");
    li.innerHTML = '<span class="empty">No open tabs</span>';
    tabs.appendChild(li);
  }
  list.forEach((tab) => {
    const li = document.createElement("li");
    const title = document.createElement("span");
    title.textContent = tab.title || tab.url || ("tab " + tab.id);
    title.title = tab.url || "";
    li.appendChild(title);
    if (tab.managed) {
      const tag = document.createElement("span");
      tag.className = "tag";
      tag.textContent = "managed";
      li.appendChild(tag);
    }
    tabs.appendChild(li);
  });
}

// Chrome owns shortcut customization (chrome://extensions/shortcuts); the popup
// only shows the current binding and links there.
async function refreshShortcut() {
  const commands = await chrome.commands.getAll();
  const key = commands.find((c) => c.name === "open-side-panel")?.shortcut || "";
  $("key").hidden = !key;
  $("key").textContent = key;
  $("key-hint").textContent = key ? "Shortcut works on any page" : "Shortcut not set";
}
$("key-edit").addEventListener("click", () => {
  chrome.tabs.create({ url: "chrome://extensions/shortcuts" });
  window.close();
});

let currentWindowId = null;
chrome.windows.getCurrent().then((win) => { currentWindowId = win.id; });
$("ask").addEventListener("click", () => {
  // No await before open(): Chrome only opens the panel inside the click gesture.
  if (chrome.sidePanel && currentWindowId !== null) chrome.sidePanel.open({ windowId: currentWindowId });
  window.close();
});
const send = (message, delay) => async () => {
  await chrome.runtime.sendMessage(message);
  setTimeout(refresh, delay);
};
$("reconnect").addEventListener("click", send({ type: "wisp_bridge_connect" }, 250));
$("pause").addEventListener("click", send({ type: "wisp_bridge_pause", paused: true }, 150));
$("resume").addEventListener("click", send({ type: "wisp_bridge_pause", paused: false }, 150));
$("release").addEventListener("click", send({ type: "wisp_bridge_release" }, 150));
$("disconnect").addEventListener("click", send({ type: "wisp_bridge_disconnect" }, 250));

refresh();
refreshShortcut();
