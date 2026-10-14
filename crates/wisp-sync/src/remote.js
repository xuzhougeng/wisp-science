"use strict";
// Wisp remote web client (#1460). The connection code stays in the URL
// fragment and, once it has worked, in this browser's storage; the relay only
// sees the derived rendezvous id and AES-GCM frames. Every host string is
// rendered with textContent.
(() => {
  const zh = (navigator.language || "").toLowerCase().startsWith("zh");
  const T = zh
    ? {
        connectTitle: "连接到你的 Wisp", connectHint: "输入桌面端 设置 → 渠道 → 网页远程访问 中显示的联机码，或直接打开桌面端复制的链接。",
        code: "联机码", connect: "连接", badCode: "联机码格式不正确（32 位十六进制）。",
        insecure: "浏览器只允许在 HTTPS 页面中使用加密，请通过 HTTPS 访问中继服务器。",
        online: "已连接", connecting: "连接中…", offline: "电脑离线",
        projects: "项目", conversations: "对话", newChat: "新对话", untitled: "未命名对话",
        loading: "加载中…", none: "暂无内容", back: "返回", pickChat: "选择一个对话，或开始新对话。",
        send: "发送", stop: "停止", running: "运行中", needsYou: "待处理", done: "完成", failed: "失败",
        reasoning: "思考", working: "执行中…", processed: "已处理", steps: (n) => `已执行 ${n} 步`, approvalTitle: "需要你批准：", approve: "批准", reject: "拒绝",
        readOnly: "这个对话是只读的。", acp: "ACP 会话请在桌面端继续。",
        remoteAsk: "远程发起的回合中，写文件、改文件和运行命令都需要你在这里批准。",
        disconnected: "连接已断开", timeout: "请求超时，请刷新确认结果后再操作。",
        placeholder: "给 Wisp 发消息", sessionsCount: (n) => `${n} 个对话`,
        files: "文件", closeFiles: "关闭文件", noPreview: "这类文件无法在网页中预览，请在桌面端打开。",
        truncated: (size) => `文件较大（${size}），只显示开头部分。`, imageFailed: "图片无法显示",
        queue: "排队", cancelQueued: "取消排队的消息", inbox: (n) => `还有 ${n} 个对话待处理`,
        earlier: "加载更早的消息", suggestions: "建议的后续消息", forget: "更换联机码",
        attach: "添加图片", removePhoto: "移除图片", photoFailed: "这张图片无法处理", photoLimit: (n) => `一条消息最多 ${n} 张图片。`,
        runState: { draft: "草稿", submitted: "排队中", running: "运行中", paused: "已暂停", cancelling: "取消中", succeeded: "完成", failed: "失败", cancelled: "已取消", timed_out: "超时", lost: "失联" },
      }
    : {
        connectTitle: "Connect to your Wisp", connectHint: "Enter the code shown in desktop Settings → Channels → Remote web access, or open the link copied from the desktop.",
        code: "Connection code", connect: "Connect", badCode: "Invalid code (32 hex digits).",
        insecure: "Browsers only allow encryption on HTTPS pages. Open the relay over HTTPS.",
        online: "Connected", connecting: "Connecting…", offline: "Computer offline",
        projects: "Projects", conversations: "Conversations", newChat: "New conversation", untitled: "Untitled",
        loading: "Loading…", none: "Nothing here yet", back: "Back", pickChat: "Pick a conversation or start a new one.",
        send: "Send", stop: "Stop", running: "Running", needsYou: "Needs you", done: "Done", failed: "Failed",
        reasoning: "Thinking", working: "Working…", processed: "Processed", steps: (n) => (n === 1 ? "Ran 1 step" : `Ran ${n} steps`), approvalTitle: "Approval needed: ", approve: "Approve", reject: "Reject",
        readOnly: "This conversation is read-only.", acp: "Continue ACP conversations on the desktop.",
        remoteAsk: "In remotely started turns, writing files, editing and running commands need your approval here.",
        disconnected: "Disconnected", timeout: "Request timed out. Refresh to check the result before retrying.",
        placeholder: "Message Wisp", sessionsCount: (n) => `${n} conversations`,
        files: "Files", closeFiles: "Close files", noPreview: "This file type cannot be previewed here. Open it on the desktop.",
        truncated: (size) => `Large file (${size}); only the beginning is shown.`, imageFailed: "Image unavailable",
        queue: "Queue", cancelQueued: "Cancel queued message", inbox: (n) => (n === 1 ? "1 other conversation needs you" : `${n} other conversations need you`),
        earlier: "Load earlier messages", suggestions: "Suggested next messages", forget: "Use another code",
        attach: "Add a picture", removePhoto: "Remove picture", photoFailed: "This picture could not be processed", photoLimit: (n) => `A message takes at most ${n} pictures.`,
        runState: { draft: "Draft", submitted: "Queued", running: "Running", paused: "Paused", cancelling: "Cancelling", succeeded: "Done", failed: "Failed", cancelled: "Cancelled", timed_out: "Timed out", lost: "Lost" },
      };
  document.documentElement.lang = zh ? "zh-CN" : "en";
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  const H2C = enc.encode("wisp-remote/v1/h2c");
  const C2H = enc.encode("wisp-remote/v1/c2h");
  const $ = (id) => document.getElementById(id);
  const main = $("main");
  const tr = (cn, en) => zh ? cn : en;
  const tabTitles = {
    chats: T.conversations,
    calendar: tr("科研日历", "Research calendar"),
    checklist: tr("科研清单", "Research checklist"),
    notebook: tr("实验记录本", "Lab notebook"),
  };

  function el(tag, props, ...children) {
    const node = document.createElement(tag);
    for (const [name, value] of Object.entries(props || {})) {
      if (name === "class") node.className = value;
      else if (name.startsWith("on")) node.addEventListener(name.slice(2), value);
      else if (value === true) node.setAttribute(name, "");
      else if (value != null && value !== false) node.setAttribute(name, value);
    }
    for (const child of children.flat()) {
      if (child != null && child !== false) node.append(child instanceof Node ? child : String(child));
    }
    return node;
  }

  // Same 24×24 stroke paths as the desktop's compose_icon().
  const ICONS = {
    "circle": [["circle", {"cx": "12", "cy": "12", "r": "9"}]],
    "circle-check": [["circle", {"cx": "12", "cy": "12", "r": "9"}], ["path", {"d": "m8 12 2.5 2.5L16 9"}]],
    "refresh": [["path", {"d": "M20 7v5h-5M4 17v-5h5"}], ["path", {"d": "M6 6a8 8 0 0 1 13 2M18 18a8 8 0 0 1-13-2"}]],
    "chat": [["path", {"d": "M21 15a4 4 0 0 1-4 4H8l-5 3V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4z"}], ["path", {"d": "M8 10h8"}], ["path", {"d": "M8 14h5"}]],
    "calendar": [["rect", {"x": "3", "y": "5", "width": "18", "height": "16", "rx": "2"}], ["path", {"d": "M16 3v4M8 3v4M3 11h18M8 15h2M14 15h2"}]],
    "list": [["path", {"d": "M8 6h13"}], ["path", {"d": "M8 12h13"}], ["path", {"d": "M8 18h13"}], ["path", {"d": "M3 6h.01"}], ["path", {"d": "M3 12h.01"}], ["path", {"d": "M3 18h.01"}]],
    "notebook": [["rect", {"x": "4", "y": "3", "width": "16", "height": "18", "rx": "2"}], ["path", {"d": "M8 3v18"}], ["path", {"d": "M2 7h4M2 12h4M2 17h4M12 8h4M12 12h4"}]],
    "camera": [["path", {"d": "M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3z"}], ["circle", {"cx": "12", "cy": "13", "r": "3"}]],
    "book": [["path", {"d": "M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"}], ["path", {"d": "M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"}]],
    "gear": [["path", {"d": "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"}], ["circle", {"cx": "12", "cy": "12", "r": "3"}]],
    "circle-help": [["circle", {"cx": "12", "cy": "12", "r": "10"}], ["path", {"d": "M9.1 9a3 3 0 0 1 5.8 1c0 2-3 3-3 3"}], ["path", {"d": "M12 17h.01"}]],
    "research-trail": [["path", {"d": "M3 3v18h18"}], ["path", {"d": "m7 14 4-4 4 3 5-7"}], ["circle", {"cx": "7", "cy": "14", "r": "1"}], ["circle", {"cx": "11", "cy": "10", "r": "1"}], ["circle", {"cx": "15", "cy": "13", "r": "1"}]],
    "edit": [["path", {"d": "M12 20h9"}], ["path", {"d": "M16.5 3.5a2.1 2.1 0 0 1 3 3L8 18l-4 1 1-4Z"}]],
    "square-pen": [["path", {"d": "M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"}], ["path", {"d": "M16 3a2.1 2.1 0 0 1 3 3l-9 9-4 1 1-4Z"}]],
    "inbox": [["path", {"d": "m22 12-3.45-6.9A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.1L2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2Z"}], ["path", {"d": "M2 12h6l2 3h4l2-3h6"}]],
    "chevron-left": [["path", {"d": "m15 18-6-6 6-6"}]],
    "chevron-right": [["path", {"d": "m9 18 6-6-6-6"}]],
    "plus": [["path", {"d": "M12 5v14"}], ["path", {"d": "M5 12h14"}]],
    "check": [["path", {"d": "m20 6-11 11-5-5"}]],
    "image": [["rect", {"x": "3", "y": "3", "width": "18", "height": "18", "rx": "2"}], ["circle", {"cx": "8.5", "cy": "8.5", "r": "1.5"}], ["path", {"d": "m21 15-5-5L5 21"}]],
    "arrow-up": [["path", {"d": "m5 12 7-7 7 7"}], ["path", {"d": "M12 19V5"}]],
    folder: ["M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"],
    doc: ["M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z", "M14 2v6h6"],
    close: ["M18 6 6 18", "m6 6 12 12"],
    clock: ["M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20z", "M12 6v6l4 2"],
  };
  function icon(name) {
    const ns = "http://www.w3.org/2000/svg";
    const svg = document.createElementNS(ns, "svg");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("aria-hidden", "true");
    svg.setAttribute("stroke", "currentColor");
    for (const shape of ICONS[name]) {
      const [tag, attrs] = typeof shape === "string" ? ["path", { d: shape }] : shape;
      const node = document.createElementNS(ns, tag);
      for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
      svg.append(node);
    }
    return svg;
  }

  // ---------------------------------------------------------------- crypto
  const toHex = (bytes) => Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  function concat(a, b) {
    const out = new Uint8Array(a.length + b.length);
    out.set(a);
    out.set(b, a.length);
    return out;
  }
  function b64(bytes) {
    let text = "";
    for (let i = 0; i < bytes.length; i += 0x8000) text += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(text);
  }
  function unb64(text) {
    const raw = atob(text);
    return Uint8Array.from(raw, (c) => c.charCodeAt(0));
  }
  function parseCode(text) {
    const hex = (text || "").replace(/[^0-9a-f]/gi, "").toLowerCase();
    return hex.length === 32 ? Uint8Array.from(hex.match(/../g), (h) => parseInt(h, 16)) : null;
  }
  async function derive(secret) {
    const sid = new Uint8Array(await crypto.subtle.digest("SHA-256", concat(enc.encode("wisp-remote/sid/v1"), secret)));
    const raw = await crypto.subtle.digest("SHA-256", concat(enc.encode("wisp-remote/key/v1"), secret));
    const key = await crypto.subtle.importKey("raw", raw, "AES-GCM", false, ["encrypt", "decrypt"]);
    return { sid: toHex(sid.subarray(0, 16)), key };
  }
  async function seal(key, value) {
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const body = await crypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData: C2H }, key, enc.encode(JSON.stringify(value)));
    return b64(concat(iv, new Uint8Array(body)));
  }
  async function unseal(key, frame) {
    const bytes = unb64(frame);
    const body = await crypto.subtle.decrypt({ name: "AES-GCM", iv: bytes.subarray(0, 12), additionalData: H2C }, key, bytes.subarray(12));
    return JSON.parse(dec.decode(body));
  }

  // ------------------------------------------------------------ connection
  const state = {
    code: "", sid: null, key: null, ws: null, nonce: null, seq: 0, retry: 0, host: null,
    pending: new Map(), waiters: [], sendChain: Promise.resolve(), recvChain: Promise.resolve(),
    route: {}, projects: [], sessions: [], inbox: [], reconnect: 0,
  };
  // A page installed on the home screen opens without the link, so a code
  // that has worked is kept on this device until "Use another code".
  const saved = {
    get() {
      try {
        return localStorage.getItem("wisp-remote-code") || "";
      } catch {
        return ""; // Storage can be disabled; the link still works.
      }
    },
    set(code) {
      try {
        if (code) localStorage.setItem("wisp-remote-code", code);
        else localStorage.removeItem("wisp-remote-code");
      } catch {}
    },
  };
  /// The relay serves this page, so the desktop behind it can be older.
  const can = (command) => !!state.host && (state.host.commands || []).includes(command);
  // Each pane reloads on its own; a stale loop sees a newer generation and stops.
  const gens = { side: 0, chat: 0, panel: 0 };

  function setStatus(kind) {
    const node = $("status");
    node.dataset.state = kind === "online" ? "online" : kind === "offline" ? "offline" : "idle";
    node.textContent = T[kind];
  }

  function connect() {
    setStatus("connecting");
    const url = new URL(`v1/remote/client/${state.sid}`, location.href);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.hash = "";
    const ws = new WebSocket(url);
    state.ws = ws;
    ws.onmessage = (event) => {
      state.recvChain = state.recvChain.then(() => receive(ws, event.data)).catch(() => {});
    };
    ws.onclose = (event) => {
      if (state.ws !== ws) return;
      state.ws = null;
      state.nonce = null;
      for (const pending of state.pending.values()) {
        clearTimeout(pending.timer);
        pending.reject(new Error(T.disconnected));
      }
      state.pending.clear();
      setStatus(event.code === 4404 ? "offline" : "connecting");
      state.reconnect = setTimeout(connect, Math.min(8000, 1000 * 2 ** state.retry++));
    };
  }

  async function receive(ws, data) {
    if (state.ws !== ws || typeof data !== "string") return;
    let message;
    try {
      message = await unseal(state.key, data);
    } catch {
      return; // Not sealed with this code: drop it.
    }
    if (message.type === "hello") {
      state.nonce = message.nonce;
      state.seq = 0;
      state.retry = 0;
      state.host = message;
      saved.set(state.code);
      setStatus("online");
      for (const resolve of state.waiters.splice(0)) resolve();
      renderHeader();
      refreshNow();
    } else if (message.type === "response" && message.response) {
      const response = message.response;
      const pending = state.pending.get(response.id);
      if (!pending) return;
      state.pending.delete(response.id);
      clearTimeout(pending.timer);
      if (response.error != null) pending.reject(new Error(response.error));
      else pending.resolve(response.result);
    }
  }

  const whenReady = () => (state.nonce ? Promise.resolve() : new Promise((resolve) => state.waiters.push(resolve)));

  // Mutations are never retried automatically; a lost reply surfaces as an
  // error and the next snapshot shows what actually happened.
  async function call(command, projectId, args = {}, timeout = 90000) {
    await whenReady();
    return new Promise((resolve, reject) => {
      const ws = state.ws;
      const nonce = state.nonce;
      const id = crypto.randomUUID();
      const timer = setTimeout(() => {
        state.pending.delete(id);
        reject(Object.assign(new Error(T.timeout), { timedOut: true }));
      }, timeout);
      state.pending.set(id, { resolve, reject, timer });
      // Sequence numbers must leave in order, so sealing is serialized.
      state.sendChain = state.sendChain
        .then(async () => {
          if (state.ws !== ws || state.nonce !== nonce) return;
          const frame = await seal(state.key, { nonce, seq: ++state.seq, id, command, project_id: projectId ?? null, args });
          if (state.ws === ws) ws.send(frame);
        })
        .catch(() => {});
    });
  }

  // A phone suspends the page in the background. Back in view, it catches up
  // at once instead of at the next poll, and replaces a socket that died
  // quietly: one that cannot answer in a few seconds is closed and redialled.
  document.addEventListener("visibilitychange", () => {
    if (!state.key) return;
    renderHeader();
    if (document.visibilityState !== "visible") return;
    const ws = state.ws;
    if (!ws) {
      clearTimeout(state.reconnect);
      state.retry = 0;
      connect();
    } else if (state.nonce) {
      // ponytail: a fixed 4 s. A link too slow for that redials once per return; scale it if that shows up.
      call("list_projects", null, {}, 4000).catch((error) => error.timedOut && state.ws === ws && ws.close());
    }
    refreshNow();
  });

  // ----------------------------------------------------------------- route
  // "#<code>/p/<project>/s/<session>/d/<directory>" (or "/f/<file>"). The whole
  // fragment stays in the browser, so Back, reload and bookmarks work without
  // the relay learning the code or what is open.
  function safeDecode(text) {
    try {
      return decodeURIComponent(text);
    } catch {
      return text;
    }
  }
  function readRoute() {
    const parts = location.hash.slice(1).split("/");
    const route = { code: safeDecode(parts[0]), tab: "chats", project: null, session: null, dir: null, file: null };
    const keys = { p: "project", s: "session", d: "dir", f: "file", t: "tab" };
    for (let i = 1; i + 1 < parts.length; i += 2) {
      if (keys[parts[i]] && parts[i + 1]) route[keys[parts[i]]] = safeDecode(parts[i + 1]);
    }
    if (!Object.hasOwn(tabTitles, route.tab)) route.tab = "chats";
    if (route.tab !== "chats") route.project = null;
    if (!route.project) route.session = null;
    if (!route.session || route.file) route.dir = null;
    if (!route.session) route.file = null;
    return route;
  }
  function href(patch) {
    const route = { ...state.route, ...patch };
    let hash = `#${state.code}`;
    if (route.tab && route.tab !== "chats") return `${hash}/t/${route.tab}`;
    if (!route.project) return hash;
    hash += `/p/${encodeURIComponent(route.project)}`;
    if (!route.session) return hash;
    hash += `/s/${encodeURIComponent(route.session)}`;
    if (route.file) return `${hash}/f/${encodeURIComponent(route.file)}`;
    return route.dir ? `${hash}/d/${encodeURIComponent(route.dir)}` : hash;
  }
  const go = (patch) => (location.hash = href(patch));
  const noFiles = { dir: null, file: null };
  const parentDir = (path) => (path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : ".");
  function resolvePath(dir, target) {
    if (target.startsWith("/")) return target;
    const parts = dir === "." ? [] : dir.split("/");
    for (const part of target.split("/")) {
      if (part === "..") parts.pop();
      else if (part && part !== ".") parts.push(part);
    }
    return parts.join("/") || ".";
  }
  const baseName = (path) => path.slice(path.lastIndexOf("/") + 1);

  function applyRoute() {
    closeSheet();
    const prev = state.route;
    const route = (state.route = readRoute());
    const filesOpen = route.dir != null || route.file != null;
    document.body.dataset.pane = filesOpen ? "panel" : route.session ? "chat" : route.project ? "side" : "home";
    document.body.dataset.tab = route.tab;
    $("tabs").hidden = !!route.session;
    for (const link of $("tabs").children) {
      if (link.dataset.tab === route.tab) link.setAttribute("aria-current", "page");
      else link.removeAttribute("aria-current");
    }
    if (route.project !== prev.project) sideView(++gens.side, route.project);
    else markCurrent();
    if (route.project !== prev.project || route.session !== prev.session || route.tab !== prev.tab) {
      const gen = ++gens.chat;
      main.scrollTop = 0;
      $("composer").hidden = true;
      if (route.tab === "calendar") calendarView(gen);
      else if (route.tab === "checklist") checklistView(gen);
      else if (route.tab === "notebook") notebookView(gen);
      else if (!route.project) projectsView(gen);
      else if (!route.session) idleView();
      else chatView(gen, route.project, route.session);
    }
    if (route.session !== prev.session || route.dir !== prev.dir || route.file !== prev.file) {
      panelView(++gens.panel, route);
    }
    renderHeader();
  }

  function renderHeader() {
    const route = state.route;
    const project = state.projects.find((item) => item.id === route.project);
    const session = state.sessions.find((item) => item.id === route.session);
    const host = state.host ? `${state.host.name} · Wisp ${state.host.version}` : "";
    const filesOpen = route.dir != null || route.file != null;
    let title = route.tab === "chats" ? "Wisp" : tabTitles[route.tab];
    let subtitle = "";
    let up = null;
    if (route.file || (route.dir && route.dir !== ".")) {
      const path = route.file || route.dir;
      title = baseName(path);
      subtitle = (session && session.title) || T.files;
      up = { file: null, dir: parentDir(path) };
    } else if (route.session) {
      title = (session && session.title) || T.untitled;
      subtitle = project ? project.name : host;
      up = filesOpen ? noFiles : { session: null, ...noFiles };
    } else if (route.project) {
      title = project ? project.name : T.projects;
      up = { project: null };
    }
    $("title").textContent = title;
    $("subtitle").textContent = subtitle;
    $("project").value = route.project || "";
    // Out of view, the open conversation counts too: the tab title is all there is.
    const others = state.inbox.filter((row) => row.id !== route.session);
    const waiting = document.visibilityState === "hidden" ? state.inbox.length : others.length;
    document.title = `${waiting ? `(${waiting}) ` : ""}${title} · Wisp`;
    const inbox = $("inbox");
    inbox.hidden = !!route.session || route.tab !== "chats" || !!route.project;
    inbox.disabled = !others.length;
    inbox.lastElementChild.hidden = !others.length;
    inbox.lastElementChild.textContent = others.length;
    inbox.setAttribute("aria-label", T.inbox(others.length));
    inbox.onclick = others.length ? () => go({ tab: "chats", project: others[0].project_id, session: others[0].id, ...noFiles }) : null;
    // Keep pending approvals reachable while reading a conversation.
    if (route.session && others.length) inbox.hidden = false;
    const back = $("back");
    back.hidden = !up;
    back.onclick = up && (() => go(up));
    back.setAttribute("aria-label", T.back);
    const files = $("files");
    files.hidden = !route.session;
    files.setAttribute("aria-label", T.files);
    files.setAttribute("aria-pressed", String(filesOpen));
    files.onclick = () => go(filesOpen ? noFiles : { dir: "." });
    $("settings").hidden = !!route.project || route.tab === "calendar" || route.tab === "notebook";
    $("help").hidden = route.tab !== "notebook";
    $("header-new").hidden = !route.project || !!route.session;
    $("header-new").disabled = $("new").disabled;
    $("journey").hidden = !can("native_research_calendar");
    $("journey").href = href({ tab: "calendar", project: null, session: null });
    const hostLabel = $("host-name");
    if (hostLabel) hostLabel.textContent = state.host?.name || T.connecting;
    $("status").title = host;
  }

  // ----------------------------------------------------------------- views
  const wakes = new Set();
  function sleep(ms) {
    return new Promise((resolve) => {
      const wake = () => {
        clearTimeout(timer);
        wakes.delete(wake);
        resolve();
      };
      // Out of view nothing needs to be fresh; coming back wakes every loop.
      const timer = setTimeout(wake, document.visibilityState === "hidden" ? Math.max(ms, 30000) : ms);
      wakes.add(wake);
    });
  }
  const refreshNow = () => [...wakes].forEach((wake) => wake());

  const banner = (error) => el("p", { class: "banner", role: "alert" }, error.message || String(error));

  function when(ts) {
    if (!ts) return "";
    const date = new Date(ts > 1e12 ? ts : ts * 1000);
    return date.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
  }
  function bytes(size) {
    if (size < 1024) return `${size} B`;
    return size < 1048576 ? `${(size / 1024).toFixed(1)} KB` : `${(size / 1048576).toFixed(1)} MB`;
  }

  function statusBadge(status) {
    if (status === "running") return el("span", { class: "badge running" }, T.running);
    if (status === "needs_you") return el("span", { class: "badge needs_you" }, T.needsYou);
    return status === "complete" ? el("span", { class: "badge complete" }, T.done) : null;
  }

  async function refreshProjects() {
    state.projects = await call("list_projects", null);
    const select = $("project");
    select.replaceChildren(...state.projects.map((project) => el("option", { value: project.id }, project.name)));
    renderHeader();
  }

  /// Conversations in any project that wait for an answer or an approval.
  async function watchInbox() {
    for (;;) {
      try {
        if (!state.projects.length) await refreshProjects();
        // The inbox spans every project; the host only needs a valid one to ask through.
        if (state.projects.length && can("native_conversation_inbox")) {
          state.inbox = await call("native_conversation_inbox", state.projects[0].id);
          renderHeader();
        }
      } catch {
        // The next round asks again.
      }
      await sleep(10000);
    }
  }

  async function projectsView(gen) {
    $("composer").hidden = true;
    const list = el("div", { class: "list grouped projects" }, el("p", { class: "empty" }, T.loading));
    // Reachable while the computer is offline, when a reset code is the reason.
    const forget = el("button", { type: "button", class: "quiet", onclick: () => {
      saved.set("");
      location.hash = "";
    } }, T.forget);
    main.replaceChildren(el("h2", { class: "section-label", id: "host-name" }, state.host?.name || T.connecting), list, forget);
    let shown = "";
    while (gen === gens.chat) {
      try {
        await refreshProjects();
        if (gen !== gens.chat) return;
        // An unchanged list keeps its nodes, and with them focus and scroll.
        const data = JSON.stringify(state.projects);
        if (data !== shown) {
          shown = data;
          list.replaceChildren(
            ...(state.projects.length ? state.projects.map((project) =>
              el("a", { class: "row", href: href({ project: project.id, session: null, ...noFiles }) },
                el("span", { class: "main" },
                  el("span", { class: "name" }, project.name),
                  el("span", { class: "sub" }, T.sessionsCount(project.session_count || 0)),
                  project.needs_you_count ? statusBadge("needs_you") : project.running_count ? statusBadge("running") : null),
                icon("chevron-right"))
            ) : [el("p", { class: "empty" }, T.none)]));
        }
      } catch (error) {
        shown = "";
        if (gen === gens.chat) list.replaceChildren(banner(error));
      }
      await sleep(15000);
    }
  }

  function idleView() {
    $("composer").hidden = true;
    main.replaceChildren(el("p", { class: "empty" }, T.pickChat));
  }

  function markCurrent() {
    for (const row of $("sessions").querySelectorAll("a.row")) {
      if (row.dataset.id === state.route.session) row.setAttribute("aria-current", "page");
      else row.removeAttribute("aria-current");
    }
  }

  async function sideView(gen, projectId) {
    const list = $("sessions");
    state.sessions = [];
    list.replaceChildren();
    if (!projectId) return;
    list.replaceChildren(el("p", { class: "empty" }, T.loading));
    let shown = "";
    while (gen === gens.side) {
      try {
        if (!state.projects.length) refreshProjects().catch(() => {});
        const sessions = await call("remote_sessions", projectId);
        if (gen !== gens.side) return;
        state.sessions = sessions;
        const data = JSON.stringify(sessions);
        if (data !== shown) {
          shown = data;
          list.replaceChildren(
            ...(sessions.length ? sessions.map((session) =>
              el("a", { class: "row", "data-id": session.id, href: href({ project: projectId, session: session.id, ...noFiles }) },
                el("span", { class: "main" },
                  el("span", { class: "name" }, session.title || T.untitled),
                  statusBadge(session.status)),
                el("span", { class: "session-date" }, new Date(seconds(session.activity_at || session.ts) * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" })),
                icon("chevron-right"))
            ) : [el("p", { class: "empty" }, T.none)]));
        }
        markCurrent();
        renderHeader();
      } catch (error) {
        shown = "";
        if (gen === gens.side) list.replaceChildren(banner(error));
      }
      await sleep(8000);
    }
  }

  // ----------------------------------------------------- workspace navigation
  const sheetStack = [];
  $("sheet").setAttribute("aria-labelledby", "sheet-title");
  function showSheet(title, body) {
    sheetStack.push({ title, body, focus: document.activeElement });
    renderSheet();
  }
  function renderSheet() {
    const top = sheetStack.at(-1);
    if (!top) { $("sheet").close(); return; }
    $("sheet-title").textContent = top.title;
    $("sheet-body").replaceChildren(top.body);
    if (!$("sheet").open) $("sheet").showModal();
  }
  function closeSheet(all = true) {
    const top = sheetStack.pop();
    if (all) sheetStack.length = 0;
    renderSheet();
    if (top?.focus?.isConnected) top.focus.focus();
  }
  window.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") return;
    if (sheetStack.length) {
      event.preventDefault(); event.stopImmediatePropagation(); closeSheet(false);
    } else if (state.route.dir != null || state.route.file != null) {
      event.preventDefault(); event.stopImmediatePropagation(); go(noFiles);
    }
  }, true);
  $("sheet").addEventListener("cancel", (event) => { event.preventDefault(); closeSheet(false); });
  $("sheet-close").append(icon("close"));
  $("sheet-close").setAttribute("aria-label", tr("关闭", "Close"));
  $("sheet-close").onclick = () => closeSheet(false);
  const action = (label, kind, onclick, props = {}) => el("button", { type: "button", onclick, ...props }, icon(kind), el("span", null, label));
  function setupWorkspace() {
    const tabs = [["chats", "chat", T.conversations], ["calendar", "calendar", tr("日历", "Calendar")],
      ["checklist", "list", tr("清单", "Checklist")], ["notebook", "notebook", tr("记录本", "Notebook")]];
    $("tabs").replaceChildren(...tabs.map(([tab, kind, label]) => el("a", {
      href: href({ tab, project: null, session: null, ...noFiles }), "data-tab": tab,
    }, icon(kind), el("span", null, label))));
    $("settings").append(icon("gear"));
    $("settings").setAttribute("aria-label", tr("设置", "Settings"));
    $("settings").onclick = () => {
      const theme = el("select", { "aria-label": tr("外观", "Appearance") },
        ...[["auto", tr("跟随系统", "System")], ["light", tr("浅色", "Light")], ["dark", tr("深色", "Dark")]]
          .map(([value, label]) => el("option", { value }, label)));
      theme.value = document.documentElement.dataset.theme || "auto";
      theme.onchange = () => {
        document.documentElement.dataset.theme = theme.value;
        try { localStorage.setItem("wisp-remote-theme", theme.value); } catch {}
      };
      showSheet(tr("设置", "Settings"), el("div", { class: "form-stack" },
        el("p", { class: "note" }, state.host ? `${state.host.name} · Wisp ${state.host.version}` : T.connecting),
        el("label", null, tr("外观", "Appearance"), theme),
        el("p", { class: "note" }, tr("清单连接主机后保存；记录本仅保存在当前浏览器。", "Checklist changes are saved on the connected host. Notebook entries stay in this browser.")),
        el("button", { type: "button", onclick: () => { saved.set(""); location.hash = ""; } }, T.forget)));
    };
    $("inbox").firstElementChild.replaceWith(icon("inbox"));
    $("header-new").append(icon("square-pen"));
    $("header-new").setAttribute("aria-label", T.newChat);
    $("header-new").onclick = () => $("new").click();
    $("help").append(icon("circle-help"));
    $("help").setAttribute("aria-label", tr("记录本帮助", "Notebook help"));
    $("help").onclick = notebookHelp;
    $("journey").append(icon("research-trail"), el("span", null, tr("研究历程", "Research history")), icon("chevron-right"));
    $("journey").onclick = () => { calendarProject = state.route.project; };
    try { document.documentElement.dataset.theme = localStorage.getItem("wisp-remote-theme") || "auto"; } catch {}
  }
  const localDay = (date = new Date()) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
  const monthStart = (date) => new Date(date.getFullYear(), date.getMonth(), 1);
  const range = (from, until) => ({ from: Math.floor(from.getTime() / 1000), until: Math.floor(until.getTime() / 1000) });
  const unavailable = () => new Error(tr("当前主机尚未提供此功能，请更新 Wisp 主机后重连。", "Update and reconnect your Wisp host to use this feature."));
  async function requireCapability(command) {
    await whenReady();
    if (!can(command)) throw unavailable();
  }

  // Research records stay in memory. Each day has its own query, so a capped
  // month never silently drops records from the selected day. Local dates use
  // calendar arithmetic rather than 24-hour increments (DST).
  let calendarProject = null;
  async function calendarView(gen) {
    const live = () => gen === gens.chat;
    let month = monthStart(new Date()), selected = new Date(), revision = 0;
    const filters = el("div", { class: "project-filters", "aria-label": T.projects });
    const title = el("h2", { id: "month-title" });
    const grid = el("div", { class: "calendar-grid", id: "calendar-grid", role: "group", "aria-label": tr("选择日期", "Choose a date") });
    const records = el("div", { class: "grouped research-records" });
    const dayHeading = el("h2", { class: "section-label" });
    const feedback = el("div", { "aria-live": "polite" });
    const days = zh ? ["一", "二", "三", "四", "五", "六", "日"] : ["M", "T", "W", "T", "F", "S", "S"];
    const changeMonth = (delta) => { month = new Date(month.getFullYear(), month.getMonth() + delta, 1); selected = month; draw(); };
    const controls = el("div", { class: "month-controls" },
      action(tr("上个月", "Previous month"), "chevron-left", () => changeMonth(-1), { class: "icon", "aria-label": tr("上个月", "Previous month") }),
      el("button", { type: "button", onclick: () => { selected = new Date(); month = monthStart(selected); draw(); } }, tr("今天", "Today")),
      action(tr("下个月", "Next month"), "chevron-right", () => changeMonth(1), { class: "icon", "aria-label": tr("下个月", "Next month") }));
    main.replaceChildren(el("section", { class: "calendar-card grouped" }, filters, title, controls,
      el("div", { class: "weekdays", "aria-hidden": "true" }, days.map(day => el("span", null, day))), grid), feedback, dayHeading, records);
    async function fetchRecords(from, until) {
      const ids = calendarProject ? [calendarProject] : state.projects.map(project => project.id);
      const rows = [];
      for (let offset = 0; offset < ids.length; offset += 50) {
        rows.push(...await call("native_research_calendar", null, { ...range(from, until), project_ids: ids.slice(offset, offset + 50) }));
        if (!live()) return [];
      }
      return rows;
    }
    function showRecords(rows) {
      records.replaceChildren();
      for (const row of rows) {
        const project = state.projects.find(item => item.id === row.project_id);
        if (row.error) records.append(banner(`${project?.name || ""}: ${row.error}`));
        if (row.history.truncated) records.append(banner(tr("记录较多，仅显示部分内容。请在主机查看完整记录。", "Some records are omitted. Open the host for the full history.")));
        for (const entry of row.history.entries) {
          records.append(el("button", { type: "button", class: "row", onclick: () => {
            const body = el("div", { class: "form-stack" }, el("p", { class: "note" }, project?.name || ""),
              el("p", { class: "msg" }, entry.summary || entry.title), el("p", { class: "note" }, when(entry.occurred_at)));
            if (entry.frame_id && !entry.source_discarded) body.append(el("a", { class: "row", href: href({ tab: "chats", project: row.project_id, session: entry.frame_id, ...noFiles }) }, T.conversations, icon("chevron-right")));
            showSheet(entry.title, body);
          } }, el("span", { class: "main" }, el("span", { class: "name" }, entry.title), el("span", { class: "sub" }, project?.name || "")), icon("chevron-right")));
        }
      }
      if (!records.childElementCount) records.append(el("p", { class: "empty" }, tr("这一天还没有研究记录", "No research records for this day")));
    }
    async function draw() {
      const token = ++revision;
      const current = () => live() && token === revision;
      title.textContent = month.toLocaleDateString(undefined, { year: "numeric", month: "long" });
      dayHeading.textContent = selected.toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric", weekday: "long" });
      filters.replaceChildren(...[{ id: null, name: tr("所有项目", "All projects") }, ...state.projects].map(project => el("button", {
        type: "button", "aria-pressed": String(calendarProject === project.id), onclick: () => { calendarProject = project.id; draw(); },
      }, project.name)));
      grid.replaceChildren();
      const offset = (month.getDay() + 6) % 7;
      for (let i = 0; i < offset; i++) grid.append(el("span"));
      const count = new Date(month.getFullYear(), month.getMonth() + 1, 0).getDate();
      for (let day = 1; day <= count; day++) {
        const date = new Date(month.getFullYear(), month.getMonth(), day);
        const key = localDay(date);
        grid.append(el("button", { type: "button", "data-day": key, "aria-label": key,
          "aria-pressed": String(key === localDay(selected)), "aria-current": key === localDay() ? "date" : null,
          onclick: () => { selected = date; draw(); } }, day));
      }
      feedback.replaceChildren();
      records.replaceChildren(el("p", { class: "empty", role: "status" }, T.loading));
      const dayStart = new Date(selected.getFullYear(), selected.getMonth(), selected.getDate());
      const dayEnd = new Date(selected.getFullYear(), selected.getMonth(), selected.getDate() + 1);
      // Independent reads: day results need not wait for every month's project.
      const reads = [
        fetchRecords(month, new Date(month.getFullYear(), month.getMonth() + 1, 1)).then(rows => {
          if (!current()) return;
          const active = new Set(rows.flatMap(row => row.history.entries.map(entry => localDay(new Date(entry.occurred_at * 1000)))));
          for (const button of grid.querySelectorAll("button")) button.classList.toggle("has-records", active.has(button.dataset.day));
          if (rows.some(row => row.history.truncated)) feedback.append(el("p", { class: "note" }, tr("月视图仅显示部分活动标记；选择日期可单独读取。", "Month markers are partial; select a day to load its records.")));
        }).catch(error => { if (current()) feedback.append(banner(error)); }),
        fetchRecords(dayStart, dayEnd).then(rows => { if (current()) showRecords(rows); }).catch(error => { if (current()) records.replaceChildren(banner(error)); }),
      ];
      await Promise.all(reads);
    }
    for (const button of controls.querySelectorAll("button")) button.disabled = true;
    records.append(el("p", { class: "empty" }, T.loading));
    try {
      await requireCapability("native_research_calendar");
      await refreshProjects();
      if (!live()) return;
      if (!state.projects.some(project => project.id === calendarProject)) calendarProject = null;
      for (const button of controls.querySelectorAll("button")) button.disabled = false;
      await draw();
    } catch (error) { if (live()) records.replaceChildren(banner(error)); }
  }

  // The web checklist uses online, revision-checked edits. It does not claim
  // the iOS offline replica's semantics or automatically retry a mutation.
  async function checklistView(gen) {
    const live = () => gen === gens.chat;
    let meta = null, items = [], busy = false, quickID = crypto.randomUUID();
    const feedback = el("div", { "aria-live": "polite" });
    const list = el("div", { class: "checklist-items" }, el("p", { class: "empty" }, T.loading));
    const count = el("span");
    const input = el("input", { id: "checklist-title", maxlength: 1000, placeholder: tr("添加科研事项", "Add a research task"), "aria-label": tr("添加科研事项", "Add a research task") });
    const add = el("button", { type: "submit", disabled: true }, tr("添加", "Add"));
    const form = el("form", { class: "checklist-add", onsubmit: async (event) => {
      event.preventDefault();
      if (!input.value.trim() || busy || !meta) return;
      if (await change({ id: quickID, revision: 0, title: input.value.trim(), day: localDay(), status: "open" })) {
        input.value = ""; quickID = crypto.randomUUID(); syncForm();
      }
    } }, icon("plus"), input, add);
    function syncForm() { add.disabled = busy || !meta || !state.nonce || !input.value.trim(); input.disabled = busy; }
    input.oninput = syncForm;
    const reload = action(tr("刷新", "Refresh"), "refresh", () => load(), { class: "quiet" });
    main.replaceChildren(el("div", { class: "checklist-meta" }, el("span", { id: "host-name" }, state.host?.name || T.connecting), count), feedback, list,
      el("div", { class: "checklist-compose" }, form, el("p", { class: "note" }, tr("点圆圈完成，点事项查看或改期。更改保存到已连接的主机。", "Tap a circle to complete, or a task to edit. Changes are saved to the connected host.")), reload));
    async function load() {
      if (busy) return;
      busy = true; syncForm(); reload.disabled = true;
      try {
        await requireCapability("native_research_checklist_sync");
        let cursor = { epoch: null, cursor: 0, visibility: null }, next = new Map(), more;
        do {
          const reply = await call("native_research_checklist_sync", null, { ...cursor, changes: [] });
          if (!live()) return;
          if (reply.schema !== "wisp.research-checklist.v1") throw unavailable();
          if (reply.reset || (cursor.visibility && cursor.visibility !== reply.visibility)) next.clear();
          for (const item of reply.items) next.set(item.id, item);
          const visible = new Set(reply.visible_project_ids);
          for (const [id, item] of next) if (item.project_id && !visible.has(item.project_id)) next.delete(id);
          cursor = { epoch: reply.epoch, cursor: reply.cursor, visibility: reply.visibility }; more = reply.has_more;
        } while (more);
        meta = cursor; items = [...next.values()]; draw();
      } catch (error) { if (live()) { meta = null; list.replaceChildren(banner(error)); } }
      finally { busy = false; if (live()) { syncForm(); reload.disabled = false; } }
    }
    async function change(item, errors = feedback, conflict) {
      if (busy || !meta) return false;
      if (!state.nonce) { errors.replaceChildren(banner(T.disconnected)); return false; }
      busy = true; syncForm(); errors.replaceChildren();
      for (const button of list.querySelectorAll("button")) button.disabled = true;
      let success = false;
      try {
        const reply = await call("native_research_checklist_sync", null, { ...meta, changes: [{
          mutation_id: crypto.randomUUID(), id: item.id, base_revision: item.revision,
          title: item.title, day: item.day, status: item.status,
        }] });
        const outcome = reply.outcomes.find(outcome => outcome.id === item.id);
        if (reply.reset || outcome?.status !== "applied") {
          if (!reply.reset && outcome?.status === "conflict" && outcome.item && conflict) {
            conflict(outcome.item);
          } else {
            throw new Error(tr("主机上的记录已变化，请刷新清单后重新编辑。你的输入仍保留在这里。", "The host record changed. Refresh the checklist before editing again. Your input is kept here."));
          }
        } else success = true;
      } catch (error) { if (live()) errors.replaceChildren(banner(error)); }
      busy = false;
      if (live()) { if (success) await load(); else { draw(); syncForm(); } }
      return success;
    }
    function edit(item) {
      let revision = item.revision;
      const title = el("input", { value: item.title, maxlength: 1000, required: true, "aria-label": tr("事项", "Task") });
      const day = el("input", { type: "date", value: item.day, required: true, "aria-label": tr("日期", "Date") });
      const errors = el("div");
      const save = el("button", { type: "submit", class: "primary" }, tr("保存", "Save"));
      const editor = el("form", { class: "form-stack", onsubmit: async event => {
        event.preventDefault();
        if (!title.value.trim() || !day.value) return;
        save.disabled = true;
        if (await change({ ...item, revision, title: title.value.trim(), day: day.value }, errors, server => {
          errors.replaceChildren(banner(tr("此事项已在主机更新，请选择保留哪一版。", "This task changed on the host. Choose which version to keep.")),
            el("p", { class: "msg" }, `${server.title} · ${server.day}`),
            el("button", { type: "button", onclick: () => {
              revision = server.revision; errors.replaceChildren();
              // Explicitly choosing the local version enables a new save; it
              // does not send a second mutation by itself.
            } }, tr("保留我的修改", "Keep my changes")),
            el("button", { type: "button", onclick: async () => {
              closeSheet(false); await load();
            } }, tr("采用主机版本", "Use host version")));
        }) && save.isConnected) closeSheet(false);
        save.disabled = false;
      } }, el("label", null, tr("事项", "Task"), title), el("label", null, tr("日期", "Date"), day), errors, save);
      showSheet(tr("科研事项", "Research task"), editor);
    }
    function draw() {
      const today = localDay();
      const open = items.filter(item => item.status === "open").sort((a, b) => a.day.localeCompare(b.day));
      count.textContent = tr(`${open.length} 项待办`, `${open.length} to do`);
      const row = item => el("div", { class: "checklist-row" },
        action(item.status === "done" ? tr("重新打开", "Reopen") : tr("标记完成", "Complete"), item.status === "done" ? "circle-check" : "circle",
          () => change({ ...item, status: item.status === "done" ? "open" : "done" }), { class: "task-toggle", "aria-label": `${item.status === "done" ? tr("重新打开", "Reopen") : tr("完成", "Complete")} ${item.title}`, "aria-pressed": String(item.status === "done") }),
        el("button", { type: "button", class: "task-text", onclick: () => edit(item) }, el("span", null, item.title), item.day !== today ? el("small", { class: item.day < today ? "overdue" : "" }, item.day) : null));
      list.replaceChildren();
      for (const [label, entries, overdue] of [[tr("逾期", "Overdue"), open.filter(item => item.day < today), true], [tr("今天", "Today"), open.filter(item => item.day === today)], [tr("接下来", "Upcoming"), open.filter(item => item.day > today)]]) {
        if (entries.length) list.append(el("section", null, el("h2", { class: overdue ? "overdue" : "" }, label, el("small", null, entries.length)), ...entries.map(row)));
      }
      if (!open.length) list.append(el("p", { class: "empty" }, tr("暂无待办，记下下一步研究计划吧。", "Nothing to do yet. Add your next research step.")));
      const done = items.filter(item => item.status === "done");
      if (done.length) list.append(el("details", { class: "completed-tasks" }, el("summary", null, `${tr("已完成", "Completed")} · ${done.length}`), ...done.map(row)));
    }
    await load();
  }

  // Browser-local notebook: original photos and text are stored atomically in
  // IndexedDB. No host upload, OCR, or cross-device synchronization is implied.
  let notebookDB;
  function notebookStore(mode, operation) {
    if (!notebookDB) notebookDB = new Promise((resolve, reject) => {
      const request = indexedDB.open("wisp-remote-notebook", 1);
      request.onupgradeneeded = () => request.result.createObjectStore("entries", { keyPath: "id" });
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
      request.onblocked = () => reject(new Error(tr("请关闭其他 Wisp 网页后重试。", "Close other Wisp pages and try again.")));
    }).catch(error => { notebookDB = null; throw error; });
    return notebookDB.then(db => new Promise((resolve, reject) => {
      const tx = db.transaction("entries", mode);
      const request = operation(tx.objectStore("entries"));
      tx.oncomplete = () => resolve(request.result);
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error || new Error("Notebook transaction aborted"));
    }));
  }
  function notebookHelp() {
    showSheet(tr("关于实验记录本", "About the notebook"), el("p", null, tr(
      "文字和照片原图保存在当前浏览器，无需主机在线。清除网站数据会移除这些记录。Web 端暂不支持文字识别，也不会自动发送到主机；重要记录请保留备份。",
      "Text and original photos stay in this browser, even when the host is offline. Clearing website data removes these records. OCR and sending to the host are not supported here yet; keep a backup of important records.")));
  }
  async function notebookView(gen) {
    const live = () => gen === gens.chat;
    const list = el("div", { class: "grouped notebook-entries" });
    const feedback = el("div", { "aria-live": "polite" });
    const tile = (kind, title, subtitle, onclick) => el("button", { type: "button", class: "row notebook-action", onclick },
      icon(kind), el("span", { class: "main" }, el("span", { class: "name" }, title), el("span", { class: "sub" }, subtitle)), icon("chevron-right"));
    main.replaceChildren(el("h2", { class: "section-label notebook-question" }, tr("你现在想做什么？", "What would you like to do?")),
      el("div", { class: "grouped notebook-actions" },
        tile("camera", tr("拍下纸质实验记录", "Capture a paper record"), tr("保留原图，补充文字说明", "Keep the original and add your notes"), () => editNote(null, true)),
        tile("edit", tr("记下刚才的观察", "Write an observation"), tr("写过程、现象，也可以附上照片", "Record what happened, with optional photos"), () => editNote()),
        tile("book", tr("先看一条示例", "See an example"), tr("了解一份完整记录是什么样", "Explore a sample lab note"), () => showSheet(tr("实验记录示例", "Example lab note"),
          el("div", { class: "form-stack" }, el("h3", null, tr("根尖样本观察", "Root tip observation")), el("p", { class: "msg" }, tr("目的：比较处理组与对照组的根尖形态。\n过程：同一光照条件下观察，每组取 3 个样本。\n观察：处理组根毛较密，需补充重复实验。\n下一步：记录长度并拍摄同倍率照片。", "Aim: compare treated and control root tips.\nMethod: observe 3 samples per group under the same lighting.\nObservation: denser root hairs in the treated group; more replicates needed.\nNext: measure lengths and photograph at the same magnification.")), el("p", { class: "note" }, tr("这是一条示例，不会保存到你的记录中。", "This example is not saved to your notebook.")))))),
      el("p", { class: "notebook-note" }, tr("无需连接主机。文字和照片仅保存在当前浏览器，不会自动上传。", "No host connection needed. Text and photos stay in this browser and are never uploaded automatically.")),
      el("h2", { class: "section-label" }, tr("本机实验记录", "On this device")), feedback, list);
    async function load() {
      try {
        const entries = await notebookStore("readonly", store => store.getAll());
        if (!live()) return;
        entries.sort((a, b) => b.updated - a.updated);
        list.replaceChildren(...(entries.length ? entries.map(entry => el("button", { type: "button", class: "row", onclick: () => editNote(entry) },
          el("span", { class: "main" }, el("span", { class: "name" }, entry.title || tr("未命名记录", "Untitled note")), el("span", { class: "sub" }, entry.day)), icon("chevron-right"))) :
          [el("p", { class: "notebook-empty" }, tr("第一条记录会出现在这里。从上方选择拍照或写观察，就能开始。", "Your first note will appear here. Start with a photo or an observation above."))]));
      } catch (error) { if (live()) feedback.replaceChildren(banner(error)); }
    }
    function editNote(entry, camera = false) {
      const id = entry?.id || crypto.randomUUID();
      const title = el("input", { value: entry?.title || "", placeholder: tr("标题（可稍后补）", "Title (optional)"), "aria-label": tr("标题", "Title") });
      const day = el("input", { type: "date", value: entry?.day || localDay(), required: true, "aria-label": tr("日期", "Date") });
      const text = el("textarea", { rows: 7, placeholder: tr("记录过程、现象和观察…", "Record your process and observations…"), "aria-label": tr("观察记录", "Observation") }, entry?.text || "");
      const photos = [...(entry?.photos || [])];
      const previews = el("div", { class: "notebook-photos" });
      const errors = el("div", { "aria-live": "polite" });
      const picker = el("input", { type: "file", accept: "image/jpeg,image/png,image/webp", multiple: true, hidden: true, id: "notebook-photo" });
      if (camera) picker.setAttribute("capture", "environment");
      const save = el("button", { type: "submit", class: "primary" }, tr("完成并保存", "Save note"));
      const addPhoto = action(tr("添加照片", "Add photo"), "camera", () => picker.click());
      const renderPhotos = () => previews.replaceChildren(...photos.map((photo, i) => el("div", null,
        el("img", { src: photo.data, alt: photo.name }), el("button", { type: "button", onclick: () => { photos.splice(i, 1); renderPhotos(); } }, T.removePhoto))));
      picker.onchange = async () => {
        const files = [...picker.files]; picker.value = ""; save.disabled = true; addPhoto.disabled = true; errors.replaceChildren();
        try {
          for (const file of files) {
            if (photos.length >= 12 || file.size > 20 * 1024 * 1024 || !["image/jpeg", "image/png", "image/webp"].includes(file.type)) throw new Error(tr("最多保存 12 张照片，每张不超过 20 MB，支持 JPEG、PNG 和 WebP。", "Use up to 12 JPEG, PNG or WebP photos, at most 20 MB each."));
            const data = await new Promise((resolve, reject) => { const reader = new FileReader(); reader.onload = () => resolve(reader.result); reader.onerror = () => reject(reader.error); reader.readAsDataURL(file); });
            photos.push({ name: file.name, data }); renderPhotos();
          }
        } catch (error) { errors.replaceChildren(banner(error)); }
        save.disabled = false; addPhoto.disabled = false;
      };
      renderPhotos();
      showSheet(tr("实验记录", "Lab note"), el("form", { class: "form-stack", onsubmit: async event => {
        event.preventDefault();
        if (!text.value.trim() && !title.value.trim() && !photos.length) {
          errors.replaceChildren(banner(tr("请先写下观察或添加照片。", "Add an observation or a photo first."))); return;
        }
        save.disabled = true; errors.replaceChildren();
        try {
          await notebookStore("readwrite", store => store.put({ id, title: title.value.trim(), day: day.value, text: text.value, photos, updated: Date.now() }));
          if (save.isConnected) closeSheet(false);
          if (live()) await load();
        } catch (error) { errors.replaceChildren(banner(error)); }
        save.disabled = false;
      } }, title, day, text, picker, previews, addPhoto, errors, save,
        el("p", { class: "note" }, tr("仅保存在当前浏览器", "Saved only in this browser"))));
      if (camera) picker.click();
    }
    await load();
  }

  // -------------------------------------------------------------- markdown
  // A small renderer for what replies actually contain: headings, lists,
  // tables, code and links. It only ever builds text nodes and a fixed set
  // of elements, so there is nothing to sanitize.
  const LIST = /^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/;
  const FENCE = /^\s*(`{3,}|~{3,})/;
  const TABLE_RULE = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;
  const INLINE = /(`+)(.+?)\1|!\[([^\]]*)\]\(\s*(?:<([^>]+)>|([^)\s]+))(?:\s+"[^"]*")?\s*\)|\[([^\]]+)\]\(\s*(?:<([^>]+)>|([^)\s]+))(?:\s+"[^"]*")?\s*\)|\*\*(.+?)\*\*|\*(?![\s*])([^*]+?)\*|~~(.+?)~~|(https?:\/\/[^\s<>]*[^\s<>.,;:!?)\]}'"])/g;

  /// A project-relative path, or null for URLs and anything with a scheme.
  function localPath(reference) {
    const text = (reference || "").trim();
    if (!text || text.startsWith("//") || text.startsWith("#") || /^[a-z][a-z0-9+.-]*:/i.test(text)) return null;
    return safeDecode(text).replace(/^\.\//, "");
  }

  function mdInline(text, ctx) {
    const out = [];
    const plain = (value) => out.push(value.replace(/\\([\\`*_{}[\]()#+\-.!|~>])/g, "$1"));
    let last = 0;
    for (const m of text.matchAll(INLINE)) {
      if (m.index > last) plain(text.slice(last, m.index));
      last = m.index + m[0].length;
      if (m[1]) out.push(el("code", null, m[2].trim() || m[2]));
      else if (m[3] !== undefined) out.push(ctx.image ? ctx.image(m[4] || m[5], m[3]) : m[3]);
      else if (m[6] !== undefined) {
        const target = m[7] || m[8];
        const path = localPath(target);
        const label = mdInline(m[6], ctx);
        if (/^(https?:|mailto:)/i.test(target)) out.push(el("a", { href: target, target: "_blank", rel: "noopener noreferrer" }, label));
        else if (path && ctx.file) out.push(el("a", { href: ctx.file(path) }, label));
        else out.push(...label);
      } else if (m[9] !== undefined) out.push(el("strong", null, mdInline(m[9], ctx)));
      else if (m[10] !== undefined) out.push(el("em", null, mdInline(m[10], ctx)));
      else if (m[11] !== undefined) out.push(el("del", null, mdInline(m[11], ctx)));
      else out.push(el("a", { href: m[12], target: "_blank", rel: "noopener noreferrer" }, m[12]));
    }
    if (last < text.length) plain(text.slice(last));
    return out;
  }

  function markdown(source, ctx = {}) {
    const lines = String(source || "").replace(/\r\n?/g, "\n").split("\n");
    const out = el("div", { class: "md" });
    const cells = (row) =>
      row.trim().replace(/\\\|/g, "\u0000").replace(/^\|/, "").replace(/\|$/, "").split("|").map((cell) => cell.trim().replace(/\u0000/g, "|"));
    // Consumes a fenced block starting at `at`; returns the node and next line.
    const fence = (at) => {
      const mark = lines[at].match(FENCE)[1];
      const body = [];
      let i = at + 1;
      while (i < lines.length && !lines[i].trim().startsWith(mark)) body.push(lines[i++]);
      return [el("pre", null, el("code", null, body.join("\n"))), i + 1];
    };
    const table = (i) => lines[i].includes("|") && (lines[i + 1] || "").includes("|") && TABLE_RULE.test(lines[i + 1]);
    const blockStart = (i) => FENCE.test(lines[i]) || /^\s{0,3}(#{1,6}\s|>)/.test(lines[i]) || LIST.test(lines[i]) || table(i);
    let i = 0;
    while (i < lines.length) {
      const line = lines[i];
      let m;
      if (!line.trim()) {
        i++;
      } else if (FENCE.test(line)) {
        const [node, next] = fence(i);
        out.append(node);
        i = next;
      } else if ((m = line.match(/^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/))) {
        out.append(el(`h${Math.min(6, m[1].length + 2)}`, null, mdInline(m[2], ctx)));
        i++;
      } else if (/^\s{0,3}([-*_])(\s*\1){2,}\s*$/.test(line)) {
        out.append(el("hr"));
        i++;
      } else if (/^\s{0,3}>/.test(line)) {
        const body = [];
        while (i < lines.length && /^\s{0,3}>/.test(lines[i])) body.push(lines[i++].replace(/^\s{0,3}>\s?/, ""));
        out.append(el("blockquote", null, ...markdown(body.join("\n"), ctx).childNodes));
      } else if (LIST.test(line)) {
        // Indentation decides nesting; `stack` is the open lists, outermost first.
        const stack = [];
        for (; i < lines.length; i++) {
          const item = lines[i].match(LIST);
          let top = stack[stack.length - 1];
          if (item) {
            const indent = item[1].replace(/\t/g, "    ").length;
            while (stack.length > 1 && indent < top.indent) top = (stack.pop(), stack[stack.length - 1]);
            if (!top || indent > top.indent) {
              const ordered = /\d/.test(item[2]);
              const list = el(ordered ? "ol" : "ul", ordered && parseInt(item[2], 10) !== 1 ? { start: parseInt(item[2], 10) } : null);
              (top ? top.item : out).append(list);
              stack.push((top = { indent, list, item: null }));
            }
            top.list.append((top.item = el("li", null, mdInline(item[3], ctx))));
          } else if (!lines[i].trim()) {
            const next = lines[i + 1];
            if (next == null || !(LIST.test(next) || /^\s+\S/.test(next))) break;
          } else if (/^\s/.test(lines[i]) && FENCE.test(lines[i])) {
            const [node, next] = fence(i);
            top.item.append(node);
            i = next - 1;
          } else if (/^\s/.test(lines[i])) {
            top.item.append("\n", ...mdInline(lines[i].trim(), ctx));
          } else break;
        }
      } else if (table(i)) {
        const head = cells(line);
        const body = el("tbody");
        for (i += 2; i < lines.length && lines[i].includes("|") && lines[i].trim(); i++) {
          const row = cells(lines[i]);
          body.append(el("tr", null, head.map((_, column) => el("td", null, mdInline(row[column] || "", ctx)))));
        }
        const header = el("thead", null, el("tr", null, head.map((cell) => el("th", null, mdInline(cell, ctx)))));
        out.append(el("div", { class: "table-wrap" }, el("table", null, header, body)));
      } else {
        const body = [line];
        for (i++; i < lines.length && lines[i].trim() && !blockStart(i); i++) body.push(lines[i]);
        out.append(el("p", null, mdInline(body.join("\n"), ctx)));
      }
    }
    return out;
  }

  // ---------------------------------------------------------------- images
  // The desktop only ever sends a bounded PNG thumbnail. Loaded images are
  // remembered so a transcript refresh does not fetch or flash them again.
  const imageCache = new Map();
  const IMAGE_EXT = new Set(["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"]);
  // Markdown itself, and the documents the desktop already converts to it.
  const MARKDOWN_EXT = new Set(["md", "markdown", "pdf", "doc", "docx", "docm", "odt", "rtf", "epub", "ppt", "pptx", "odp", "xls", "xlsx", "ods"]);
  const extension = (path) => (/\.([^./\\]+)$/.exec(path) || ["", ""])[1].toLowerCase();

  function loadImage(project, session, source) {
    const key = `${project}\n${session}\n${source.resource_id || ""}\n${source.path || ""}`;
    let pending = imageCache.get(key);
    if (!pending) {
      pending = call("native_conversation_image", project, { session_id: session, resource_id: source.resource_id || null, path: source.path || null })
        .then((content) => {
          if (!content || !content.base64) throw new Error(T.imageFailed);
          return `data:${content.mime || "image/png"};base64,${content.base64}`;
        });
      pending.catch(() => imageCache.delete(key));
      imageCache.set(key, pending);
      if (imageCache.size > 24) imageCache.delete(imageCache.keys().next().value);
    }
    return pending;
  }

  /// `pin` is asked before the picture takes up room and returns what to run
  /// once it has, so a late image cannot push the newest text out of view.
  function imageNode(project, session, source, label, pin) {
    const caption = el("span", { class: "cap" }, label || T.loading);
    const node = el("span", { class: "img" }, caption);
    const fail = (error) => (caption.textContent = `${label ? `${label} · ` : ""}${(error && error.message) || T.imageFailed}`);
    if (!source) fail();
    else {
      loadImage(project, session, source).then((url) => {
        const image = el("img", { src: url, alt: label || "" });
        if (pin) image.addEventListener("load", pin());
        caption.textContent = label || "";
        node.prepend(image);
      }, fail);
    }
    return node;
  }

  /// A Markdown image is a captured message resource when the desktop bound
  /// one, else a project file; remote URLs are never fetched.
  function imageSource(item, reference) {
    const path = localPath(reference);
    const resource = (item.resources || []).find((entry) =>
      entry.originalReference === reference || (path && localPath(entry.originalReference) === path));
    if (resource) return resource.status === "ready" && resource.artifactVersionId ? { resource_id: resource.id } : null;
    return path ? { path } : null;
  }

  // ------------------------------------------------------------ transcript
  /// `opened` remembers only what the reader changed, so a disclosure that
  /// opens by itself while it is live closes again once it settles.
  function fold(key, opened, auto, cls, summary, body) {
    const node = el("details", { class: cls, open: opened.get(key) ?? auto }, el("summary", null, summary), body);
    node.addEventListener("toggle", () => (node.open === auto ? opened.delete(key) : opened.set(key, node.open)));
    return node;
  }

  function toolNode(item, key, opened, extra) {
    const ok = item.ok === true ? T.done : item.ok === false ? T.failed : T.running;
    const body = [item.input, item.text].filter(Boolean).join("\n\n").slice(0, 20000);
    const node = fold(key, opened, false, "tool", [
      el("span", null, item.tool_name || item.kind || item.role),
      el("span", { class: item.ok === false ? "state fail" : "state" }, ok),
    ], body ? el("pre", null, body) : null);
    return extra ? el("div", { class: "transcript" }, node, extra) : node;
  }

  // Tools whose own row is the result worth seeing; they never fold away.
  const SHOWN_TOOLS = new Set(["attempt_completion", "monitor_run", "wisp_monitor_run", "generate_image", "generate_video"]);
  const isTool = (item) => (item.role === "tool" || item.role === "acp_tool") && !SHOWN_TOOLS.has(item.tool_name);

  /// Assistant text that introduces more work, not the turn's answer.
  function isCommentary(items, index) {
    for (let next = index + 1; next < items.length; next++) {
      const item = items[next];
      if (item.role !== "reasoning" && (item.role !== "assistant" || item.text)) return isTool(item);
    }
    return false;
  }

  /// Folds process rows as the desktop does: a finished turn keeps one row for
  /// all of its work; a running one keeps its commentary readable and folds
  /// each run of steps, with the newest open. `nodes[i]` renders `items[i]`.
  function foldSteps(items, nodes, running, opened) {
    const liveFrom = running ? items.map((item) => item.role).lastIndexOf("user") : items.length;
    const out = [];
    for (let start = 0; start < items.length; ) {
      const live = start > liveFrom;
      let end = start;
      for (; end < items.length; end++) {
        const item = items[end];
        if (item.role === "assistant" ? item.text && (live || !isCommentary(items, end)) : item.role !== "reasoning" && !isTool(item)) break;
      }
      const rows = nodes.slice(start, end).filter(Boolean);
      if (rows.length < 2) {
        end = Math.max(end, start + 1);
        out.push(...nodes.slice(start, end));
      } else {
        // ponytail: position-keyed and rebuilt per refresh; the rows inside keep their nodes.
        const auto = live && end === items.length;
        const tools = items.slice(start, end).filter(isTool).length;
        out.push(fold(`${live ? "steps" : "turn"}:${start}`, opened, auto, "tool steps",
          el("span", null, auto ? T.working : tools ? T.steps(tools) : T.processed),
          el("div", { class: "transcript" }, rows)));
      }
      start = end;
    }
    return out.filter(Boolean);
  }

  function itemNode(item, index, opened, ctx) {
    const key = item.call_id || `i${index}`;
    switch (item.role) {
      case "user": {
        // Pictures sent with the message are shown; the line naming them is not.
        const sent = item.attachments || [];
        const pictures = sent.filter((path) => IMAGE_EXT.has(extension(path)));
        const text = pictures.length === sent.length
          ? item.text.split("\n\n").filter((block) => !block.startsWith("Uploaded files: ")).join("\n\n")
          : item.text;
        return el("div", { class: "msg user" }, text, pictures.map((path) => ctx.image({ path }, baseName(path))));
      }
      case "assistant":
        if (!item.text) return null;
        return markdown(item.text, {
          file: ctx.file,
          image: (reference, alt) => ctx.image(imageSource(item, reference), alt || baseName(localPath(reference) || "")),
        });
      case "tool":
      case "acp_tool": {
        // A generated picture is the result worth seeing, as on the desktop.
        const made = item.tool_name === "generate_image" && item.ok === true && localPath(item.input);
        const picture = made && IMAGE_EXT.has(extension(made)) ? ctx.image({ path: made }, baseName(made)) : null;
        return toolNode(item, key, opened, picture);
      }
      case "reasoning":
        return toolNode({ ...item, tool_name: T.reasoning, ok: true }, key, opened);
      case "plan":
        return item.proposal
          ? el("div", { class: "msg note" }, item.proposal.entries.map((entry) => `• ${entry.content}`).join("\n"))
          : null;
      case "question":
      case "system":
        return item.text ? el("div", { class: "msg note" }, item.text) : null;
      default:
        return null;
    }
  }

  function approvalNode(approval, session, project) {
    const decide = (approved) => async (event) => {
      for (const button of event.currentTarget.parentElement.querySelectorAll("button")) button.disabled = true;
      try {
        await call("native_conversation_approve", project, { session_id: session, approval_id: approval.approval_id, approved });
      } catch (error) {
        main.prepend(banner(error));
      }
      refreshNow();
    };
    return el("section", { class: "approval" },
      el("h3", null, T.approvalTitle + approval.tool),
      approval.message ? el("div", { class: "msg" }, approval.message) : null,
      approval.preview ? el("pre", null, approval.preview) : null,
      el("div", { class: "actions" },
        el("button", { type: "button", class: "danger", onclick: decide(false) }, T.reject),
        el("button", { type: "button", class: "primary", onclick: decide(true) }, T.approve)));
  }

  // --------------------------------------------------------------------- runs
  const RUN_OVER = new Set(["succeeded", "failed", "cancelled", "timed_out", "lost"]);
  const RUN_FAILED = new Set(["failed", "timed_out", "lost"]);
  const seconds = (ts) => (ts > 1e12 ? ts / 1000 : ts);
  function elapsed(from, to) {
    const total = Math.max(0, Math.round(seconds(to) - seconds(from)));
    const [h, m, sec] = [Math.floor(total / 3600), Math.floor(total / 60) % 60, total % 60];
    return h ? `${h}h ${m}m` : m ? `${m}m ${sec}s` : `${sec}s`;
  }

  /// A Run this conversation started: its state and the end of its output.
  /// It is updated in place, so a reader keeps their place while output grows.
  function runNode(card, opened) {
    const [name, time, status, pre] = [el("span", { class: "name" }), el("span", { class: "sub" }), el("span"), el("pre")];
    const node = fold(`run:${card.id}`, opened, !RUN_OVER.has(card.status), "tool run", [name, time, status, icon("chevron-right")], pre);
    node.addEventListener("toggle", () => node.open && (pre.scrollTop = pre.scrollHeight));
    const update = (card) => {
      const over = RUN_OVER.has(card.status);
      const failed = RUN_FAILED.has(card.status);
      const output = [card.stdout_tail, failed && card.stderr_tail, card.last_poll_error].filter(Boolean).join("\n\n").slice(-8000);
      const text = [card.command && `$ ${card.command}`, output].filter(Boolean).join("\n\n");
      const atEnd = pre.scrollHeight - pre.scrollTop - pre.clientHeight < 24;
      name.textContent = card.title || card.kind;
      time.textContent = card.started_at ? elapsed(card.started_at, card.ended_at || Date.now()) : "";
      status.className = failed ? "state fail" : over ? "state" : "state live";
      status.textContent = T.runState[card.status] || card.status;
      if (pre.textContent !== text) {
        pre.textContent = text;
        if (atEnd) pre.scrollTop = pre.scrollHeight;
      }
    };
    update(card);
    return { node, update };
  }

  // ----------------------------------------------------------------- pictures
  const MAX_PHOTOS = 4;
  // A picture travels in one sealed browser frame, which the relay caps at
  // 1 MiB. Base64 inside base64 leaves room for about this much image.
  // ponytail: one frame per picture; chunked upload (#1460) lifts the limit.
  const MAX_PHOTO_BYTES = 560 * 1024;

  /// Redraws a picture as a JPEG, smaller each time until it fits one frame.
  async function shrink(file) {
    const bitmap = await createImageBitmap(file).catch(() => {
      throw new Error(T.photoFailed);
    });
    for (let side = 1600, quality = 0.85; side >= 400; side = Math.round(side * 0.7), quality = 0.75) {
      const scale = Math.min(1, side / Math.max(bitmap.width, bitmap.height));
      const canvas = el("canvas", { width: Math.max(1, Math.round(bitmap.width * scale)), height: Math.max(1, Math.round(bitmap.height * scale)) });
      const context = canvas.getContext("2d");
      context.fillStyle = "#fff"; // JPEG has no transparency.
      context.fillRect(0, 0, canvas.width, canvas.height);
      context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
      const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/jpeg", quality));
      if (blob && blob.size <= MAX_PHOTO_BYTES) return blob;
    }
    throw new Error(T.photoFailed);
  }

  // --------------------------------------------------------------------- chat
  async function chatView(gen, project, session) {
    const live = () => gen === gens.chat;
    main.replaceChildren(el("p", { class: "empty" }, T.loading));
    $("composer").hidden = false;
    const draft = $("draft");
    const send = $("send");
    const stop = $("stop");
    const hint = $("composer-hint");
    const attach = $("attach");
    const picker = $("photo");
    const strip = $("photos");
    // Keep a draft across reconnects, never across conversations.
    if (draft.dataset.session !== session) {
      draft.value = "";
      draft.style.height = "";
      draft.dataset.session = session;
    }
    draft.placeholder = T.placeholder;
    stop.textContent = T.stop;
    attach.setAttribute("aria-label", T.attach);
    attach.replaceChildren(icon("image"), el("span", null, tr("图片", "Photo")));
    const opened = new Map();
    const rendered = [];
    const runCards = new Map();
    let lastItems = "";
    let lastExtras = "";
    let snapshot = null;
    let sending = false;
    let uploading = 0;
    let photos = [];
    let failed = false;
    let wasRunning = false;
    let unseen = false;
    let head; // Where the latest page starts; older pages are fetched from `cursor`.
    let cursor = null;
    const earlier = el("button", { type: "button", class: "more", hidden: true }, T.earlier);
    const older = el("div", { class: "transcript" }, earlier);
    const transcript = el("div", { class: "transcript" });
    const runs = el("div", { class: "transcript" });
    const finishedList = el("div", { class: "transcript" });
    const finishedLabel = el("summary");
    const finishedRuns = el("details", { class: "tool completed-runs" }, finishedLabel, finishedList);
    const approvals = el("div", { class: "transcript" });
    const activity = el("p", { class: "activity" });
    const chips = el("div", { class: "chips", role: "group", "aria-label": T.suggestions });
    const queue = el("div", { class: "transcript" });
    const errors = el("div");
    const nearBottom = () => main.scrollHeight - main.scrollTop - main.clientHeight < 120;
    const toBottom = () => (main.scrollTop = main.scrollHeight);
    const pin = () => {
      const stick = nearBottom();
      return () => stick && live() && toBottom();
    };
    const ctx = {
      file: (path) => href({ dir: null, file: path }),
      image: (source, label) => imageNode(project, session, source, label, pin),
    };
    const seen = () => call("native_conversation_seen", project, { session_id: session }).catch(() => {});
    const fail = (error) => live() && errors.replaceChildren(banner(error));

    const sync = () => {
      const blocked = !snapshot || snapshot.read_only || !!snapshot.acp_agent_id;
      const running = !!snapshot && snapshot.running;
      // While a turn runs, the next message waits its turn on the computer.
      const queues = running && can("native_conversation_enqueue");
      draft.disabled = blocked;
      send.hidden = running && !queues;
      send.replaceChildren(queues ? T.queue : T.send, icon("arrow-up"));
      send.disabled = blocked || sending || uploading > 0 || !(draft.value.trim() || photos.length);
      attach.hidden = !can("remote_attach_image");
      attach.disabled = blocked || photos.length + uploading >= MAX_PHOTOS;
      stop.hidden = !running;
      stop.disabled = !!snapshot && snapshot.stopping;
      hint.hidden = false;
      hint.textContent = !snapshot ? "" : snapshot.read_only ? T.readOnly : snapshot.acp_agent_id ? T.acp : T.remoteAsk;
    };
    const showPhotos = () => {
      strip.hidden = !photos.length;
      strip.replaceChildren(...photos.map((photo) =>
        el("span", { class: "photo" },
          el("img", { src: photo.url, alt: "" }),
          el("button", { type: "button", "aria-label": T.removePhoto, onclick: () => {
            photos = photos.filter((other) => other !== photo);
            showPhotos();
            sync();
          } }, icon("close")))));
    };
    showPhotos();
    draft.oninput = () => {
      draft.style.height = "auto";
      draft.style.height = `${draft.scrollHeight}px`;
      sync();
    };
    attach.onclick = () => picker.click();
    picker.onchange = async () => {
      const files = [...picker.files];
      picker.value = "";
      errors.replaceChildren();
      for (const file of files) {
        if (photos.length >= MAX_PHOTOS) {
          fail(new Error(T.photoLimit(MAX_PHOTOS)));
          break;
        }
        uploading++;
        sync();
        try {
          const blob = await shrink(file);
          const base64 = b64(new Uint8Array(await blob.arrayBuffer()));
          const attached = await call("remote_attach_image", project, { session_id: session, base64 });
          if (!live()) return;
          photos = [...photos, { path: attached.path, url: `data:image/jpeg;base64,${base64}` }];
          showPhotos();
        } catch (error) {
          fail(error);
        }
        uploading--;
        if (live()) sync();
      }
    };
    send.onclick = async () => {
      const message = draft.value.trim();
      const sent = photos;
      if ((!message && !sent.length) || sending || uploading) return;
      const args = { session_id: session, request_id: crypto.randomUUID(), message };
      if (sent.length) args.attachments = sent.map((photo) => photo.path);
      const queues = snapshot && snapshot.running && can("native_conversation_enqueue");
      sending = true;
      sync();
      errors.replaceChildren();
      try {
        await call(queues ? "native_conversation_enqueue" : "native_conversation_send", project, args);
        if (live()) {
          draft.value = "";
          photos = photos.filter((photo) => !sent.includes(photo));
          showPhotos();
          draft.oninput();
        }
      } catch (error) {
        fail(error); // The draft and its pictures are kept.
      }
      sending = false;
      if (live()) sync();
      refreshNow();
    };
    stop.onclick = async () => {
      stop.disabled = true;
      try {
        await call("native_conversation_stop", project, { session_id: session });
      } catch (error) {
        fail(error);
      }
      refreshNow();
    };
    draft.onkeydown = (event) => {
      if (event.key === "Enter" && !event.shiftKey && !event.isComposing && matchMedia("(pointer: fine)").matches) {
        event.preventDefault();
        send.onclick();
      }
    };
    earlier.onclick = async () => {
      earlier.disabled = true;
      try {
        const page = await call("native_conversation_snapshot", project, { session_id: session, before_seq: cursor });
        if (!live()) return;
        const folds = new Map();
        const nodes = page.items.map((item, index) => itemNode(item, index, folds, ctx));
        const height = main.scrollHeight;
        earlier.after(el("div", { class: "transcript" }, foldSteps(page.items, nodes, false, folds)));
        cursor = page.next_before_seq ?? null;
        earlier.hidden = cursor == null;
        main.scrollTop += main.scrollHeight - height; // The reader stays where they were.
      } catch (error) {
        fail(error);
      }
      earlier.disabled = false;
    };
    const cancelQueued = (item) => async (event) => {
      event.currentTarget.disabled = true;
      try {
        await call("native_conversation_queue_action", project, { session_id: session, id: item.id, digest: item.digest, action: { kind: "cancel" } });
      } catch (error) {
        fail(error);
      }
      refreshNow();
    };
    sync();

    let first = true;
    while (live()) {
      let waiting = [];
      try {
        const next = await call("native_conversation_snapshot", project, { session_id: session });
        if (!live()) return;
        snapshot = next;
        const stick = first || nearBottom();
        // The latest page moves on with each new turn. Older pages fetched
        // behind the previous one would leave a gap, so they start over.
        const start = next.next_before_seq ?? null;
        if (start !== head) {
          head = cursor = start;
          older.replaceChildren(earlier);
          earlier.hidden = cursor == null;
        }
        const items = `${next.running}${JSON.stringify(next.items)}`; // Folding follows `running` too.
        if (items !== lastItems) {
          lastItems = items;
          // Unchanged rows keep their nodes, so open details and loaded
          // images survive the 1.5 s refresh of a running turn.
          const nodes = next.items.map((item, index) => {
            const json = JSON.stringify(item);
            if (!rendered[index] || rendered[index].json !== json) rendered[index] = { json, node: itemNode(item, index, opened, ctx) };
            return rendered[index].node;
          });
          rendered.length = next.items.length;
          transcript.replaceChildren(...foldSteps(next.items, nodes, next.running, opened));
        }
        const cards = [...(next.run_cards || [])].sort((a, b) => a.created_at - b.created_at);
        for (const id of [...runCards.keys()]) if (!cards.some((card) => card.id === id)) runCards.delete(id);
        const activeNodes = [], finishedNodes = [];
        for (const card of cards) {
          const run = runCards.get(card.id);
          if (run) run.update(card);
          else runCards.set(card.id, runNode(card, opened));
          const node = runCards.get(card.id).node;
          if (card.status === "succeeded" || card.status === "cancelled") {
            // A reader inspecting a live run keeps seeing it when it finishes.
            if (node.open && !finishedList.contains(node)) finishedRuns.open = true;
            finishedNodes.push(node);
          } else activeNodes.push(node);
        }
        finishedLabel.replaceChildren(el("span", null, tr(`已完成的记录（${finishedNodes.length}）`, `Completed records (${finishedNodes.length})`)), icon("chevron-right"));
        if (finishedNodes.length !== finishedList.children.length || finishedNodes.some((node, index) => finishedList.children[index] !== node)) finishedList.replaceChildren(...finishedNodes);
        const shown = [...activeNodes, ...(finishedNodes.length ? [finishedRuns] : [])];
        if (shown.length !== runs.children.length || shown.some((node, index) => runs.children[index] !== node)) runs.replaceChildren(...shown);
        approvals.replaceChildren(...(next.approvals || []).map((approval) => approvalNode(approval, session, project)));
        activity.textContent = next.running ? next.activity_status || T.running : next.error || "";
        const follow = next.read_only || next.acp_agent_id ? [] : next.follow_ups || [];
        waiting = (next.queue && next.queue.items) || [];
        const extras = JSON.stringify([follow, waiting]);
        if (extras !== lastExtras) {
          lastExtras = extras;
          chips.replaceChildren(...follow.map((text) =>
            el("button", { type: "button", class: "chip", onclick: () => {
              draft.value = text;
              draft.oninput();
            } }, text)));
          queue.replaceChildren(...waiting.map((item) =>
            el("div", { class: "queued" },
              icon("clock"),
              el("span", { class: "text" }, item.message),
              item.state === "queued" && can("native_conversation_queue_action")
                ? el("button", { type: "button", class: "icon", "aria-label": T.cancelQueued, onclick: cancelQueued(item) }, icon("close"))
                : null)));
        }
        if (failed) errors.replaceChildren();
        failed = false;
        if (first) main.replaceChildren(errors, older, transcript, runs, approvals, activity, chips, queue);
        sync();
        if (stick) toBottom();
        // Reading a finished reply here clears its "needs you" flag everywhere,
        // also when it finished while this page was out of view.
        if (!next.running && (first || wasRunning)) unseen = true;
        if (unseen && document.visibilityState === "visible") {
          unseen = false;
          seen();
        }
        wasRunning = next.running;
        first = false;
      } catch (error) {
        if (!live()) return;
        failed = true;
        errors.replaceChildren(banner(error));
        if (first) main.replaceChildren(errors);
      }
      await sleep(snapshot && (snapshot.running || (snapshot.approvals || []).length || waiting.length) ? 1500 : 4000);
    }
  }

  // ----------------------------------------------------------------- files
  // Read-only: a directory listing, text as text and pictures as thumbnails.
  function directoryNode(listing) {
    if (!listing.entries.length) return el("p", { class: "empty" }, T.none);
    const join = (name) => (listing.path === "." ? name : `${listing.path}/${name}`);
    return el("div", { class: "list" }, listing.entries.map((entry) =>
      el("a", { class: "row", href: href(entry.is_dir ? { file: null, dir: join(entry.name) } : { dir: null, file: join(entry.name) }) },
        icon(entry.is_dir ? "folder" : "doc"),
        el("span", { class: "main" }, el("span", { class: "name" }, entry.name)),
        entry.is_dir ? null : el("span", { class: "sub" }, bytes(entry.size)))));
  }

  async function fileNode(project, session, path) {
    const kind = extension(path);
    const name = baseName(path);
    if (IMAGE_EXT.has(kind)) return el("div", { class: "preview" }, el("img", { src: await loadImage(project, session, { path }), alt: name }));
    const { content } = await call("native_conversation_panel_file_read", project, { session_id: session, path });
    let view = el("p", { class: "empty" }, T.noPreview);
    if (content.text != null && kind === "svg") {
      // An <img> never runs the scripts an SVG may carry.
      view = el("img", { src: `data:image/svg+xml;charset=utf-8,${encodeURIComponent(content.text)}`, alt: name });
    } else if (content.text != null && MARKDOWN_EXT.has(kind) && content.text.length <= 300000) {
      // Larger documents stay plain text, so an odd one cannot stall the page.
      // Links and pictures in a document are relative to its own folder.
      const from = (target) => resolvePath(parentDir(path), target);
      view = markdown(content.text, {
        file: (target) => href({ dir: null, file: from(target) }),
        image: (reference, alt) => imageNode(project, session, localPath(reference) && { path: from(localPath(reference)) }, alt),
      });
    } else if (content.text != null) {
      view = el("pre", null, content.text);
    }
    return el("div", { class: "preview" },
      content.truncated ? el("p", { class: "note" }, T.truncated(bytes(content.total_bytes || 0))) : null,
      view);
  }

  async function panelView(gen, route) {
    const body = $("panel-body");
    const crumbs = $("crumbs");
    const path = route.file || route.dir;
    if (!path) return body.replaceChildren();
    const { project, session } = route;
    const parts = path === "." ? [] : path.split("/");
    crumbs.replaceChildren(
      parts.length ? el("a", { href: href({ file: null, dir: "." }) }, T.files) : el("strong", null, T.files),
      ...parts.flatMap((name, index) => [
        " / ",
        index === parts.length - 1
          ? el("strong", null, name)
          : el("a", { href: href({ file: null, dir: parts.slice(0, index + 1).join("/") }) }, name),
      ]));
    crumbs.scrollLeft = crumbs.scrollWidth;
    $("panel-close").setAttribute("aria-label", T.closeFiles);
    $("panel-close").onclick = () => go(noFiles);
    body.replaceChildren(el("p", { class: "empty" }, T.loading));
    body.scrollTop = 0;
    let shown = "";
    while (gen === gens.panel) {
      try {
        if (route.dir) {
          const listing = await call("native_conversation_panel_file_directory", project, { session_id: session, path });
          const data = JSON.stringify(listing);
          if (gen === gens.panel && data !== shown) {
            shown = data;
            body.replaceChildren(directoryNode(listing));
          }
        } else {
          const view = await fileNode(project, session, path);
          if (gen === gens.panel) body.replaceChildren(view);
        }
      } catch (error) {
        shown = "";
        if (gen === gens.panel) body.replaceChildren(banner(error));
      }
      // A folder keeps up with what the agent writes; a preview is a snapshot.
      if (!route.dir) return;
      await sleep(8000);
    }
  }

  function connectView(message) {
    $("title").textContent = "Wisp";
    $("subtitle").textContent = "";
    const input = el("input", { id: "code", autocomplete: "off", spellcheck: "false", placeholder: "xxxx-xxxx-xxxx-xxxx-xxxx-xxxx-xxxx-xxxx", "aria-label": T.code });
    const submit = () => {
      if (!parseCode(input.value)) {
        error.textContent = T.badCode;
        error.hidden = false;
        return;
      }
      // Only the digits: "/" separates the code from the route in the address.
      location.hash = input.value.replace(/[^0-9a-f]/gi, "");
    };
    input.addEventListener("keydown", (event) => event.key === "Enter" && submit());
    const error = el("p", { class: "banner", role: "alert", hidden: !message }, message || "");
    main.replaceChildren(
      el("div", { class: "connect" },
        el("h1", null, T.connectTitle),
        el("p", null, T.connectHint),
        error,
        input,
        el("button", { type: "button", class: "primary", onclick: submit }, T.connect)));
  }

  async function start() {
    $("composer").hidden = true;
    // Opened from the home screen: no link carries the code, this device does.
    if (!location.hash.slice(1) && saved.get()) history.replaceState(null, "", `#${saved.get()}`);
    const route = readRoute();
    const secret = parseCode(route.code);
    // A different code is a different computer: start over rather than mix state.
    window.addEventListener("hashchange", () => (state.key && readRoute().code === state.code ? applyRoute() : location.reload()));
    if (!secret) return connectView(route.code ? T.badCode : "");
    if (!window.isSecureContext || !crypto.subtle) return connectView(T.insecure);
    state.code = route.code;
    Object.assign(state, await derive(secret));
    const select = $("project");
    select.setAttribute("aria-label", T.projects);
    $("sessions").setAttribute("aria-label", T.conversations);
    $("crumbs").setAttribute("aria-label", T.files);
    select.onchange = () => go({ project: select.value, session: null, ...noFiles });
    const create = $("new");
    create.textContent = T.newChat;
    create.onclick = async () => {
      const project = state.route.project;
      create.disabled = true;
      $("header-new").disabled = true;
      try {
        go({ project, session: await call("native_conversation_create", project), ...noFiles });
        refreshNow();
      } catch (error) {
        $("sessions").prepend(banner(error));
      }
      create.disabled = false;
      $("header-new").disabled = false;
    };
    setupWorkspace();
    applyRoute();
    connect();
    watchInbox();
  }

  start();
})();
