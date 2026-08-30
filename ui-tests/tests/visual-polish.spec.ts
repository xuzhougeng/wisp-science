import { test, expect, type Page } from "@playwright/test";
import { tauriMock, parallelMock } from "./mock-tauri";

// Regression coverage for the visual polish pass: the composer sits flat until
// focused, right-pane tabs scroll instead of ellipsizing short labels, artifact
// cards render as compact rows, and follow-up suggestions carry no panel fill.

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

async function enterApp(page: Page, path = "/") {
  await page.goto(path);
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator(".composer-inner").first()).toBeVisible();
}

test("composer rests on a hairline border and small shadow until focused", async ({ page }) => {
  await enterApp(page);
  const inner = page.locator(".composer-inner").first();
  const input = page.locator(".composer-inner textarea").first();

  // Structural assertions against theme tokens rather than exact palette
  // values, so a token tweak does not break the test: the resting composer
  // uses the soft --border hairline (not --border-strong), and focus is the
  // only state that adds the big --shadow lift on top of --shadow-sm.
  const largestBlur = () => inner.evaluate((el) => {
    const shadow = getComputedStyle(el).boxShadow;
    if (!shadow || shadow === "none") return 0;
    // Split layers at top-level commas (not the ones inside rgb()/rgba()).
    return Math.max(...shadow.split(/,(?![^(]*\))/).map((layer) => {
      const lengths = layer.match(/-?\d+(?:\.\d+)?px/g) ?? [];
      // offset-x offset-y blur [spread]
      return parseFloat(lengths[2] ?? "0");
    }));
  });

  await input.blur();
  const border = await inner.evaluate((el) => {
    const swatch = document.createElement("span");
    document.body.appendChild(swatch);
    swatch.style.color = "var(--border)";
    const soft = getComputedStyle(swatch).color;
    swatch.style.color = "var(--border-strong)";
    const strong = getComputedStyle(swatch).color;
    swatch.remove();
    return { resting: getComputedStyle(el).borderTopColor, soft, strong };
  });
  expect(border.resting).toBe(border.soft);
  expect(border.resting).not.toBe(border.strong);

  // Poll past the .15s box-shadow transition before reading the resting blur.
  await expect.poll(largestBlur).toBeGreaterThan(0);
  const restingBlur = await largestBlur();

  await input.focus();
  await expect.poll(largestBlur).toBeGreaterThan(restingBlur);
});

test("composer shortcut hint appears only while the composer is focused or hovered", async ({ page }) => {
  await enterApp(page);
  const hint = page.locator(".composer-hint").first();
  const input = page.locator(".composer-inner textarea").first();

  await input.blur();
  await expect.poll(() => hint.evaluate((el) => getComputedStyle(el).opacity)).toBe("0");

  await input.focus();
  await expect.poll(() => hint.evaluate((el) => getComputedStyle(el).opacity)).toBe("1");

  await input.blur();
  await expect.poll(() => hint.evaluate((el) => getComputedStyle(el).opacity)).toBe("0");
  await hint.hover({ force: true });
  await expect.poll(() => hint.evaluate((el) => getComputedStyle(el).opacity)).toBe("1");
});

test("composer resize target stays functional without drawing a horizontal line", async ({ page }) => {
  await enterApp(page);
  const resizer = page.locator(".composer-resizer");
  await expect(resizer).toHaveCount(1);
  await expect(resizer).toBeVisible();
  await expect.poll(() => resizer.evaluate((el) => getComputedStyle(el, "::after").content)).toBe("none");

  const input = page.locator(".composer-inner textarea").first();
  // Resolve actionability and place the pointer on the live element before
  // reading its geometry. The eight-pixel target can move while the landing
  // transition settles, making a previously captured page coordinate stale.
  await resizer.hover();
  const box = await resizer.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.down();
  // mousedown reactively mounts the drag overlay that captures the following
  // move/up events; wait for it so a slow render cannot swallow the drag.
  await expect(page.locator(".drag-overlay")).toHaveCount(1);
  await page.mouse.move(box!.x + box!.width / 2, box!.y - 40, { steps: 4 });
  await page.mouse.up();

  await expect.poll(() => input.evaluate((el) => parseFloat(getComputedStyle(el).height))).toBeGreaterThan(220);
  await expect.poll(() => page.evaluate(() => localStorage.getItem("composerHeightCustom"))).toBe("1");
});

test("settings toggles use a softened active track and light thumb", async ({ page }) => {
  await enterApp(page);
  await page.locator(".sidebar").getByRole("button", { name: "Settings", exact: true }).click();

  const checkbox = page.getByTestId("notifications-enabled");
  const track = page.locator('[data-testid="notifications-enabled"] + .toggle-track');
  await expect(checkbox).toBeChecked();
  await expect(track).toBeVisible();

  const colors = await track.evaluate((el) => {
    const swatch = document.createElement("span");
    swatch.style.background = "var(--clay)";
    document.body.appendChild(swatch);
    const accent = getComputedStyle(swatch).backgroundColor;
    swatch.style.background = "var(--on-clay)";
    const onAccent = getComputedStyle(swatch).backgroundColor;
    swatch.remove();
    return {
      accent,
      onAccent,
      track: getComputedStyle(el).backgroundColor,
      thumb: getComputedStyle(el, "::after").backgroundColor,
    };
  });

  expect(colors.track).not.toBe(colors.accent);
  expect(colors.thumb).toBe(colors.onAccent);
});

test("right-pane tabs keep their labels and scroll instead of ellipsizing", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 700 });
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();

  const panel = page.locator(".rightpane");
  const addPanel = panel.getByRole("button", { name: "Add panel" });
  for (const name of [/^Notebook/, /^Highlights/, /^Provenance/, /^Side chat$/]) {
    await addPanel.click();
    await panel.locator(".rp-tab-add-menu").getByRole("button", { name }).click();
  }

  const tabs = panel.locator(".rp-tab");
  const tabCount = await tabs.count();
  expect(tabCount).toBeGreaterThanOrEqual(5);
  for (let i = 0; i < tabCount; i++) {
    const tab = tabs.nth(i);
    // Full label is always available as a tooltip.
    await expect(tab).toHaveAttribute("title", /.+/);
    // Short built-in labels never ellipsize; overflow moved to the scroller.
    const fits = await tab.evaluate((el) => el.scrollWidth <= el.clientWidth + 1);
    expect(await fits).toBe(true);
  }
  const scrollerOverflows = await panel.locator(".rp-tab-scroll")
    .evaluate((el) => el.scrollWidth > el.clientWidth);
  expect(scrollerOverflows).toBe(true);
});

test("generated artifact cards render as compact rows", async ({ page }) => {
  await enterApp(page);
  await page.locator(".composer-inner textarea").first().fill("ARTIFACTATTRIBUTION");
  await page.getByRole("button", { name: "Send" }).click();

  const card = page.locator('.message-artifact-card[data-artifact-name="new.png"]');
  await expect(card).toBeVisible({ timeout: 10_000 });
  const metrics = await card.evaluate((el) => {
    const thumb = el.querySelector(".message-artifact-thumb")!;
    const cardBox = el.getBoundingClientRect();
    const thumbBox = thumb.getBoundingClientRect();
    return { cardHeight: cardBox.height, thumbSize: thumbBox.width };
  });
  expect(metrics.thumbSize).toBe(40);
  // Compact row: thumb + one or two text lines, not the old ~140px tile.
  expect(metrics.cardHeight).toBeLessThan(72);
});

test("session row menu button is hidden until hover or keyboard focus", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await enterApp(page);
  await page.locator(".composer-inner textarea").first().fill("hover-reveal-me");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("echo:hover-reveal-me")).toBeVisible({ timeout: 10_000 });
  const session = page.locator(".side-item.ses", { hasText: "hover-reveal-me" });
  await expect(session).toBeVisible({ timeout: 10_000 });
  const row = session.locator("..");
  const actions = row.getByRole("button", { name: "Conversation actions" });

  const opacity = () => actions.evaluate((el) => getComputedStyle(el).opacity);
  await page.mouse.move(10, 400); // park the pointer off the sidebar rows
  await expect.poll(opacity).toBe("0");

  await row.hover();
  await expect.poll(opacity).toBe("1");

  await page.mouse.move(10, 400);
  await expect.poll(opacity).toBe("0");

  await actions.focus();
  await expect.poll(opacity).toBe("1");
});

test("session status spinner is hidden at rest and rotates while running", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await enterApp(page);
  // parallelMock holds Done for ~5s; the sidebar running indicator shows once the
  // session is in the background, so start a second conversation to push it there.
  await page.locator(".composer-inner textarea").first().fill("dot-shimmer");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("echo:dot-shimmer")).toBeVisible({ timeout: 10_000 });
  await page.locator(".sidebar").getByRole("button", { name: "New session" }).click();

  const session = page.locator(".side-item.ses", { hasText: "dot-shimmer" });
  const live = session.locator(".ses-live");
  await expect(session).toHaveClass(/running/);

  await expect(live).toBeVisible();
  await expect(live.locator("svg")).toBeVisible();
  await expect.poll(() => live.evaluate((el) => getComputedStyle(el).display)).not.toBe("none");
  const spin = await live.locator("svg").evaluate((el) => getComputedStyle(el).animationName);
  expect(spin).toContain("ses-spin");
  const glow = await live.evaluate((el) => getComputedStyle(el).filter);
  expect(glow).toContain("drop-shadow");

  await expect(session).not.toHaveClass(/running/, { timeout: 15_000 });
  await expect.poll(() => live.evaluate((el) => getComputedStyle(el).display)).toBe("none");
});

test("follow-up suggestions sit on the canvas without a panel fill", async ({ page }) => {
  await enterApp(page);
  await page.locator(".composer-inner textarea").first().fill("hello there");
  await page.getByRole("button", { name: "Send" }).click();

  const followUps = page.getByTestId("follow-up-questions");
  await expect(followUps.getByRole("button").first()).toBeVisible({ timeout: 10_000 });
  const background = await followUps.evaluate((el) => getComputedStyle(el).backgroundColor);
  expect(background).toBe("rgba(0, 0, 0, 0)");
});

test("assistant markdown rejoins bare list markers and wraps at word boundaries", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await enterApp(page);
  // `- ` alone on a line with the item text on the next line used to render an
  // orphan bullet dot above a flush-left paragraph.
  const payload = [
    "coords:",
    "",
    "**Module results** (Seurat 5):",
    "",
    "- 450 = x",
    "- ",
    "",
    "Tb1 15,248,784 y",
    "  - [QC violin](plots/qc.png)",
    "",
    "    (threshold lines included)",
    "  - [PCA scatter](plots/pca.png) 、 [PCA elbow](plots/elbow.png)",
    "",
  ].join("\n");
  await page.locator(".composer-inner textarea").first().fill(payload);
  await page.getByRole("button", { name: "Send" }).click();

  const body = page.locator(".msg.assistant .body.md").last();
  await expect(body).toContainText("Tb1 15,248,784 y", { timeout: 10_000 });
  await expect(body.locator("li", { hasText: "Tb1 15,248,784 y" })).toHaveCount(1);
  await expect(body.locator("li:empty")).toHaveCount(0);

  const listLayout = await body.locator("li", { hasText: "Tb1 15,248,784 y" }).first()
    .evaluate((el) => {
      const cs = getComputedStyle(el);
      const nested = el.querySelector("li");
      return {
        display: cs.display,
        marker: cs.listStyleType,
        nestedMarker: nested ? getComputedStyle(nested).listStyleType : "",
      };
    });
  expect(listLayout).toEqual({ display: "list-item", marker: "disc", nestedMarker: "circle" });

  const sectionLead = body.locator("p", { hasText: "Module results" });
  await expect(sectionLead.locator("strong")).toBeVisible();
  expect(await sectionLead.locator("strong").evaluate((el) => getComputedStyle(el).borderLeftWidth))
    .toBe("3px");
  await expect(sectionLead).toHaveClass(/md-lead-strong/);

  const links = body.locator('a[href="plots/pca.png"], a[href="plots/elbow.png"]');
  await expect(links).toHaveCount(2);
  const linkBoxes = await links.evaluateAll((els) => els.map((el) => {
    const rect = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    return { top: Math.round(rect.top), width: rect.width, display: cs.display };
  }));
  expect(linkBoxes[0].top).toBe(linkBoxes[1].top);
  expect(linkBoxes.every((box) => box.width < 240 && box.display === "inline")).toBe(true);

  const descriptionGap = await body.locator("li", { hasText: "QC violin" }).first()
    .evaluate((el) => {
      const link = el.querySelector("a")?.getBoundingClientRect();
      const detail = Array.from(el.querySelectorAll("p"))
        .find((p) => p.textContent?.includes("threshold"))?.getBoundingClientRect();
      return link && detail ? detail.top - link.bottom : 999;
    });
  expect(descriptionGap).toBeLessThan(12);

  // `overflow-wrap: anywhere` from `.msg .body` must not win on markdown bodies:
  // it splits inline code chips mid-token (`file.p|y`) even at normal break points.
  const wrap = await body.evaluate((el) => {
    const cs = getComputedStyle(el);
    return { overflowWrap: cs.overflowWrap, wordBreak: cs.wordBreak };
  });
  expect(wrap.overflowWrap).toBe("break-word");
  expect(wrap.wordBreak).toBe("normal");
});

test("inline mid-paragraph strong does not get the section-lead bar", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await enterApp(page);
  // `:first-child` ignores text nodes, so `「**点**」` used to pick up the same
  // green bar as a standalone `**可圈可点**` term.
  const payload = [
    "哈哈，这个接得妙，原样奉还了！",
    "",
    "「可」字开头",
    "",
    "**可圈可点**",
    "",
    "该你了，接一个「**点**」字开头的成语！",
    "",
    "规则：你接一个以**上一个成语最后一个字**开头的成语。",
  ].join("\n");
  await page.locator(".composer-inner textarea").first().fill(payload);
  await page.getByRole("button", { name: "Send" }).click();

  const body = page.locator(".msg.assistant .body.md").last();
  await expect(body).toContainText("可圈可点", { timeout: 10_000 });

  const term = body.locator("p", { hasText: /^可圈可点$/ });
  await expect(term).toHaveClass(/md-lead-strong/);
  expect(await term.locator("strong").evaluate((el) => getComputedStyle(el).borderLeftWidth))
    .toBe("3px");

  const inlinePoint = body.locator("p", { hasText: "该你了" });
  await expect(inlinePoint).not.toHaveClass(/md-lead-strong/);
  expect(await inlinePoint.locator("strong").evaluate((el) => getComputedStyle(el).borderLeftWidth))
    .toBe("0px");

  const inlineMid = body.locator("p", { hasText: "上一个成语最后一个字" });
  await expect(inlineMid).not.toHaveClass(/md-lead-strong/);
  expect(await inlineMid.locator("strong").evaluate((el) => getComputedStyle(el).borderLeftWidth))
    .toBe("0px");
});
