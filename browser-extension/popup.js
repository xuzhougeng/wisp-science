const status = document.getElementById("status");

async function refresh() {
  const result = await chrome.runtime.sendMessage({ type: "wisp_bridge_status" });
  status.className = result?.connected ? "ok" : "bad";
  status.textContent = result?.connected
    ? "Connected to Wisp"
    : (result?.error || "Wisp is not connected");
}

document.getElementById("reconnect").addEventListener("click", async () => {
  await chrome.runtime.sendMessage({ type: "wisp_bridge_connect" });
  setTimeout(refresh, 250);
});

refresh();
