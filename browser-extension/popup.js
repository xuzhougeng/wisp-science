const status = document.getElementById("status");
const meta = document.getElementById("meta");
const tabs = document.getElementById("tabs");

async function refresh() {
  const result = await chrome.runtime.sendMessage({ type: "wisp_bridge_status" });
  status.className = result?.connected ? "ok" : "bad";
  status.textContent = result?.connected
    ? (result.paused ? "Connected (paused)" : "Connected to Wisp")
    : (result?.error || "Wisp is not connected");
  meta.textContent = [
    "v" + (result?.extension_version || "?"),
    "protocol " + (result?.protocol_version || "?"),
    result?.session || "shared",
    result?.endpoint || ""
  ].join(" · ");
  tabs.innerHTML = "";
  (result?.tabs || []).slice(0, 8).forEach((tab) => {
    const li = document.createElement("li");
    const mark = tab.managed ? "managed · " : "";
    li.textContent = mark + (tab.title || tab.url || ("tab " + tab.id));
    tabs.appendChild(li);
  });
}

let currentWindowId = null;
chrome.windows.getCurrent().then((win) => { currentWindowId = win.id; });
document.getElementById("ask").addEventListener("click", () => {
  // No await before open(): Chrome only opens the panel inside the click gesture.
  if (chrome.sidePanel && currentWindowId !== null) chrome.sidePanel.open({ windowId: currentWindowId });
  window.close();
});
document.getElementById("reconnect").addEventListener("click", async () => {
  await chrome.runtime.sendMessage({ type: "wisp_bridge_connect" });
  setTimeout(refresh, 250);
});
document.getElementById("pause").addEventListener("click", async () => {
  await chrome.runtime.sendMessage({ type: "wisp_bridge_pause", paused: true });
  setTimeout(refresh, 150);
});
document.getElementById("resume").addEventListener("click", async () => {
  await chrome.runtime.sendMessage({ type: "wisp_bridge_pause", paused: false });
  setTimeout(refresh, 150);
});
document.getElementById("release").addEventListener("click", async () => {
  await chrome.runtime.sendMessage({ type: "wisp_bridge_release" });
  setTimeout(refresh, 150);
});
document.getElementById("disconnect").addEventListener("click", async () => {
  await chrome.runtime.sendMessage({ type: "wisp_bridge_disconnect" });
  setTimeout(refresh, 250);
});

refresh();
