import { expect, test } from "@playwright/test";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { resolve } from "node:path";

const repositoryRoot = resolve(__dirname, "../..");
const readRepositoryFile = (path: string) =>
  readFileSync(resolve(repositoryRoot, path), "utf8");

const skillCount = readdirSync(resolve(repositoryRoot, "skills")).filter((name) =>
  existsSync(resolve(repositoryRoot, "skills", name, "SKILL.md")),
).length;

const loadI18n = () => {
  const source = readRepositoryFile("docs/assets/i18n.js");
  const sandbox: Record<string, unknown> = {
    document: {
      addEventListener() {},
      documentElement: {
        lang: "",
        dataset: {},
        classList: { add() {} },
      },
      querySelector() {
        return null;
      },
      querySelectorAll() {
        return [];
      },
    },
    location: { search: "", href: "https://example.test/" },
    history: { replaceState() {} },
    URL,
    URLSearchParams,
    localStorage: { getItem() { return null; }, setItem() {} },
  };
  sandbox.globalThis = sandbox;
  runInNewContext(source, sandbox);
  return sandbox.WISP_PAGES_I18N as { zh: Record<string, string>; en: Record<string, string> };
};

test("GitHub Pages homepage describes current capabilities and ships a language switch", () => {
  const index = readRepositoryFile("docs/index.html");
  const i18nJs = readRepositoryFile("docs/assets/i18n.js");

  expect(index).toContain('class="lang-switch"');
  expect(index).toContain("assets/i18n.js");
  expect(index).toContain(`${skillCount} 个内置技能`);
  for (const match of index.matchAll(/(\d+) 个内置技能/g)) {
    expect(Number(match[1]), "homepage fallback Skill count").toBe(skillCount);
  }
  expect(index).not.toContain("v1.5.0");
  expect(index).not.toContain("数据不出机器");
  expect(index).toContain("Linux");
  expect(index).toContain("Python / R");
  expect(index).not.toContain("30 个内置");
  expect(index).not.toContain("29 bundled");
  expect(index).not.toContain("暂未签名");
  expect(index).not.toContain("仅支持从源码构建");
  expect(index).not.toContain("v0.2 仍为 beta");
  expect(index).toContain("trusted-logos/pku.svg");
  expect(index).toContain("trusted-logos/cas.svg");
  expect(index).toContain("trusted-logos/zhejiang.svg");
  expect(index).toContain("trusted-logos/washu.png");
  expect(index).toContain("trusted-logos/slu.png");
  expect(index).toContain("trusted-logos/sjtu.svg");
  expect(index).toContain("trusted-logos/meduniwien.svg");
  expect(i18nJs).toContain(`${skillCount} bundled`);
  expect(i18nJs).toContain(`${skillCount} 个内置`);
  expect(i18nJs).toContain(`${skillCount} bundled SKILL`);

  // Check each claim, including metadata and FAQ copy in both languages: one
  // corrected title must not hide stale counts elsewhere in the dictionaries.
  const i18n = loadI18n();
  for (const locale of ["zh", "en"] as const) {
    for (const key of ["meta.home.desc", "meta.skills.desc", "features.skillTitle", "stack.skill", "faq.a2"]) {
      const counts = [...i18n[locale][key].matchAll(/(\d+) (?:个内置技能|bundled|domain SKILLs)/g)]
        .map(match => Number(match[1]));
      expect(counts, `${locale}: ${key}`).toEqual([skillCount]);
    }
  }
});

test("Pages i18n dictionaries cover every data-i18n key and stay in sync", () => {
  const i18n = loadI18n();
  const zhKeys = Object.keys(i18n.zh).sort();
  const enKeys = Object.keys(i18n.en).sort();
  expect(zhKeys).toEqual(enKeys);

  for (const page of ["index.html", "mcp.html", "skills.html", "tutorials.html", ...readdirSync(resolve(repositoryRoot, "docs/tutorials")).filter(name => name.endsWith(".html")).map(name => `tutorials/${name}`)]) {
    const html = readRepositoryFile(`docs/${page}`);
    expect(html).toContain('class="lang-switch"');
    expect(html).toContain("assets/i18n.js");
    const used = new Set(
      [...html.matchAll(/data-i18n(?:-html|-aria)?="([^"]+)"/g)].map((match) => match[1]),
    );
    const missing = [...used].filter((key) => !i18n.zh[key]).sort();
    expect(missing, page).toEqual([]);
  }
});

async function serveTutorialSite(page: import("@playwright/test").Page) {
  await page.route("**/*", (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== "https://tutorials.test") return route.abort();
    const file = resolve(repositoryRoot, "docs", url.pathname.replace(/^\/wisp-science\//, "") || "index.html");
    return existsSync(file) ? route.fulfill({ path: file }) : route.abort();
  });
}

test("Skills follows MCP in navigation and presents every bundled skill in both languages", async ({ page }) => {
  await serveTutorialSite(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("https://tutorials.test/wisp-science/index.html");
  const link = page.locator('.nav-links [data-i18n="nav.mcp"] + a');
  await expect(link).toHaveText("SKILLS");
  await link.click();
  await expect(page).toHaveURL(/skills\.html\?lang=zh$/);
  await expect(page.locator("h1")).toHaveText("科研技能");
  const ids = readdirSync(resolve(repositoryRoot, "skills")).filter(name => existsSync(resolve(repositoryRoot, "skills", name, "SKILL.md")));
  await expect(page.locator(".skill-card")).toHaveCount(ids.length);
  expect(await page.locator(".skill-card").evaluateAll(cards => cards.map(card => card.getAttribute("data-skill-id")).sort())).toEqual(ids.sort());
  for (const id of ids) {
    await expect(page.locator(`[data-skill-id="${id}"] > a`)).toHaveAttribute("href", `https://github.com/xuzhougeng/wisp-science/blob/main/skills/${id}/SKILL.md`);
  }
  await page.screenshot({ path: test.info().outputPath("skills-zh.png") });
  for (const lang of ["en", "zh"]) {
    await page.locator(`.lang-switch [data-lang="${lang}"]`).click();
    await expect(page.locator("h1")).toHaveText(lang === "en" ? "Research Skills" : "科研技能");
    if (lang === "en") expect(await page.locator("main").innerText()).not.toMatch(/[\p{Script=Han}]/u);
    await page.setViewportSize({ width: 390, height: 844 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.locator('.skills-groups a[href="#environment"]').click();
    await expect(page.locator("#environment h2")).toBeInViewport();
    await page.screenshot({ path: test.info().outputPath(`skills-${lang}-mobile.png`) });
  }
});

test("tutorial directory stays compact and links to independent articles", async ({ page }) => {
  await serveTutorialSite(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("https://tutorials.test/wisp-science/index.html");
  await page.locator('.nav-links a[href^="tutorials.html"]').click();
  await expect(page).toHaveTitle("教程 | Wisp Science");
  const sources = readdirSync(resolve(repositoryRoot, "docs/wechat")).filter((name) => name.endsWith(".md"));
  await expect(page.locator(".tutorial-card")).toHaveCount(sources.length);
  await expect(page.locator(".tutorial-article")).toHaveCount(0);
  await expect(page.locator(".tutorial-card h2").first()).toHaveText("快速开始");
  await page.screenshot({ path: test.info().outputPath("tutorials-desktop.png") });
  for (const width of [1440, 1280, 1120, 1101, 1100, 980, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    for (const lang of ["en", "zh"]) {
      await page.locator(`button[data-lang="${lang}"]`).click();
      await expect(page).toHaveTitle(lang === "en" ? "Tutorials | Wisp Science" : "教程 | Wisp Science");
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      const links = page.locator(".nav-links");
      if (await links.isVisible()) {
        const brand = (await page.locator(".site-nav .brand").boundingBox())!;
        const firstLink = (await links.locator("a").first().boundingBox())!;
        const lastLink = (await links.locator("a").last().boundingBox())!;
        const controls = (await page.locator(".nav-cta").boundingBox())!;
        expect(firstLink.x).toBeGreaterThanOrEqual(brand.x + brand.width);
        expect(lastLink.x + lastLink.width).toBeLessThanOrEqual(controls.x);
      }
    }
  }
  await page.screenshot({ path: test.info().outputPath("tutorials-mobile.png") });
});

for (const name of readdirSync(resolve(repositoryRoot, "docs/wechat")).filter((name) => name.endsWith(".md"))) {
  const id = name.replace(/\.md$/, "");
  test(`tutorial ${id} opens alone, keeps its images, and returns to the directory`, async ({ page }) => {
    await serveTutorialSite(page);
    await page.goto("https://tutorials.test/wisp-science/tutorials.html");
    await page.locator(`.tutorial-card#${id}`).click();
    await expect(page).toHaveURL(new RegExp(`/tutorials/${id}\\.html\\?lang=zh$`));
    const title = readRepositoryFile(`docs/wechat/${name}`).split("\n")[0].replace(/^# /, "");
    await expect(page.locator(".tutorial-article")).toHaveCount(1);
    await expect(page.locator("h1")).toHaveText(title);
    for (const img of await page.locator(".tutorial-body:visible img").all()) {
      await img.scrollIntoViewIfNeeded();
      await expect.poll(() => img.evaluate((el: HTMLImageElement) => el.complete && el.naturalWidth > 0)).toBe(true);
    }
    await page.setViewportSize({ width: 390, height: 844 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.locator('.lang-switch [data-lang="en"]').click();
    const englishTitle = readRepositoryFile(`docs/wechat/en/${name}`).split("\n")[0].replace(/^# /, "");
    await expect(page.locator("h1")).toHaveText(englishTitle);
    await expect(page.locator('.tutorial-body[lang="en"]')).toBeVisible();
    await expect(page.locator('.tutorial-body[lang="zh-CN"]')).toBeHidden();
    for (const img of await page.locator(".tutorial-body:visible img").all()) {
      await expect(img).toHaveAttribute("src", /^\.\.\/assets\/tutorials\/en\//);
      await img.scrollIntoViewIfNeeded();
      await expect.poll(() => img.evaluate((el: HTMLImageElement) => el.complete && el.naturalWidth > 0)).toBe(true);
    }
    expect(await page.locator(".tutorial-body:visible").innerText()).not.toMatch(/[\p{Script=Han}]/u);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await expect(page.locator('[data-href-en]')).toHaveAttribute("href", new RegExp(`/wechat/en/${name}$`));
    await expect(page.locator(".tutorial-breadcrumb a")).toHaveText("Back to tutorials");
    const browserTitle = await page.title();
    await page.reload();
    await expect(page.locator("h1")).toHaveText(englishTitle);
    await expect(page).toHaveTitle(browserTitle);
    await page.locator(".tutorial-reader-back a").click();
    await expect(page).toHaveURL(new RegExp(`tutorials\\.html\\?lang=en#${id}$`));
    await expect(page.locator(`.tutorial-card#${id}`)).toBeInViewport();
    await expect(page.locator(".tutorial-article")).toHaveCount(0);
  });
}

test("article links, previous and next navigation, and browser back stay within the series", async ({ page }) => {
  await serveTutorialSite(page);
  await page.goto("https://tutorials.test/wisp-science/tutorials/wisp-science-quick-start.html");
  await expect(page.locator(".tutorial-previous")).toHaveCount(0);
  await page.locator(".tutorial-next").click();
  await expect(page.locator("h1")).toContainText("模型配置");
  await page.locator(".tutorial-next").click();
  await expect(page.locator("h1")).toContainText("浏览器使用");
  await page.locator(".tutorial-previous").click();
  await expect(page.locator("h1")).toContainText("模型配置");
  await page.goto("https://tutorials.test/wisp-science/tutorials/wisp-science-acp.html");
  await expect(page.locator(".tutorial-next")).toHaveCount(0);
  await expect(page.locator("h1")).toHaveText("Wisp Science高级：ACP配置");
  await page.locator(".tutorial-previous").click();
  await expect(page.locator("h1")).toHaveText("Wisp Science进阶");
  await page.locator(".tutorial-previous").click();
  await expect(page.locator("h1")).toContainText("研究历程");
  await page.locator(".tutorial-previous").click();
  await page.locator('.tutorial-body:visible p a[href^="wisp-science-skills.html"]').click();
  await expect(page.locator("h1")).toContainText("Skills");
  await expect(page.locator(".tutorial-body:visible pre").filter({ hasText: "name: lab-paper-note" })).toContainText("# 实验室论文阅读笔记");
  await page.goBack();
  await expect(page.locator("h1")).toContainText("轨迹");
  await page.screenshot({ path: test.info().outputPath("tutorial-article.png") });
});

test("article reading and returning to the directory work without JavaScript", async ({ browser }) => {
  const context = await browser.newContext({ javaScriptEnabled: false, viewport: { width: 390, height: 844 } });
  const page = await context.newPage();
  await serveTutorialSite(page);
  await page.goto("https://tutorials.test/wisp-science/index.html");
  await page.locator('.footer-links a[href^="tutorials.html"]').click();
  await page.locator(".tutorial-card#wisp-science-skills").click();
  await expect(page.locator("h1")).toContainText("Skills");
  await expect(page.locator(".tutorial-article")).toHaveCount(1);
  await page.locator(".tutorial-breadcrumb a").click();
  await expect(page.locator("#wisp-science-skills")).toBeInViewport();
  await context.close();
});

test("English links preserve language without storage and old configuration pages are removed", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, "localStorage", { get() { throw new Error("storage disabled"); } });
  });
  await serveTutorialSite(page);
  await page.goto("https://tutorials.test/wisp-science/index.html?lang=en");
  const quickStart = page.locator('.hero-actions a[data-i18n="hero.quickStart"]');
  await expect(quickStart).toHaveText("Quick Start");
  await quickStart.click();
  await expect(page).toHaveURL(/wisp-science-quick-start\.html\?lang=en$/);
  await expect(page.locator("h1")).toHaveText("Wisp Science Basics: Quick Start");
  await page.locator(".tutorial-next").click();
  await expect(page.locator("h1")).toHaveText("Wisp Science Basics: Model Configuration");
  await page.locator(".tutorial-breadcrumb a").click();
  await expect(page.locator(".tutorial-card#wisp-science-models h2")).toHaveText("Model Configuration");
  await page.locator(".tutorial-card#wisp-science-acp").click();
  await expect(page.locator("h1")).toHaveText("Wisp Science Advanced: ACP Configuration");
  await page.screenshot({ path: test.info().outputPath("acp-english.png") });
  await page.locator('.lang-switch [data-lang="zh"]').click();
  await expect(page.locator("h1")).toHaveText("Wisp Science高级：ACP配置");
  await expect(page.locator('.tutorial-body[lang="en"]')).toBeHidden();
  await page.locator(".tutorial-breadcrumb a").click();
  await expect(page.locator(".tutorial-card#wisp-science-acp h2")).toHaveText("ACP配置");
  expect(existsSync(resolve(repositoryRoot, "docs/model-configuration.html"))).toBe(false);
  expect(existsSync(resolve(repositoryRoot, "docs/acp-agents.html"))).toBe(false);
  for (const name of ["index.html", "tutorials.html", "mcp.html"]) {
    const html = readRepositoryFile(`docs/${name}`);
    expect(html).not.toContain('data-i18n="nav.models"');
    expect(html).not.toContain('data-i18n="nav.acp"');
    expect(html).not.toMatch(/(?:model-configuration|acp-agents)\.html/);
  }
  expect(readRepositoryFile("docs/assets/i18n.js")).not.toMatch(/(?:model-configuration|acp-agents)\.html/);
});

for (const readme of ["README.md", "README_zh.md"]) {
  test(`${readme} wordmark selects a readable asset for each color scheme`, async ({ page }) => {
    await page.route("https://wordmark.test/**", (route) => route.fulfill({
      contentType: "image/svg+xml",
      body: readRepositoryFile(new URL(route.request().url()).pathname.slice(1)),
    }));
    const picture = readRepositoryFile(readme).match(/<picture>[\s\S]*?<\/picture>/)?.[0];
    expect(picture).toBeTruthy();
    await page.setContent(`<base href="https://wordmark.test/">${picture}`);
    const logo = page.getByRole("img", { name: "Wisp Science", exact: true });
    for (const mode of ["light", "dark"] as const) {
      await page.emulateMedia({ colorScheme: mode });
      await expect.poll(() => logo.evaluate((el: HTMLImageElement) => el.currentSrc))
        .toContain(`wordmark-${mode}.svg`);
      await expect.poll(() => logo.evaluate((el: HTMLImageElement) => el.complete && el.naturalWidth > 0)).toBe(true);
    }
  });
}

test("website hero wordmark and bilingual title fit desktop and mobile", async ({ page }) => {
  await page.route("**/*", (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== "https://wordmark.test") return route.abort();
    const file = resolve(repositoryRoot, "docs", url.pathname.slice(1) || "index.html");
    return existsSync(file) ? route.fulfill({ path: file }) : route.abort();
  });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("https://wordmark.test/");
  const logo = page.locator(".hero-wordmark");
  await expect(logo).toHaveAttribute("src", "assets/wordmark-light.svg");
  await expect(logo).toHaveAccessibleName("Wisp Science");
  await expect.poll(() => logo.evaluate((el: HTMLImageElement) => el.complete && el.naturalWidth > 0)).toBe(true);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(logo).toBeVisible();
    const box = (await logo.boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(width);
    expect(box.width / box.height).toBeCloseTo(520 / 344, 2);
    for (const [lang, title] of [
      ["zh", "严谨做科研， Wisp Science 在身边。"],
      ["en", "Let rigor be your guide, with Wisp Science by your side."],
    ]) {
      await page.locator(`button[data-lang="${lang}"]`).click();
      const heading = page.locator(".hero h1");
      await expect(heading).toHaveText(title, { useInnerText: true });
      await expect(heading).toBeInViewport();
      expect(await heading.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
      await expect(page.locator(".hero-actions .btn-primary")).toBeInViewport();
      await expect(page.locator('.hero-actions [data-i18n="hero.quickStart"]')).toBeInViewport();
    }
  }
});
