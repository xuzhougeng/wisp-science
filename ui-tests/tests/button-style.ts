import { expect, type Locator } from "@playwright/test";

// Check rendered chrome, so a valid class name with a missing ancestor selector
// cannot silently fall back to the browser's default button appearance.
export async function expectPrimaryButton(button: Locator, hover = false) {
  await expect(button).toBeVisible();
  if (hover) await button.hover();
  else await button.page().mouse.move(0, 0);
  await expect.poll(() => button.evaluate((el, hovered) => {
    const probe = document.createElement("span");
    probe.style.backgroundColor = hovered ? "var(--clay-strong)" : "var(--clay)";
    probe.style.color = "var(--on-clay)";
    el.append(probe);
    const expected = getComputedStyle(probe);
    const actual = getComputedStyle(el);
    const result = {
      background: actual.backgroundColor === expected.backgroundColor,
      foreground: actual.color === expected.color,
      rounded: parseFloat(actual.borderRadius) > 0,
      padded: parseFloat(actual.paddingInlineStart) >= 8,
      emphasis: Number(actual.fontWeight) >= 600,
    };
    probe.remove();
    return result;
  }, hover)).toEqual({ background: true, foreground: true, rounded: true, padded: true, emphasis: true });
}
