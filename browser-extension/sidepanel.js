// Wisp side panel: ask Wisp's local agent loop about the page in this window.
// The service worker reads the tab and owns the bridge socket (side_relay.js);
// this page only renders the conversation.

const zh = /^zh/i.test(navigator.language || "");
const T = zh
  ? {
      placeholder: "就当前页面提问…",
      send: "提问",
      stop: "停止",
      stopping: "正在停止…",
      newChat: "新对话",
      empty: "问问这个页面有没有你需要的信息。先选中一段文字，就只针对那一段提问。",
      thinking: "思考中…",
      running: (name) => "正在运行 " + name + "…",
      selection: (n) => "选中 " + n + " 字",
      pageText: (n) => "页面正文 " + n + " 字",
      noText: "读不到正文，Wisp 会自行打开链接",
      approval: "Wisp 请求执行",
      allow: "允许一次",
      deny: "拒绝",
      allowed: "已允许",
      denied: "已拒绝",
      noReply: "（没有文本回复）",
      noPage: "无法读取这个标签页，请切换到一个 http(s) 网页。",
      offline: "Wisp 没有运行。请先启动 Wisp 桌面端，再重新提问。",
      timeout: "Wisp 没有响应。请把 Wisp 更新到支持侧边栏的版本。",
      interrupted: "与 Wisp 的连接中断了。这次回答会在 Wisp 桌面端的会话里继续。"
    }
  : {
      placeholder: "Ask about this page…",
      send: "Ask",
      stop: "Stop",
      stopping: "Stopping…",
      newChat: "New chat",
      empty: "Ask whether this page has what you need. Select text first to ask about just that passage.",
      thinking: "Thinking…",
      running: (name) => "Running " + name + "…",
      selection: (n) => "selection, " + n + " chars",
      pageText: (n) => "page text, " + n + " chars",
      noText: "no readable text; Wisp will open the URL itself",
      approval: "Wisp wants to run",
      allow: "Allow once",
      deny: "Deny",
      allowed: "Allowed",
      denied: "Denied",
      noReply: "(no text reply)",
      noPage: "This tab can't be read. Switch to an http(s) page.",
      offline: "Wisp isn't running. Start the Wisp desktop app, then ask again.",
      timeout: "Wisp did not respond. Update Wisp to a version that supports the side panel.",
      interrupted: "The connection to Wisp was interrupted. This answer continues in the conversation in the Wisp app."
    };

const START_TIMEOUT_MS = 20000;

const log = document.getElementById("log");
const pageLabel = document.getElementById("page");
const input = document.getElementById("question");
const sendButton = document.getElementById("send");
const stopButton = document.getElementById("stop");
const newButton = document.getElementById("new");

// sessionId: the Wisp conversation this panel continues.
// sentUrl: the page whose text that conversation already holds.
let conv = { sessionId: null, sentUrl: null };
let pending = null;
let port = null;
let windowId = null;
let asked = 0;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
}

function scrollToEnd() {
  log.scrollTop = log.scrollHeight;
}

function setBusy(busy) {
  sendButton.disabled = busy;
  newButton.disabled = busy;
  stopButton.hidden = !busy;
  stopButton.textContent = T.stop;
  // Stopping needs the conversation id, which arrives with side_started.
  stopButton.disabled = true;
}

function showEmpty() {
  log.replaceChildren(el("div", "empty", T.empty));
}

function link() {
  if (port) return port;
  port = chrome.runtime.connect({ name: "wisp_side_panel" });
  port.onMessage.addListener(onMessage);
  port.onDisconnect.addListener(() => {
    port = null;
    if (pending) fail(T.interrupted);
  });
  return port;
}

async function refreshPage() {
  if (windowId === null) return;
  const tab = (await chrome.tabs.query({ active: true, windowId }))[0];
  pageLabel.textContent = (tab && (tab.title || tab.url)) || "";
  pageLabel.title = (tab && tab.url) || "";
}

function finish() {
  clearTimeout(pending.timer);
  pending.status.remove();
  pending = null;
  setBusy(false);
  input.focus();
}

function fail(text) {
  clearTimeout(pending.timer);
  pending.status.textContent = text;
  pending.status.classList.add("error");
  pending = null;
  setBusy(false);
}

function errorText(message) {
  if (message.code === "NO_PAGE") return T.noPage;
  if (message.code === "WISP_OFFLINE") return T.offline;
  return message.error || T.timeout;
}

function showApproval(message) {
  const card = el("div", "approval");
  card.append(el("div", "", T.approval + (message.tool ? " " + message.tool : "")));
  const detail = message.preview || message.message;
  if (detail) card.append(el("pre", "", detail));
  const actions = el("div", "actions");
  const deny = el("button", "", T.deny);
  const allow = el("button", "primary", T.allow);
  const decide = (approved) => {
    link().postMessage({ type: "approval", id: message.id, approvalId: message.approval_id, approved });
    actions.replaceWith(el("div", "status", approved ? T.allowed : T.denied));
  };
  deny.addEventListener("click", () => decide(false));
  allow.addEventListener("click", () => decide(true));
  actions.append(deny, allow);
  card.append(actions);
  pending.turn.insertBefore(card, pending.status);
  pending.status.textContent = "";
  scrollToEnd();
}

function onMessage(message) {
  if (!pending || !message || message.id !== pending.id) return;
  switch (message.type) {
    case "side_sent": {
      const parts = [message.title || message.url];
      if (message.selection_chars) parts.push(T.selection(message.selection_chars));
      else if (message.text_chars) parts.push(T.pageText(message.text_chars));
      else if (conv.sentUrl !== message.url) parts.push(T.noText);
      pending.meta.textContent = parts.join(" · ");
      if (message.text_chars) pending.sentUrl = message.url;
      break;
    }
    case "side_started":
      clearTimeout(pending.timer);
      conv.sessionId = message.session_id;
      stopButton.disabled = false;
      break;
    case "side_delta":
      pending.status.textContent = "";
      pending.answer.textContent += message.text;
      scrollToEnd();
      break;
    case "side_tool":
      if (message.state === "started") {
        const text = pending.answer.textContent;
        if (text && !text.endsWith("\n")) pending.answer.textContent = text + "\n\n";
        pending.status.textContent = T.running(message.name);
      } else {
        pending.status.textContent = T.thinking;
      }
      break;
    case "side_approval":
      showApproval(message);
      break;
    case "side_done":
      // The page text is in the conversation only once the turn was accepted.
      if (pending.sentUrl) conv.sentUrl = pending.sentUrl;
      // ponytail: the reply is shown as plain text; add a Markdown renderer if raw Markdown reads badly.
      pending.answer.textContent = message.answer || pending.answer.textContent || T.noReply;
      finish();
      scrollToEnd();
      break;
    case "side_error":
      fail(errorText(message));
      break;
  }
}

function submit() {
  const question = input.value.trim();
  if (!question || pending || windowId === null) return;
  if (!log.querySelector(".turn")) log.replaceChildren();
  const turn = el("div", "turn");
  const meta = el("div", "meta");
  const answer = el("div", "answer");
  const status = el("div", "status", T.thinking);
  turn.append(el("div", "question", question), meta, answer, status);
  log.append(turn);
  asked += 1;
  const id = "side-" + Date.now().toString(36) + "-" + asked;
  pending = {
    id,
    turn,
    meta,
    answer,
    status,
    sentUrl: null,
    timer: setTimeout(() => {
      if (pending && pending.id === id) fail(T.timeout);
    }, START_TIMEOUT_MS)
  };
  input.value = "";
  setBusy(true);
  link().postMessage({
    type: "ask",
    id,
    question,
    windowId,
    sessionId: conv.sessionId,
    sentUrl: conv.sentUrl
  });
  scrollToEnd();
}

document.getElementById("form").addEventListener("submit", (event) => {
  event.preventDefault();
  submit();
});
input.addEventListener("keydown", (event) => {
  // isComposing: Enter that confirms an IME candidate must not send.
  if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
    event.preventDefault();
    submit();
  }
});
stopButton.addEventListener("click", () => {
  if (!pending || !conv.sessionId) return;
  stopButton.disabled = true;
  stopButton.textContent = T.stopping;
  link().postMessage({ type: "stop", sessionId: conv.sessionId });
});
newButton.addEventListener("click", () => {
  if (pending) return;
  conv = { sessionId: null, sentUrl: null };
  showEmpty();
  input.focus();
});

chrome.tabs.onActivated.addListener((info) => {
  if (info.windowId === windowId) refreshPage();
});
chrome.tabs.onUpdated.addListener((_id, change, tab) => {
  if (tab.active && tab.windowId === windowId && (change.title || change.url)) refreshPage();
});

document.documentElement.lang = zh ? "zh" : "en";
input.placeholder = T.placeholder;
sendButton.textContent = T.send;
newButton.textContent = T.newChat;
setBusy(false);
showEmpty();
chrome.windows.getCurrent().then((win) => {
  windowId = win.id;
  refreshPage();
});
input.focus();
