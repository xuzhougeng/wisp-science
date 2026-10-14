import test from "node:test";
import assert from "node:assert/strict";
import worker, { byteRange } from "../src/worker.mjs";

const key = `releases/v1.12.0/${"a".repeat(64)}/wisp-science_1.12.0_x64.dmg`;
function fixture() {
  const calls = [];
  const metadata = { size: 10, etag: "etag", httpEtag: '"etag"' };
  return { calls, env: {
    ASSETS: { fetch: async () => new Response("website") },
    DOWNLOADS: {
      head: async key => { calls.push(["head", key]); return metadata; },
      get: async (key, options) => {
        calls.push(["get", key, options]);
        const body = "0123456789";
        return { ...metadata, body: options.range ? body.slice(options.range.offset, options.range.offset + options.range.length) : body };
      },
    },
  } };
}
const request = (path, options) => new Request(`https://wisp-science.sfl.bio${path}`, options);

test("website bypasses R2; download API accepts only known paths and read methods", async () => {
  const { env, calls } = fixture();
  assert.equal(await (await worker.fetch(request("/download.html"), env)).text(), "website");
  assert.equal((await worker.fetch(request("/downloads/private.txt"), env)).status, 404);
  const response = await worker.fetch(request(`/downloads/${key}`, { method: "POST" }), env);
  assert.equal(response.status, 405);
  assert.equal(response.headers.get("allow"), "GET, HEAD");
  assert.equal(calls.length, 0);
});

test("streams installer with original filename and immutable cache; HEAD skips body", async () => {
  const { env, calls } = fixture();
  const response = await worker.fetch(request(`/downloads/${key}`), env);
  assert.equal(await response.text(), "0123456789");
  assert.match(response.headers.get("content-disposition"), /wisp-science_1.12.0_x64.dmg/);
  assert.match(response.headers.get("cache-control"), /immutable/);
  assert.equal(response.headers.get("content-length"), "10");
  calls.length = 0;
  const head = await worker.fetch(request(`/downloads/${key}`, { method: "HEAD", headers: { Range: "bytes=1-3" } }), env);
  assert.equal(await head.text(), "");
  assert.equal(head.status, 200);
  assert.equal(calls.length, 1);
});

test("APK downloads use the Android package type so the browser can open the installer", async () => {
  const { env } = fixture();
  const response = await worker.fetch(request(`/downloads/${key.replace("x64.dmg", "android.apk")}`), env);
  assert.equal(response.headers.get("content-type"), "application/vnd.android.package-archive");
  assert.match(response.headers.get("content-disposition"), /wisp-science_1.12.0_android.apk/);
  assert.equal((await worker.fetch(request(`/downloads/${key}`), env)).headers.get("content-type"), "application/octet-stream");
});

test("manifest supports GitHub Pages CORS with short cache and conditional GET", async () => {
  const { env, calls } = fixture();
  const response = await worker.fetch(request("/downloads/latest.json"), env);
  assert.equal(response.headers.get("access-control-allow-origin"), "*");
  assert.equal(response.headers.get("cache-control"), "public, max-age=60");
  assert.equal(response.headers.get("content-disposition"), null);
  assert.equal(calls[0][1], "latest.json");
  const cached = await worker.fetch(request("/downloads/latest.json", { headers: { "If-None-Match": 'W/"etag"' } }), env);
  assert.equal(cached.status, 304);
});

test("resume serves exact byte range, rejects unsatisfiable, and honors If-Range", async () => {
  const { env } = fixture();
  for (const [range, expected, contentRange] of [["bytes=2-5", "2345", "bytes 2-5/10"], ["bytes=8-", "89", "bytes 8-9/10"], ["bytes=-3", "789", "bytes 7-9/10"]]) {
    const response = await worker.fetch(request(`/downloads/${key}`, { headers: { Range: range } }), env);
    assert.equal(response.status, 206);
    assert.equal(response.headers.get("content-range"), contentRange);
    assert.equal(await response.text(), expected);
  }
  const invalid = await worker.fetch(request(`/downloads/${key}`, { headers: { Range: "bytes=20-" } }), env);
  assert.equal(invalid.status, 416);
  assert.equal(invalid.headers.get("content-range"), "bytes */10");
  const changed = await worker.fetch(request(`/downloads/${key}`, { headers: { Range: "bytes=2-5", "If-Range": '"old"' } }), env);
  assert.equal(changed.status, 200);
  assert.equal(await changed.text(), "0123456789");
  assert.equal(byteRange("bytes=0-2,4-6", 10), null);
  assert.equal(byteRange("bytes=-0", 10), false);
});

test("missing storage and manifest replacement return uncached errors", async () => {
  const { env } = fixture();
  env.DOWNLOADS.get = async () => ({ etag: "changed" });
  assert.equal((await worker.fetch(request("/downloads/latest.json"), env)).status, 503);
  env.DOWNLOADS.head = async () => null;
  const response = await worker.fetch(request(`/downloads/${key}`), env);
  assert.equal(response.status, 404);
  assert.equal(response.headers.get("cache-control"), "no-store");
});
