// Cloudflare edition of the remote web access rendezvous (docs/remote-access.md).
//
// Same wire protocol as crates/wisp-sync/src/remote.rs: a desktop dials out to
// /v1/remote/host/{sid} with the relay token, browsers open /remote#<code> and
// connect to /v1/remote/client/{sid}. One Durable Object per sid pairs the two
// and forwards opaque AES-GCM frames; it never sees the connection code.
//
// Only remote access is served here. Project sync and Wisp Lab need the
// binary relay, which stores files.

export const HOST_OFFLINE_CLOSE = 4404;
const HOST_MAX_FRAME_BYTES = 16 * 1024 * 1024;
const CLIENT_MAX_FRAME_BYTES = 1024 * 1024;
const MAX_CLIENTS_PER_HOST = 16;

// Mirrors static_asset() in remote.rs so the page runs under the same CSP.
export const PAGE_HEADERS = {
  "content-security-policy":
    "default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; connect-src 'self'; " +
    "img-src data: 'self'; manifest-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
  "referrer-policy": "no-referrer",
  "x-content-type-options": "nosniff",
  "cache-control": "no-cache",
};

export const MANIFEST =
  '{"name":"Wisp Remote","short_name":"Wisp","start_url":"remote","display":"standalone",' +
  '"background_color":"#f2f2f7","theme_color":"#f2f2f7","icons":[{"src":"remote-icon-192.png",' +
  '"sizes":"192x192","type":"image/png"},{"src":"remote-icon-512.png","sizes":"512x512","type":"image/png"}]}';

export function validSid(sid) {
  return /^[0-9a-f]{32}$/.test(sid);
}

/// Constant-time bearer check: both sides are hashed first, so lengths never leak.
export async function authorized(request, token) {
  const header = request.headers.get("authorization") ?? "";
  if (!token || !header.startsWith("Bearer ")) return false;
  const digest = async (text) =>
    new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)));
  const [expected, provided] = await Promise.all([digest(token), digest(header.slice(7))]);
  let diff = 0;
  for (let i = 0; i < expected.length; i++) diff |= expected[i] ^ provided[i];
  return diff === 0;
}

function page(body, contentType) {
  return new Response(body, { headers: { ...PAGE_HEADERS, "content-type": contentType } });
}

/// Worker entry. `assets` holds the page, script and icons bundled by worker.mjs.
export async function handle(request, env, assets) {
  const url = new URL(request.url);
  switch (url.pathname) {
    case "/healthz":
      return new Response("ok");
    case "/remote":
      return page(assets.html, "text/html; charset=utf-8");
    case "/remote.js":
      return page(assets.script, "text/javascript; charset=utf-8");
    case "/remote.webmanifest":
      return page(MANIFEST, "application/manifest+json");
    case "/remote-icon-192.png":
      return page(assets.icon192, "image/png");
    case "/remote-icon-512.png":
      return page(assets.icon512, "image/png");
    case "/remote-touch-icon.png":
      return page(assets.touchIcon, "image/png");
  }
  const socket = /^\/v1\/remote\/(host|client)\/([^/]+)$/.exec(url.pathname);
  if (!socket) return new Response("not found", { status: 404 });
  const [, role, sid] = socket;
  if (role === "host") {
    if (!env.WISP_RELAY_TOKEN) {
      return new Response("WISP_RELAY_TOKEN secret is not set on this Worker", { status: 503 });
    }
    if (!(await authorized(request, env.WISP_RELAY_TOKEN))) {
      return new Response("unauthorized", { status: 401 });
    }
  }
  if (!validSid(sid)) return new Response("bad request", { status: 400 });
  if (request.headers.get("upgrade")?.toLowerCase() !== "websocket") {
    return new Response("expected a WebSocket", { status: 426 });
  }
  // Validated here, before the Durable Object is billed for the request.
  return env.REMOTE.get(env.REMOTE.idFromName(sid)).fetch(request);
}

/// One sid: at most one host socket and up to 16 browsers, all hibernatable.
/// Per-socket state lives in the attachment, so a wake-up after hibernation
/// finds it again; the only in-memory state is nothing.
export class RemoteSid {
  constructor(ctx) {
    this.ctx = ctx;
  }

  async fetch(request) {
    const role = new URL(request.url).pathname.split("/")[3];
    const pair = new WebSocketPair();
    const [client, server] = [pair[0], pair[1]];
    if (role === "host") {
      // A redial replaces a half-open predecessor; its browsers reconnect here.
      for (const ws of this.ctx.getWebSockets("client")) close(ws, HOST_OFFLINE_CLOSE, "host offline");
      for (const ws of this.ctx.getWebSockets("host")) close(ws, 1012, "replaced");
      this.ctx.acceptWebSocket(server, ["host"]);
      server.serializeAttachment({ host: true });
    } else {
      const host = this.host();
      if (!host || this.ctx.getWebSockets("client").length >= MAX_CLIENTS_PER_HOST) {
        server.accept();
        close(server, HOST_OFFLINE_CLOSE, "host offline");
        return new Response(null, { status: 101, webSocket: client });
      }
      const id = ((await this.ctx.storage.get("next_id")) ?? 1);
      await this.ctx.storage.put("next_id", id + 1);
      this.ctx.acceptWebSocket(server, ["client", `c:${id}`]);
      server.serializeAttachment({ id });
      if (!send(host, JSON.stringify({ t: "open", c: id }))) {
        close(server, HOST_OFFLINE_CLOSE, "host offline");
      }
    }
    return new Response(null, { status: 101, webSocket: client });
  }

  host() {
    return this.ctx.getWebSockets("host")[0] ?? null;
  }

  webSocketMessage(ws, message) {
    if (typeof message !== "string") return;
    const me = ws.deserializeAttachment() ?? {};
    if (me.host) {
      if (message.length > HOST_MAX_FRAME_BYTES) return close(ws, 1009, "frame too large");
      let frame;
      try {
        frame = JSON.parse(message);
      } catch {
        return;
      }
      const target = this.ctx.getWebSockets(`c:${frame.c}`)[0];
      if (!target) return;
      if (frame.t === "msg" && typeof frame.d === "string") {
        // A browser that cannot be written to is dropped, not retried.
        if (!send(target, frame.d)) close(target, HOST_OFFLINE_CLOSE, "host offline");
      } else if (frame.t === "close") {
        close(target, HOST_OFFLINE_CLOSE, "host offline");
      }
      return;
    }
    if (message.length > CLIENT_MAX_FRAME_BYTES) return close(ws, 1009, "frame too large");
    const host = this.host();
    if (!host || !send(host, JSON.stringify({ t: "msg", c: me.id, d: message }))) {
      close(ws, HOST_OFFLINE_CLOSE, "host offline");
    }
  }

  webSocketClose(ws) {
    this.gone(ws);
  }

  webSocketError(ws) {
    this.gone(ws);
  }

  gone(ws) {
    const me = ws.deserializeAttachment() ?? {};
    if (me.host) {
      // Only the last host takes the browsers with it; a replaced one does not.
      if (this.ctx.getWebSockets("host").some((other) => other !== ws)) return;
      for (const client of this.ctx.getWebSockets("client")) close(client, HOST_OFFLINE_CLOSE, "host offline");
      return;
    }
    const host = this.host();
    if (host) send(host, JSON.stringify({ t: "close", c: me.id }));
  }
}

function send(ws, text) {
  try {
    ws.send(text);
    return true;
  } catch {
    return false;
  }
}

function close(ws, code, reason) {
  try {
    ws.close(code, reason);
  } catch {
    // Already closed.
  }
}
