// node --test relay.test.mjs
// Drives the Durable Object through the Hibernation API surface it uses
// (acceptWebSocket, getWebSockets by tag, attachments, storage) with fakes.
import assert from "node:assert/strict";
import { test } from "node:test";
import { authorized, handle, HOST_OFFLINE_CLOSE, RemoteSid, validSid } from "./relay.mjs";

class FakeSocket {
  sent = [];
  closed = null;
  attachment = null;
  tags = [];
  send(text) {
    if (this.closed || this.broken) throw new Error("socket closed");
    this.sent.push(text);
  }
  close(code, reason) {
    this.closed = { code, reason };
  }
  accept() {}
  serializeAttachment(value) {
    this.attachment = value;
  }
  deserializeAttachment() {
    return this.attachment;
  }
}

class FakeCtx {
  sockets = [];
  store = new Map();
  storage = {
    get: async (key) => this.store.get(key),
    put: async (key, value) => void this.store.set(key, value),
  };
  acceptWebSocket(ws, tags) {
    ws.tags = tags;
    this.sockets.push(ws);
  }
  getWebSockets(tag) {
    return this.sockets.filter((ws) => !ws.closed && (!tag || ws.tags.includes(tag)));
  }
}

// undici's Response rejects status 101; the Worker runtime's accepts it.
globalThis.Response = class {
  constructor(body, init = {}) {
    this.body = body;
    this.status = init.status ?? 200;
    this.headers = new Map(Object.entries(init.headers ?? {}));
    this.webSocket = init.webSocket ?? null;
  }
};
let lastPair;
globalThis.WebSocketPair = class {
  constructor() {
    this[0] = new FakeSocket();
    this[1] = new FakeSocket();
    lastPair = this;
  }
};

const SID = "cf4778d13d24d0dd1313bca1709267bb";
const request = (path, headers = {}) =>
  new Request(`https://relay.example${path}`, { headers });
const upgrade = (path, headers = {}) => request(path, { upgrade: "websocket", ...headers });
const last = (ws) => JSON.parse(ws.sent.at(-1));

/// Connects a host and `clients` browsers to one object; returns their server ends.
async function room(ctx = new FakeCtx(), clients = 1) {
  const sid = new RemoteSid(ctx);
  const host = (await sid.fetch(upgrade(`/v1/remote/host/${SID}`))).webSocket;
  const hostServer = ctx.sockets.find((ws) => ws.tags.includes("host"));
  const browsers = [];
  for (let i = 0; i < clients; i++) {
    await sid.fetch(upgrade(`/v1/remote/client/${SID}`));
    browsers.push(ctx.sockets.at(-1));
  }
  return { sid, ctx, host, hostServer, browsers };
}

test("frames cross between the host and one browser with the Rust envelope", async () => {
  const { sid, hostServer, browsers: [browser] } = await room();
  assert.deepEqual(last(hostServer), { t: "open", c: 1 });

  sid.webSocketMessage(browser, "sealed-request");
  assert.deepEqual(last(hostServer), { t: "msg", c: 1, d: "sealed-request" });

  sid.webSocketMessage(hostServer, JSON.stringify({ t: "msg", c: 1, d: "sealed-reply" }));
  assert.deepEqual(browser.sent, ["sealed-reply"]);

  sid.webSocketClose(browser);
  assert.deepEqual(last(hostServer), { t: "close", c: 1 });
});

test("a host closing a browser, or going away, closes browsers with 4404", async () => {
  const { sid, hostServer, browsers } = await room(new FakeCtx(), 2);
  sid.webSocketMessage(hostServer, JSON.stringify({ t: "close", c: 1 }));
  assert.equal(browsers[0].closed.code, HOST_OFFLINE_CLOSE);
  assert.equal(browsers[1].closed, null);

  sid.webSocketClose(hostServer);
  assert.equal(browsers[1].closed.code, HOST_OFFLINE_CLOSE);
});

test("without a host a browser is accepted and closed 4404 at once", async () => {
  const sid = new RemoteSid(new FakeCtx());
  const response = await sid.fetch(upgrade(`/v1/remote/client/${SID}`));
  assert.equal(response.status, 101);
  assert.equal(response.webSocket, lastPair[0]);
  assert.equal(lastPair[1].closed.code, HOST_OFFLINE_CLOSE);
  assert.equal(sid.ctx.sockets.length, 0, "a refused browser is never registered");
});

test("a redial replaces the old host and its browsers reconnect, without the old close taking the new ones", async () => {
  const { sid, ctx, hostServer: old, browsers: [oldBrowser] } = await room();
  await sid.fetch(upgrade(`/v1/remote/host/${SID}`));
  const fresh = ctx.getWebSockets("host")[0];
  assert.notEqual(fresh, old);
  assert.equal(old.closed.code, 1012);
  assert.equal(oldBrowser.closed.code, HOST_OFFLINE_CLOSE);

  await sid.fetch(upgrade(`/v1/remote/client/${SID}`));
  const newBrowser = ctx.sockets.at(-1);
  assert.deepEqual(last(fresh), { t: "open", c: 2 });
  sid.webSocketClose(old);
  assert.equal(newBrowser.closed, null);
});

test("browser ids keep counting after the object was evicted and rebuilt", async () => {
  const ctx = new FakeCtx();
  await room(ctx);
  const again = new RemoteSid(ctx);
  await again.fetch(upgrade(`/v1/remote/client/${SID}`));
  assert.deepEqual(last(ctx.getWebSockets("host")[0]), { t: "open", c: 2 });
});

test("the seventeenth browser and oversized frames are refused", async () => {
  const { sid, ctx, browsers } = await room(new FakeCtx(), 16);
  await sid.fetch(upgrade(`/v1/remote/client/${SID}`));
  assert.equal(lastPair[1].closed.code, HOST_OFFLINE_CLOSE);
  assert.equal(ctx.getWebSockets("client").length, 16);

  sid.webSocketMessage(browsers[0], "x".repeat(1024 * 1024 + 1));
  assert.equal(browsers[0].closed.code, 1009);
  sid.webSocketMessage(browsers[1], new ArrayBuffer(8));
  assert.equal(browsers[1].closed, null, "binary frames are ignored, as in remote.rs");
});

test("a browser that cannot be written to is dropped instead of stalling the host", async () => {
  const { sid, hostServer, browsers: [browser] } = await room();
  browser.broken = true;
  sid.webSocketMessage(hostServer, JSON.stringify({ t: "msg", c: 1, d: "reply" }));
  assert.equal(browser.closed.code, HOST_OFFLINE_CLOSE);
  sid.webSocketMessage(hostServer, "not json");
  sid.webSocketMessage(hostServer, JSON.stringify({ t: "msg", c: 99, d: "nobody" }));
});

test("the bearer token is required, compared whole, and never matches when unset", async () => {
  const bearer = (token) => request("/", { authorization: `Bearer ${token}` });
  assert.equal(await authorized(bearer("secret"), "secret"), true);
  assert.equal(await authorized(bearer("secret2"), "secret"), false);
  assert.equal(await authorized(bearer("secre"), "secret"), false);
  assert.equal(await authorized(request("/"), "secret"), false);
  assert.equal(await authorized(bearer(""), ""), false);
  assert.equal(await authorized(request("/", { authorization: "secret" }), "secret"), false);
});

test("the Worker gates sockets before the Durable Object sees them and serves the page under the relay CSP", async () => {
  const seen = [];
  const env = {
    WISP_RELAY_TOKEN: "secret",
    REMOTE: {
      idFromName: (name) => `id:${name}`,
      get: (id) => ({ fetch: async (req) => (seen.push([id, new URL(req.url).pathname]), new Response(null, { status: 101 })) }),
    },
  };
  const assets = { html: "<html>", script: "js", icon192: "a", icon512: "b", touchIcon: "c" };
  const status = async (req, e = env) => (await handle(req, e, assets)).status;

  assert.equal(await status(request("/healthz")), 200);
  const page = await handle(request("/remote"), env, assets);
  assert.equal(page.body, "<html>");
  assert.match(page.headers.get("content-security-policy"), /connect-src 'self'/);
  assert.equal((await handle(request("/remote.js"), env, assets)).headers.get("content-type"), "text/javascript; charset=utf-8");
  assert.equal(await status(request("/elsewhere")), 404);

  assert.equal(await status(upgrade(`/v1/remote/host/${SID}`)), 401);
  assert.equal(await status(upgrade(`/v1/remote/host/${SID}`, { authorization: "Bearer nope" })), 401);
  assert.equal(await status(upgrade(`/v1/remote/host/${SID}`), { ...env, WISP_RELAY_TOKEN: "" }), 503);
  assert.equal(await status(upgrade(`/v1/remote/host/not-a-sid`, { authorization: "Bearer secret" })), 400);
  assert.equal(await status(request(`/v1/remote/client/${SID}`)), 426);
  assert.deepEqual(seen, []);

  assert.equal(await status(upgrade(`/v1/remote/host/${SID}`, { authorization: "Bearer secret" })), 101);
  assert.equal(await status(upgrade(`/v1/remote/client/${SID}`)), 101);
  assert.deepEqual(seen, [
    [`id:${SID}`, `/v1/remote/host/${SID}`],
    [`id:${SID}`, `/v1/remote/client/${SID}`],
  ]);
  assert.equal(validSid(SID.toUpperCase()), false);
});
