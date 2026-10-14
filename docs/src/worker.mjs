import { contentType, validDownloadKey } from "../assets/download-catalog.mjs";

function error(message, status, extra = {}) {
  return new Response(message, { status, headers: { "Cache-Control": "no-store", ...extra } });
}

// Single byte ranges cover browser/download-manager resume; ignore unsupported
// multiple ranges and malformed headers as permitted by HTTP semantics.
export function byteRange(value, size) {
  if (!value || !/^bytes=\d*-\d*$/.test(value)) return null;
  const [start, end] = value.slice(6).split("-");
  if (!start && !end) return null;
  const offset = start ? Number(start) : Math.max(0, size - Number(end));
  const last = start && end ? Math.min(Number(end), size - 1) : size - 1;
  if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(last) || offset >= size || last < offset) return false;
  return { offset, length: last - offset + 1 };
}

export default {
  async fetch(request, env) {
    const pathname = new URL(request.url).pathname;
    if (!pathname.startsWith("/downloads/")) return env.ASSETS.fetch(request);
    if (!["GET", "HEAD"].includes(request.method)) return error("Method not allowed", 405, { Allow: "GET, HEAD" });
    const manifest = pathname === "/downloads/latest.json";
    const key = manifest ? "latest.json" : pathname.slice("/downloads/".length);
    if (!manifest && !validDownloadKey(key)) return error("Not found", 404);
    try {
      const metadata = await env.DOWNLOADS.head(key);
      if (!metadata) return error("Download not available. Please use GitHub Releases.", 404);
      const headers = new Headers({
        "Cache-Control": manifest ? "public, max-age=60" : "public, max-age=31536000, immutable",
        "Content-Type": manifest ? "application/json; charset=utf-8" : contentType(key),
        "ETag": metadata.httpEtag,
        "Access-Control-Allow-Origin": "*",
        "X-Content-Type-Options": "nosniff",
        "Accept-Ranges": "bytes",
      });
      if (!manifest) headers.set("Content-Disposition", `attachment; filename="${key.split("/").at(-1)}"`);
      const matches = request.headers.get("If-None-Match")?.split(",").map(tag => tag.trim().replace(/^W\//, ""));
      if (matches?.includes(metadata.httpEtag) || matches?.includes("*")) return new Response(null, { status: 304, headers });
      const ifRange = request.headers.get("If-Range");
      const range = request.method === "GET" && (!ifRange || ifRange === metadata.httpEtag)
        ? byteRange(request.headers.get("Range"), metadata.size) : null;
      if (range === false) return error("Range not satisfiable", 416, { "Content-Range": `bytes */${metadata.size}` });
      headers.set("Content-Length", String(range ? range.length : metadata.size));
      if (range) headers.set("Content-Range", `bytes ${range.offset}-${range.offset + range.length - 1}/${metadata.size}`);
      if (request.method === "HEAD") return new Response(null, { headers });
      // The manifest may change between head/get. A conditional get avoids
      // returning a new body with stale Content-Length or ETag headers.
      const object = await env.DOWNLOADS.get(key, { onlyIf: { etagMatches: metadata.etag }, ...(range ? { range } : {}) });
      if (!object) return error("Not found", 404);
      if (!("body" in object)) return error("Download changed. Please retry.", 503, { "Retry-After": "1" });
      return new Response(object.body, { status: range ? 206 : 200, headers });
    } catch (err) {
      console.error(JSON.stringify({ event: "download_failed", key, message: err.message }));
      return error("Downloads temporarily unavailable. Please use GitHub Releases.", 503);
    }
  },
};
