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
        loading: "加载中…", none: "暂无内容", back: "返回",
        send: "发送", stop: "停止", running: "运行中", needsYou: "待处理", done: "完成", failed: "失败",
        reasoning: "思考", approvalTitle: "需要你批准：", approve: "批准", reject: "拒绝",
        readOnly: "这个对话是只读的。", acp: "ACP 会话请在桌面端继续。",
        remoteAsk: "远程发起的回合中，写文件、改文件和运行命令都需要你在这里批准。",
        disconnected: "连接已断开", timeout: "请求超时，请刷新确认结果后再操作。",
        placeholder: "给 Wisp 发消息", sessionsCount: (n) => `${n} 个对话`,
      }
    : {
        connectTitle: "Connect to your Wisp", connectHint: "Enter the code shown in desktop Settings → Channels → Remote web access, or open the link copied from the desktop.",
        code: "Connection code", connect: "Connect", badCode: "Invalid code (32 hex digits).",
        insecure: "Browsers only allow encryption on HTTPS pages. Open the relay over HTTPS.",
        online: "Connected", connecting: "Connecting…", offline: "Computer offline",
        projects: "Projects", conversations: "Conversations", newChat: "New conversation", untitled: "Untitled",
        loading: "Loading…", none: "Nothing here yet", back: "Back",
        send: "Send", stop: "Stop", running: "Running", needsYou: "Needs you", done: "Done", failed: "Failed",
        reasoning: "Thinking", approvalTitle: "Approval needed: ", approve: "Approve", reject: "Reject",
        readOnly: "This conversation is read-only.", acp: "Continue ACP conversations on the desktop.",
        remoteAsk: "In remotely started turns, writing files, editing and running commands need your approval here.",
        disconnected: "Disconnected", timeout: "Request timed out. Refresh to check the result before retrying.",
        placeholder: "Message Wisp", sessionsCount: (n) => `${n} conversations`,
      };
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
    sid: null, key: null, ws: null, nonce: null, seq: 0, retry: 0, host: null,
    pending: new Map(), waiters: [], sendChain: Promise.resolve(), recvChain: Promise.resolve(),
    view: null, gen: 0, wake: null,
  };

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
      load();
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

  // ----------------------------------------------------------------- views
  function sleep(ms) {
    return new Promise((resolve) => {
      const timer = setTimeout(resolve, ms);
      state.wake = () => {
        clearTimeout(timer);
        resolve();
      };
    });
  }
  const refreshNow = () => state.wake && state.wake();

  function header(title, subtitle, back) {
    $("title").textContent = title;
    $("subtitle").textContent = subtitle || (state.host ? `${state.host.name} · Wisp ${state.host.version}` : "");
    document.title = `${title} · Wisp`;
    const button = $("back");
    button.hidden = !back;
    button.onclick = back || null;
    button.setAttribute("aria-label", T.back);
  }

  function show(view) {
    state.view = view;
    window.scrollTo(0, 0);
    load();
  }

  function load() {
    const gen = ++state.gen;
    const view = state.view;
    $("composer").hidden = view.name !== "chat";
    if (view.name === "projects") projectsView(gen);
    else if (view.name === "sessions") sessionsView(gen, view.project);
    else if (view.name === "chat") chatView(gen, view);
  }

  const banner = (error) => el("p", { class: "banner", role: "alert" }, error.message || String(error));
  const live = (gen) => gen === state.gen;

  function when(ts) {
    if (!ts) return "";
    const date = new Date(ts > 1e12 ? ts : ts * 1000);
    return date.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
  }

  function statusBadge(status) {
    if (status === "running") return el("span", { class: "badge running" }, T.running);
    if (status === "needs_you") return el("span", { class: "badge needs_you" }, T.needsYou);
    return null;
  }

  async function projectsView(gen) {
    header(T.projects, null, null);
    main.replaceChildren(el("p", { class: "empty" }, T.loading));
    while (live(gen)) {
      try {
        const projects = await call("list_projects", null);
        if (!live(gen)) return;
        main.replaceChildren(
          el("div", { class: "list" },
            projects.length ? projects.map((project) =>
              el("button", { type: "button", class: "row", onclick: () => show({ name: "sessions", project }) },
                el("span", { class: "main" },
                  el("span", { class: "name" }, project.name),
                  el("span", { class: "sub" }, T.sessionsCount(project.session_count || 0))),
                project.needs_you_count ? statusBadge("needs_you") : project.running_count ? statusBadge("running") : null)
            ) : el("p", { class: "empty" }, T.none)));
      } catch (error) {
        if (live(gen)) main.replaceChildren(banner(error));
      }
      await sleep(15000);
    }
  }

  async function sessionsView(gen, project) {
    header(project.name, null, () => show({ name: "projects" }));
    main.replaceChildren(el("p", { class: "empty" }, T.loading));
    const create = async (event) => {
      event.currentTarget.disabled = true;
      try {
        const id = await call("native_conversation_create", project.id);
        show({ name: "chat", project, session: { id, title: T.newChat } });
      } catch (error) {
        main.prepend(banner(error));
      }
    };
    while (live(gen)) {
      try {
        const sessions = await call("remote_sessions", project.id);
        if (!live(gen)) return;
        main.replaceChildren(
          el("div", { class: "list-head" },
            el("h2", null, T.conversations),
            el("button", { type: "button", class: "primary", onclick: create }, T.newChat)),
          el("div", { class: "list" },
            sessions.length ? sessions.map((session) =>
              el("button", { type: "button", class: "row", onclick: () => show({ name: "chat", project, session }) },
                el("span", { class: "main" },
                  el("span", { class: "name" }, session.title || T.untitled),
                  el("span", { class: "sub" }, when(session.activity_at || session.ts))),
                statusBadge(session.status))
            ) : el("p", { class: "empty" }, T.none)));
      } catch (error) {
        if (live(gen)) main.replaceChildren(banner(error));
      }
      await sleep(8000);
    }
  }

  function toolNode(item, key, opened) {
    const ok = item.ok === true ? T.done : item.ok === false ? T.failed : T.running;
    const body = [item.input, item.text].filter(Boolean).join("\n\n").slice(0, 20000);
    const node = el("details", { class: "tool", open: opened.has(key) },
      el("summary", null,
        el("span", null, item.tool_name || item.kind || item.role),
        el("span", { class: item.ok === false ? "state fail" : "state" }, ok)),
      body ? el("pre", null, body) : null);
    node.addEventListener("toggle", () => (node.open ? opened.add(key) : opened.delete(key)));
    return node;
  }

  function itemNode(item, index, opened) {
    const key = item.call_id || `i${index}`;
    switch (item.role) {
      case "user":
        return el("div", { class: "msg user" }, item.text);
      case "assistant":
        return item.text ? el("div", { class: "msg" }, item.text) : null;
      case "tool":
      case "acp_tool":
        return toolNode(item, key, opened);
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
        await call("native_conversation_approve", project.id, { session_id: session.id, approval_id: approval.approval_id, approved });
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

  async function chatView(gen, view) {
    const { project, session } = view;
    header(session.title || T.untitled, project.name, () => show({ name: "sessions", project }));
    main.replaceChildren(el("p", { class: "empty" }, T.loading));
    const draft = $("draft");
    const send = $("send");
    const stop = $("stop");
    const hint = $("composer-hint");
    // Keep a draft across reconnects, never across conversations.
    if (draft.dataset.session !== session.id) {
      draft.value = "";
      draft.style.height = "";
      draft.dataset.session = session.id;
    }
    draft.placeholder = T.placeholder;
    send.textContent = T.send;
    stop.textContent = T.stop;
    const opened = new Set();
    let lastItems = "";
    let snapshot = null;
    let sending = false;
    const transcript = el("div", { class: "transcript" });
    const approvals = el("div", { class: "transcript" });
    const activity = el("p", { class: "activity" });
    const errors = el("div");

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
        await call("native_conversation_send", project.id, { session_id: session.id, request_id: crypto.randomUUID(), message });
        if (live(gen)) {
          draft.value = "";
          draft.oninput();
        }
      } catch (error) {
        errors.replaceChildren(banner(error)); // The draft is kept.
      }
      sending = false;
      sync();
      refreshNow();
    };
    stop.onclick = async () => {
      stop.disabled = true;
      try {
        await call("native_conversation_stop", project.id, { session_id: session.id });
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
    while (live(gen)) {
      try {
        const next = await call("native_conversation_snapshot", project.id, { session_id: session.id });
        if (!live(gen)) return;
        snapshot = next;
        const nearBottom = window.innerHeight + window.scrollY >= document.body.scrollHeight - 120;
        const items = JSON.stringify(next.items);
        if (items !== lastItems) {
          lastItems = items;
          transcript.replaceChildren(...next.items.map((item, i) => itemNode(item, i, opened)).filter(Boolean));
        }
        approvals.replaceChildren(...(next.approvals || []).map((approval) => approvalNode(approval, session, project)));
        activity.textContent = next.running ? next.activity_status || T.running : next.error || "";
        if (first) main.replaceChildren(errors, transcript, approvals, activity);
        sync();
        if (first || nearBottom) window.scrollTo(0, document.body.scrollHeight);
        first = false;
      } catch (error) {
        if (live(gen)) errors.replaceChildren(banner(error));
        if (first && live(gen)) main.replaceChildren(errors);
      }
      await sleep(snapshot && (snapshot.running || (snapshot.approvals || []).length) ? 1500 : 4000);
    }
  }

  function connectView(message) {
    header("Wisp", "", null);
    const input = el("input", { id: "code", autocomplete: "off", spellcheck: "false", placeholder: "xxxx-xxxx-xxxx-xxxx-xxxx-xxxx-xxxx-xxxx", "aria-label": T.code });
    const submit = () => {
      if (!parseCode(input.value)) {
        error.textContent = T.badCode;
        error.hidden = false;
        return;
      }
      location.hash = input.value.trim();
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
    const secret = parseCode(decodeURIComponent(location.hash.slice(1)));
    if (!secret) return connectView(location.hash.length > 1 ? T.badCode : "");
    if (!window.isSecureContext || !crypto.subtle) return connectView(T.insecure);
    Object.assign(state, await derive(secret));
    state.view = { name: "projects" };
    load();
    connect();
  }

  window.addEventListener("hashchange", () => location.reload());
  start();
})();
