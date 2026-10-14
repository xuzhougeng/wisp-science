import { test, expect } from "@playwright/test";

const suffixes = {
  "windows-x64-exe": "x64-setup.exe", "windows-x64-msi": "x64_en-US.msi",
  "macos-arm64-dmg": "aarch64.dmg", "macos-x64-dmg": "x64.dmg",
  "linux-x64-deb": "amd64.deb", "linux-arm64-deb": "arm64.deb",
  "linux-x64-appimage": "amd64.AppImage", "linux-arm64-appimage": "aarch64.AppImage",
  "android-universal-apk": "android.apk",
};
const manifest = {
  schema: 1, tag: "v1.12.0",
  packages: Object.entries(suffixes).map(([id, suffix]) => {
    const name = `wisp-science_1.12.0_${suffix}`;
    return { id, name, sha256: "a".repeat(64), size: 24000000, key: `releases/v1.12.0/${"a".repeat(64)}/${name}` };
  }),
};

test.beforeEach(async ({ page }) => {
  await page.route("https://fonts.googleapis.com/**", route => route.abort());
  await page.route("https://fonts.gstatic.com/**", route => route.abort());
  await page.route("https://wisp-science.sfl.bio/downloads/latest.json", route => route.fulfill({ json: manifest }));
});

test("homepage download stays on the homepage and tutorials return to its download section", async ({ page }) => {
  await page.goto("/index.html");
  await page.getByRole("link", { name: "下载桌面安装包", exact: true }).click();
  await expect(page).toHaveURL(/index\.html(?:\?[^#]*)?#download$/);
  await expect(page.locator("#download-title")).toBeInViewport();
  await expect(page.locator("#release-status")).toContainText("v1.12.0");
  await page.goto("/tutorials/wisp-science-quick-start.html");
  await expect(page.locator(".nav-cta .btn-primary")).toHaveAttribute("href", /\.\.\/index\.html.*#download/);
});

test("all nine packages resolve exact versioned mirror and GitHub links", async ({ page }) => {
  await page.goto("/index.html#download");
  await expect(page.locator("#release-status")).toContainText("v1.12.0");
  for (const asset of manifest.packages) {
    const [os, arch, format] = asset.id.split("-");
    await page.locator("#download-os").selectOption(os);
    await page.locator("#download-arch").selectOption(arch);
    await page.locator("#download-format").selectOption(format);
    await expect(page.locator("#download-primary")).toHaveAttribute("href", `https://wisp-science.sfl.bio/downloads/${asset.key}`);
    await expect(page.locator("#download-github")).toHaveAttribute("href", `https://github.com/xuzhougeng/wisp-science/releases/download/v1.12.0/${asset.name}`);
    await expect(page.locator("#package-name")).toHaveText(asset.name);
  }
});

test("Mac never guesses architecture, and switching languages preserves selection", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { value: "MacIntel" });
    Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 Macintosh" });
  });
  await page.goto("/index.html#download");
  await expect(page.locator("#download-os")).toHaveValue("macos");
  await expect(page.locator("#download-arch")).toHaveValue("");
  await expect(page.locator("#download-primary")).not.toHaveAttribute("href");
  await page.locator("#download-arch").selectOption("arm64");
  await page.getByRole("button", { name: "EN", exact: true }).click();
  await expect(page.locator("#download-title")).toHaveText("Choose the version for your computer");
  await expect(page).toHaveTitle(/Wisp Science/);
  await expect(page.locator("#release-status")).toHaveText("Available release v1.12.0");
  await expect(page.locator("#download-arch")).toHaveValue("arm64");
  await expect(page.locator("#download-primary")).toHaveText("Download from Cloudflare");
  await expect(page.locator("#download-primary")).toHaveAttribute("href", /aarch64\.dmg$/);
  await page.locator("#download-os").selectOption("linux");
  await expect(page.locator("#download-arch")).toHaveValue("");
  await expect(page.locator("#download-primary")).not.toHaveAttribute("href");
});

for (const failure of ["network", "invalid"]) {
  test(`${failure} failure keeps GitHub fallback and disables mirror`, async ({ page }) => {
    await page.route("https://wisp-science.sfl.bio/downloads/latest.json", route => failure === "network" ? route.abort() : route.fulfill({ json: { schema: 1, tag: "v1.12.0", packages: [] } }));
    await page.goto("/index.html?lang=en#download");
    await expect(page.locator("#release-status")).toContainText("temporarily unavailable");
    await page.locator("#download-os").selectOption("windows");
    await expect(page.locator("#download-primary")).not.toHaveAttribute("href");
    await expect(page.locator("#download-github")).toHaveAttribute("href", "https://github.com/xuzhougeng/wisp-science/releases/latest");
  });
}

test("Android phones get the APK preselected within the viewport", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { value: "Linux aarch64" });
    Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (Linux; Android 14; Pixel 8)" });
  });
  await page.goto("/index.html#download");
  await expect(page.locator("#download-os")).toHaveValue("android");
  await expect(page.locator("#download-format")).toHaveValue("apk");
  await expect(page.locator("#download-primary")).toHaveAttribute("href", /_android\.apk$/);
  await expect(page.locator("#architecture-help")).toContainText("Android 8.0");
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.locator("#download").screenshot({ path: "website-test-results/download-mobile.png" });
});

test("iPhone does not guess a desktop", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { value: "iPhone" });
    Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)" });
  });
  await page.goto("/index.html#download");
  await expect(page.locator("#download-os")).toHaveValue("");
  await expect(page.locator("#download-primary")).not.toHaveAttribute("href");
});

test("a release without the APK keeps desktop downloads and points Android to all releases", async ({ page }) => {
  const desktopOnly = { ...manifest, packages: manifest.packages.filter(asset => asset.id !== "android-universal-apk") };
  await page.route("https://wisp-science.sfl.bio/downloads/latest.json", route => route.fulfill({ json: desktopOnly }));
  await page.goto("/index.html?lang=en#download");
  await expect(page.locator("#release-status")).toHaveText("Available release v1.12.0");
  await page.locator("#download-os").selectOption("windows");
  await expect(page.locator("#download-primary")).toHaveAttribute("href", /x64-setup\.exe$/);
  await page.locator("#download-os").selectOption("android");
  await expect(page.locator("#download-primary")).not.toHaveAttribute("href");
  await expect(page.locator("#package-name")).toContainText("does not include this package");
  await expect(page.locator("#download-github")).toHaveAttribute("href", "https://github.com/xuzhougeng/wisp-science/releases");
});

test("desktop download page", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1080 });
  await page.goto("/index.html#download");
  await page.locator("#download-os").selectOption("windows");
  await expect(page.locator("#download-primary")).toHaveAttribute("href", /x64-setup.exe$/);
  await page.locator("#download").screenshot({ path: "website-test-results/download-desktop.png" });
});


test("old download URL redirects to the homepage section and preserves language", async ({ page }) => {
  await page.goto("/download.html?lang=en");
  await expect(page).toHaveURL(/index\.html\?lang=en#download$/);
  await expect(page.locator("#download-title")).toHaveText("Choose the version for your computer");
});
