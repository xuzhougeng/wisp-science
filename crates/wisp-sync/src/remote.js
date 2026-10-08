"use strict";
// Wisp remote web client (#1460). The connection code stays in the URL
// fragment and in memory; the relay only sees the derived rendezvous id and
// AES-GCM frames. Every host string is rendered with textContent.
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
      };
  document.documentElement.lang = zh ? "zh-CN" : "en";
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  const H2C = enc.encode("wisp-remote/v1/h2c");
  const C2H = enc.encode("wisp-remote/v1/c2h");
  const $ = (id) => document.getElementById(id);
  const main = $("main");

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
    folder: ["M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"],
    doc: ["M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z", "M14 2v6h6"],
  };
  function icon(name) {
    const ns = "http://www.w3.org/2000/svg";
    const svg = document.createElementNS(ns, "svg");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("aria-hidden", "true");
    for (const d of ICONS[name]) {
      const path = document.createElementNS(ns, "path");
      path.setAttribute("d", d);
      svg.append(path);
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
    route: {}, projects: [], sessions: [],
  };
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
      setTimeout(connect, Math.min(8000, 1000 * 2 ** state.retry++));
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
  async function call(command, projectId, args = {}) {
    await whenReady();
    return new Promise((resolve, reject) => {
      const ws = state.ws;
      const nonce = state.nonce;
      const id = crypto.randomUUID();
      const timer = setTimeout(() => {
        state.pending.delete(id);
        reject(new Error(T.timeout));
      }, 90000);
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
    const route = { code: safeDecode(parts[0]), project: null, session: null, dir: null, file: null };
    const keys = { p: "project", s: "session", d: "dir", f: "file" };
    for (let i = 1; i + 1 < parts.length; i += 2) {
      if (keys[parts[i]] && parts[i + 1]) route[keys[parts[i]]] = safeDecode(parts[i + 1]);
    }
    if (!route.project) route.session = null;
    if (!route.session || route.file) route.dir = null;
    if (!route.session) route.file = null;
    return route;
  }
  function href(patch) {
    const route = { ...state.route, ...patch };
    let hash = `#${state.code}`;
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
    const prev = state.route;
    const route = (state.route = readRoute());
    const filesOpen = route.dir != null || route.file != null;
    document.body.dataset.pane = filesOpen ? "panel" : route.session ? "chat" : route.project ? "side" : "home";
    if (route.project !== prev.project) sideView(++gens.side, route.project);
    else markCurrent();
    if (route.project !== prev.project || route.session !== prev.session) {
      const gen = ++gens.chat;
      if (!route.project) projectsView(gen);
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
    let title = T.projects;
    let subtitle = host;
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
    document.title = `${title} · Wisp`;
    const back = $("back");
    back.hidden = !up;
    back.onclick = up && (() => go(up));
    back.setAttribute("aria-label", T.back);
    const files = $("files");
    files.hidden = !route.session;
    files.setAttribute("aria-label", T.files);
    files.setAttribute("aria-pressed", String(filesOpen));
    files.onclick = () => go(filesOpen ? noFiles : { dir: "." });
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
      const timer = setTimeout(wake, ms);
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
    return null;
  }

  async function refreshProjects() {
    state.projects = await call("list_projects", null);
    const select = $("project");
    select.replaceChildren(...state.projects.map((project) => el("option", { value: project.id }, project.name)));
    renderHeader();
  }

  async function projectsView(gen) {
    $("composer").hidden = true;
    main.replaceChildren(el("p", { class: "empty" }, T.loading));
    let shown = "";
    while (gen === gens.chat) {
      try {
        await refreshProjects();
        if (gen !== gens.chat) return;
        // An unchanged list keeps its nodes, and with them focus and scroll.
        const data = JSON.stringify(state.projects);
        if (data !== shown) {
          shown = data;
          main.replaceChildren(
            el("div", { class: "list" },
              state.projects.length ? state.projects.map((project) =>
                el("a", { class: "row", href: href({ project: project.id, session: null, ...noFiles }) },
                  el("span", { class: "main" },
                    el("span", { class: "name" }, project.name),
                    el("span", { class: "sub" }, T.sessionsCount(project.session_count || 0))),
                  project.needs_you_count ? statusBadge("needs_you") : project.running_count ? statusBadge("running") : null)
              ) : el("p", { class: "empty" }, T.none)));
        }
      } catch (error) {
        shown = "";
        if (gen === gens.chat) main.replaceChildren(banner(error));
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
                  el("span", { class: "sub" }, when(session.activity_at || session.ts))),
                statusBadge(session.status))
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
      case "user":
        return el("div", { class: "msg user" }, item.text);
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

  async function chatView(gen, project, session) {
    const live = () => gen === gens.chat;
    main.replaceChildren(el("p", { class: "empty" }, T.loading));
    $("composer").hidden = false;
    const draft = $("draft");
    const send = $("send");
    const stop = $("stop");
    const hint = $("composer-hint");
    // Keep a draft across reconnects, never across conversations.
    if (draft.dataset.session !== session) {
      draft.value = "";
      draft.style.height = "";
      draft.dataset.session = session;
    }
    draft.placeholder = T.placeholder;
    send.textContent = T.send;
    stop.textContent = T.stop;
    const opened = new Map();
    const rendered = [];
    let lastItems = "";
    let snapshot = null;
    let sending = false;
    let failed = false;
    let wasRunning = false;
    const transcript = el("div", { class: "transcript" });
    const approvals = el("div", { class: "transcript" });
    const activity = el("p", { class: "activity" });
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

    const sync = () => {
      const blocked = !snapshot || snapshot.read_only || !!snapshot.acp_agent_id;
      const running = !!snapshot && snapshot.running;
      draft.disabled = blocked;
      send.hidden = running;
      send.disabled = blocked || sending || !draft.value.trim();
      stop.hidden = !running;
      stop.disabled = !!snapshot && snapshot.stopping;
      hint.hidden = false;
      hint.textContent = !snapshot ? "" : snapshot.read_only ? T.readOnly : snapshot.acp_agent_id ? T.acp : T.remoteAsk;
    };
    draft.oninput = () => {
      draft.style.height = "auto";
      draft.style.height = `${draft.scrollHeight}px`;
      sync();
    };
    send.onclick = async () => {
      const message = draft.value.trim();
      if (!message || sending) return;
      sending = true;
      sync();
      errors.replaceChildren();
      try {
        await call("native_conversation_send", project, { session_id: session, request_id: crypto.randomUUID(), message });
        if (live()) {
          draft.value = "";
          draft.oninput();
        }
      } catch (error) {
        errors.replaceChildren(banner(error)); // The draft is kept.
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
        errors.replaceChildren(banner(error));
      }
      refreshNow();
    };
    draft.onkeydown = (event) => {
      if (event.key === "Enter" && !event.shiftKey && !event.isComposing && matchMedia("(pointer: fine)").matches) {
        event.preventDefault();
        send.onclick();
      }
    };
    sync();

    let first = true;
    while (live()) {
      try {
        const next = await call("native_conversation_snapshot", project, { session_id: session });
        if (!live()) return;
        snapshot = next;
        const stick = first || nearBottom();
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
        approvals.replaceChildren(...(next.approvals || []).map((approval) => approvalNode(approval, session, project)));
        activity.textContent = next.running ? next.activity_status || T.running : next.error || "";
        if (failed) errors.replaceChildren();
        failed = false;
        if (first) main.replaceChildren(errors, transcript, approvals, activity);
        sync();
        if (stick) toBottom();
        // Reading a finished reply here clears its "needs you" flag everywhere.
        if (!next.running && (first || wasRunning) && document.visibilityState === "visible") seen();
        wasRunning = next.running;
        first = false;
      } catch (error) {
        if (!live()) return;
        failed = true;
        errors.replaceChildren(banner(error));
        if (first) main.replaceChildren(errors);
      }
      await sleep(snapshot && (snapshot.running || (snapshot.approvals || []).length) ? 1500 : 4000);
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
      try {
        go({ project, session: await call("native_conversation_create", project), ...noFiles });
        refreshNow();
      } catch (error) {
        $("sessions").prepend(banner(error));
      }
      create.disabled = false;
    };
    applyRoute();
    connect();
  }

  start();
})();
