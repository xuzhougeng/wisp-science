import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(dir, "keepalive.js"), "utf8");
const root = {};
vm.runInNewContext(source, { self: root, globalThis: root, Date });
const { createLivenessTracker } = root;

function clock(start) {
  let t = start;
  return { now: () => t, advance: (ms) => { t += ms; } };
}

test("a socket that never opened is not stale", () => {
  const c = clock(1000);
  const tracker = createLivenessTracker({ staleAfterMs: 90000, now: c.now });
  c.advance(10 * 60 * 1000);
  assert.equal(tracker.isStale(), false);
});

test("inbound frames keep a healthy socket fresh", () => {
  const c = clock(1000);
  const tracker = createLivenessTracker({ staleAfterMs: 90000, now: c.now });
  tracker.reset();
  for (let i = 0; i < 20; i += 1) {
    c.advance(20000); // one keepalive interval
    tracker.touch(); // Wisp answered with a pong
    assert.equal(tracker.isStale(), false);
  }
});

test("silence longer than the stale window marks the socket dead", () => {
  const c = clock(1000);
  const tracker = createLivenessTracker({ staleAfterMs: 90000, now: c.now });
  tracker.reset();
  c.advance(90000);
  assert.equal(tracker.isStale(), false, "exactly the window is still allowed");
  c.advance(1);
  assert.equal(tracker.isStale(), true);
  tracker.reset(); // a fresh socket opened
  assert.equal(tracker.isStale(), false);
});

test("defaults allow four missed 20s pongs before giving up", () => {
  const tracker = createLivenessTracker();
  assert.equal(tracker.staleAfterMs, 90000);
});
