import { test, expect } from "@playwright/test";

const suffixes = {
  "windows-x64-exe": "x64-setup.exe", "windows-x64-msi": "x64_en-US.msi",
  "macos-arm64-dmg": "aarch64.dmg", "macos-x64-dmg": "x64.dmg",
  "linux-x64-deb": "amd64.deb", "linux-arm64-deb": "arm64.deb",
  "linux-x64-appimage": "amd64.AppImage", "linux-arm64-appimage": "aarch64.AppImage",
  "android-universal-apk": "android.apk",
  "macos-arm64-swiftui": "aarch64.dmg", "macos-x64-swiftui": "x86_64.dmg",
  "windows-x64-winui-self-contained": "x64-setup.exe",
};
const manifest = {
  schema: 1, tag: "v1.12.0",
  packages: Object.entries(suffixes).map(([id, suffix]) => {
    const prefix = id.includes("swiftui") ? "Wisp-Science-SwiftUI" : id.includes("winui") ? "Wisp-Science-WinUI" : "wisp-science";
    const name = `${prefix}_1.12.0_${suffix}`;
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

test("all standard, mobile and native packages resolve exact versioned mirror and GitHub links", async ({ page }) => {
  await page.goto("/index.html#download");
  await expect(page.locator("#release-status")).toContainText("v1.12.0");
  for (const asset of manifest.packages) {
    const [os, arch, ...parts] = asset.id.split("-");
    const format = parts.join("-");
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
    Object.defineProperty(navigator, "maxTouchPoints", { value: 0 });
  });
  await page.goto("/index.html#download");
  await expect(page.locator("#download-os")).toHaveValue("macos");
  await expect(page.locator("#download-arch")).toHaveValue("");
  await expect(page.locator("#download-primary")).not.toHaveAttribute("href");
  await page.locator("#download-arch").selectOption("arm64");
  await page.getByRole("button", { name: "EN", exact: true }).click();
  await expect(page.locator("#download-title")).toHaveText("Choose the version for your device");
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

test("native builds keep their own version, installation guidance and language selection", async ({ page }) => {
  const mixed = { ...manifest, tag: "v1.20.0", packages: manifest.packages.map(asset => ({ ...asset, tag: "v1.12.0" })) };
  await page.route("https://wisp-science.sfl.bio/downloads/latest.json", route => route.fulfill({ json: mixed }));
  await page.goto("/index.html#download");
  await page.locator("#download-os").selectOption("macos");
  await page.locator("#download-arch").selectOption("arm64");
  await page.locator("#download-format").selectOption("swiftui");
  await expect(page.locator("#release-status")).toHaveText("可下载版本 v1.12.0 · 最新发布 v1.20.0");
  await expect(page.locator("#install-help")).toContainText("macOS 13");
  await page.getByRole("button", { name: "EN", exact: true }).click();
  await expect(page.locator("#download-format")).toHaveValue("swiftui");
  await expect(page.locator("#release-status")).toHaveText("Available release v1.12.0 · Latest release v1.20.0");
  await expect(page.locator("#download-github")).toHaveAttribute("href", /\/v1\.12\.0\/Wisp-Science-SwiftUI_1\.12\.0_aarch64\.dmg$/);
  await page.locator("#download-os").selectOption("windows");
  await page.locator("#download-format").selectOption("winui-self-contained");
  await expect(page.locator("#install-help")).toContainText("Includes .NET");
  await expect(page.locator("#install-help")).toContainText("WebView edition is not required");
  await expect(page.locator("#install-help")).toContainText("automatically downloads and installs it if missing");
  await expect(page.locator("#install-help")).toContainText("internet is required only for a missing runtime");
  await expect(page.locator('#download-format option[value^="winui"]')).toHaveCount(1);
  await expect(page.locator('#download-format option[value^="winui"]')).not.toContainText("Preview");
  await expect(page.locator("#download-primary")).toHaveAttribute("href", /Wisp-Science-WinUI_1\.12\.0_x64-setup\.exe$/);
  await page.getByRole("button", { name: "中文", exact: true }).click();
  await expect(page.locator("#install-help")).toContainText("缺失时联网下载安装");
  await expect(page.locator("#install-help")).toContainText("无需另装 WebView 版");
});

test("historical WinUI Preview fallback does not promise automatic WebView2 setup", async ({ page }) => {
  const legacy = { ...manifest, packages: manifest.packages.map(asset => {
    if (!asset.id.includes("winui")) return asset;
    const name = "Wisp-Science-WinUI-Preview_1.12.0_x64-self-contained-setup.exe";
    return { ...asset, name, key: asset.key.replace(asset.name, name) };
  }) };
  await page.route("https://wisp-science.sfl.bio/downloads/latest.json", route => route.fulfill({ json: legacy }));
  await page.goto("/index.html?lang=en#download");
  await page.locator("#download-os").selectOption("windows");
  await page.locator("#download-format").selectOption("winui-self-contained");
  await expect(page.locator("#install-help")).toContainText("requires Microsoft WebView2 Runtime to be installed already");
  await expect(page.locator("#install-help")).not.toContainText("automatically downloads");
  await expect(page.locator("#download-primary")).toHaveAttribute("href", /WinUI-Preview_1\.12\.0_x64-self-contained-setup\.exe$/);
});

test("missing native packages preserve standard downloads and a clear GitHub fallback", async ({ page }) => {
  const standard = { ...manifest, packages: manifest.packages.filter(asset => !asset.id.includes("swiftui") && !asset.id.includes("winui")) };
  await page.route("https://wisp-science.sfl.bio/downloads/latest.json", route => route.fulfill({ json: standard }));
  await page.goto("/index.html?lang=en#download");
  await page.locator("#download-os").selectOption("windows");
  await expect(page.locator("#download-primary")).toHaveAttribute("href", /x64-setup\.exe$/);
  await page.locator("#download-format").selectOption("winui-self-contained");
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
  await expect(page.locator("#download-title")).toHaveText("Choose the version for your device");
});
