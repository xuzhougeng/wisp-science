import { expect, type Page } from "@playwright/test";

// Observe the actual mock response boundary, including a response deliberately
// delayed past a newer selection. Install after any test-specific invoke shim.
export async function trackInvokes(page: Page, commands: string[]) {
  await page.evaluate((commands) => {
    const w = window as any;
    const original = w.__TAURI__.core.invoke;
    const pending = w.__observedInvokes = new Set<Promise<unknown>>();
    w.__TAURI__.core.invoke = (command: string, args: any) => {
      const response = Promise.resolve().then(() => original(command, args));
      if (commands.includes(command)) {
        pending.add(response);
        void response.then(() => pending.delete(response), () => pending.delete(response));
      }
      return response;
    };
  }, commands);
}

export async function settleRendering(page: Page) {
  await page.evaluate(() => new Promise<void>(resolve => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  }));
}

export async function waitForInvokes(page: Page) {
  await page.evaluate(async () => {
    await Promise.allSettled([...(window as any).__observedInvokes]);
  });
  await settleRendering(page);
  await expect.poll(() => page.evaluate(() => (window as any).__observedInvokes.size)).toBe(0);
}

// Observe native timer firings instead of replacing the browser clock. Advancing
// a fake clock can invoke a WASM closure after Leptos has disposed its timer.
export async function trackIntervals(page: Page) {
  await page.addInitScript(() => {
    const w = window as any;
    const intervals = w.__observedIntervals = new Map<number, { delay: number; ticks: number }>();
    const original = w.setInterval.bind(w);
    w.setInterval = (callback: any, delay: number, ...args: any[]) => {
      if (typeof callback !== "function") return original(callback, delay, ...args);
      const observation = { delay: Number(delay), ticks: 0 };
      const id = original(() => { observation.ticks++; callback(...args); }, delay);
      intervals.set(id, observation);
      return id;
    };
  });
}

export async function waitForIntervalTicks(page: Page, delay: number, ticks: number) {
  const before = await page.evaluate(() => [...(window as any).__observedIntervals]
    .map(([id, value]: any) => [id, value.ticks]));
  await expect.poll(() => page.evaluate(({ before, delay, ticks }) => {
    const baseline = new Map(before as [number, number][]);
    return [...(window as any).__observedIntervals].some(([id, value]: any) =>
      value.delay === delay && value.ticks - (baseline.get(id) ?? 0) >= ticks);
  }, { before, delay, ticks })).toBe(true);
  await settleRendering(page);
}
