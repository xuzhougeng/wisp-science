import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// Exercise the production JS module with real browser Blobs, image decoding,
// DOM removal and MutationObserver delivery. No model, filesystem IPC or GPU.
test.beforeEach(async ({ page }) => {
  await page.route("**/__media-test/**", async (route) => {
    const file = new URL(route.request().url()).pathname.split("/").at(-1)!;
    await route.fulfill(file === "index.html"
      ? { contentType: "text/html", body: "<!doctype html><body></body>" }
      : { contentType: "text/javascript", body: readFileSync(resolve(__dirname, "../../ui/src", file), "utf8") });
  });
  await page.goto("/__media-test/index.html");
  await page.evaluate(async () => {
    const w = window as any;
    w.liveUrls = new Map<string, number>();
    w.revokedUrls = new Set<string>();
    w.duplicateRevokes = 0;
    const create = URL.createObjectURL.bind(URL);
    const revoke = URL.revokeObjectURL.bind(URL);
    URL.createObjectURL = (blob: Blob) => {
      const url = create(blob);
      w.liveUrls.set(url, blob.size);
      return url;
    };
    URL.revokeObjectURL = (url: string) => {
      if (w.revokedUrls.has(url)) w.duplicateRevokes++;
      w.revokedUrls.add(url);
      w.liveUrls.delete(url);
      revoke(url);
    };
    w.reads = 0;
    w.readBytes = () => new TextEncoder().encode('<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"><rect width="800" height="600" fill="green"/></svg>');
    w.__TAURI__ = { core: { invoke: async (cmd: string, args: any) => {
      if (cmd === "ui_heartbeat") { w.health = args.snapshot; return; }
      w.reads++;
      return w.readBytes();
    } } };
    const moduleUrl = "/__media-test/api.js";
    w.media = await import(moduleUrl);
    w.media.start_ui_health();
    w.owner = (id: string) => {
      const node = document.createElement("div");
      node.id = id;
      document.body.append(node);
      return node;
    };
    w.settle = () => new Promise((done) => setTimeout(done, 20));
  });
});

test("repeated thumbnail eviction releases blobs and reaches a stable bound", async ({ page }) => {
  const results = await page.evaluate(async () => {
    const w = window as any;
    const sizes: number[] = [];
    for (let cycle = 0; cycle < 3; cycle++) {
      for (let i = 0; i < 200; i++) {
        const id = `thumb-${cycle}-${i}`;
        const owner = w.owner(id);
        const url = await w.media.media_thumbnail_url(id, id);
        const img = new Image();
        img.src = url;
        await img.decode();
        if (img.naturalWidth !== 384) throw new Error("thumbnail was not downscaled");
        owner.remove();
      }
      await w.settle();
      sizes.push(w.liveUrls.size);
    }
    w.media.report_ui_health();
    await w.settle();
    return { sizes, revoked: w.revokedUrls.size, duplicates: w.duplicateRevokes, health: w.health };
  });
  expect(results.sizes).toEqual([192, 192, 192]); // 64 full + 128 thumbnail cache slots
  expect(results.revoked).toBeGreaterThan(900);
  expect(results.duplicates).toBe(0);
  expect(results.health).toMatchObject({ mediaBlobUrls: 192, mediaOwners: 0 });
});

test("concurrent reads share one URL and mounted owners survive cache eviction", async ({ page }) => {
  const result = await page.evaluate(async () => {
    const w = window as any;
    const a = w.owner("a");
    const b = w.owner("b");
    const urls = await Promise.all(Array.from({ length: 50 }, (_, i) => w.media.media_url("shared", i % 2 ? "a" : "b")));
    const reads = w.reads;
    for (let i = 0; i < 80; i++) {
      const owner = w.owner(`other-${i}`);
      await w.media.media_url(`other-${i}`, owner.id);
      owner.remove();
    }
    // Reparenting a mounted owner must not release its reference.
    b.append(a);
    await w.settle();
    const repeat = await w.media.media_url("shared", "a");
    a.remove();
    await w.settle();
    const stillReadable = (await fetch(urls[0])).ok;
    b.remove();
    await w.settle();
    return { unique: new Set(urls).size, reads, repeat: repeat === urls[0], stillReadable,
      revoked: w.revokedUrls.has(urls[0]), live: w.liveUrls.size, duplicates: w.duplicateRevokes };
  });
  expect(result).toEqual({ unique: 1, reads: 1, repeat: true, stillReadable: true, revoked: true, live: 64, duplicates: 0 });
});

test("byte budget bounds large media while protecting the visible video slot", async ({ page }) => {
  const result = await page.evaluate(async () => {
    const w = window as any;
    w.readBytes = () => new Uint8Array(8 * 1024 * 1024);
    const visible = w.owner("video");
    const videoUrl = await w.media.media_url("video", visible.id);
    for (let i = 0; i < 20; i++) {
      const owner = w.owner(`large-${i}`);
      await w.media.media_url(`large-${i}`, owner.id);
      owner.remove();
    }
    await w.settle();
    const sum = () => [...w.liveUrls.values()].reduce((a: number, b: any) => a + b, 0);
    const pinnedBytes = sum();
    const readable = (await fetch(videoUrl)).ok;
    visible.remove();
    await w.settle();
    return { pinnedBytes, readable, cachedBytes: sum(), duplicates: w.duplicateRevokes };
  });
  expect(result).toEqual({ pinnedBytes: 72 * 1024 * 1024, readable: true, cachedBytes: 64 * 1024 * 1024, duplicates: 0 });
});

test("small thumbnails share source ownership across both cache evictions", async ({ page }) => {
  const result = await page.evaluate(async () => {
    const w = window as any;
    w.readBytes = () => new TextEncoder().encode('<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"/>');
    const owner = w.owner("small");
    const full = await w.media.media_url("small", owner.id);
    const thumb = await w.media.media_thumbnail_url("small", owner.id);
    for (let i = 0; i < 150; i++) {
      const other = w.owner(`small-${i}`);
      await w.media.media_thumbnail_url(other.id, other.id);
      other.remove();
    }
    await w.settle();
    const readable = (await fetch(thumb)).ok;
    owner.remove();
    await w.settle();
    return { shared: full === thumb, readable, revoked: w.revokedUrls.has(thumb), live: w.liveUrls.size, duplicates: w.duplicateRevokes };
  });
  expect(result).toEqual({ shared: true, readable: true, revoked: true, live: 128, duplicates: 0 });
});

test("late loads cannot attach to a replacement owner and failed reads remain retryable", async ({ page }) => {
  const result = await page.evaluate(async () => {
    const w = window as any;
    const bytes = w.readBytes();
    let resolveRead: (bytes: Uint8Array) => void = () => {};
    w.readBytes = () => new Promise<Uint8Array>((resolve) => { resolveRead = resolve; });
    const removed = w.owner("pending");
    const pending = w.media.media_url("pending", removed.id);
    removed.remove();
    w.owner("pending");
    resolveRead(bytes);
    const late = await pending;
    const lateUrl = [...w.liveUrls.keys()][0];
    w.readBytes = () => { throw new Error("missing"); };
    const failures = await Promise.all(Array.from({ length: 150 }, (_, i) => w.media.media_thumbnail_url(`missing-${i}`, "pending")));
    w.readBytes = () => bytes;
    const recovered = await w.media.media_thumbnail_url("missing-0", "pending");
    document.getElementById("pending")!.remove();
    for (let i = 0; i < 70; i++) {
      const owner = w.owner(`evict-${i}`);
      await w.media.media_url(owner.id, owner.id);
      owner.remove();
    }
    await w.settle();
    w.media.report_ui_health();
    await w.settle();
    return { late, failed: failures.every((url) => url === null), recovered: recovered?.startsWith("blob:"),
      lateReleased: w.revokedUrls.has(lateUrl), owners: w.health.mediaOwners, errors: w.health.unhandledRejections };
  });
  expect(result).toEqual({ late: null, failed: true, recovered: true, lateReleased: true, owners: 0, errors: 0 });
});

test("historical image hydration handles missing files, decode failures and replacement rows", async ({ page }) => {
  const result = await page.evaluate(async () => {
    const w = window as any;
    const bytes = w.readBytes();
    const owner = w.owner("history");
    owner.innerHTML = '<img alt="old plot" data-workspace-image-path="/project/old.png">';
    let resolveRead: (bytes: Uint8Array) => void = () => {};
    w.readBytes = () => new Promise<Uint8Array>((resolve) => { resolveRead = resolve; });
    const pending = w.media.hydrate_workspace_images("history", "Unavailable");
    owner.innerHTML = '<img alt="pinned" data-resource-id="pinned" src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7">';
    resolveRead(bytes);
    await pending;
    const replacementUnchanged = owner.querySelector("img").src.startsWith("data:");

    owner.innerHTML = '<img alt="missing" data-workspace-image-path="/project/missing.png">';
    w.readBytes = () => { throw new Error("not found"); };
    await w.media.hydrate_workspace_images("history", "Unavailable");
    const missing = owner.textContent;
    owner.innerHTML = '<img alt="corrupt" data-workspace-image-path="/project/corrupt.png">';
    w.readBytes = () => new Uint8Array([1, 2, 3]);
    await w.media.hydrate_workspace_images("history", "Unavailable");
    const corrupt = owner.textContent;

    w.readBytes = () => bytes;
    owner.innerHTML = '<img alt="recovered" data-workspace-image-path="/project/missing.png">';
    await w.media.hydrate_workspace_images("history", "Unavailable");
    return { replacementUnchanged, missing, corrupt, recovered: owner.querySelector("img").naturalWidth };
  });
  expect(result).toEqual({ replacementUnchanged: true, missing: "missing — Unavailable", corrupt: "corrupt — Unavailable", recovered: 800 });
});
