import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { PACKAGES, assetName, contentType, validDownloadKey, validateManifest, detectOS } from "../assets/download-catalog.mjs";
import { selectAssets, syncRelease } from "../scripts/sync-downloads.mjs";

const release = () => ({ tagName: "v1.12.0", isDraft: false, isPrerelease: false,
  assets: PACKAGES.map(pkg => ({ id: "github-asset-id", name: assetName("v1.12.0", pkg), size: 7 })) });

test("selects nine exact installer names and excludes updater/source archives", () => {
  const data = release();
  data.assets.push({ name: "latest.json", size: 123 }, { name: "wisp-science_1.12.0_x64.app.tar.gz", size: 123 });
  assert.equal(selectAssets(data).length, 9);
  data.assets.pop();
  data.assets.shift();
  assert.throws(() => selectAssets(data), /Missing/);
  assert.throws(() => selectAssets({ ...release(), isPrerelease: true }), /stable/);
  const duplicate = release();
  duplicate.assets.push(duplicate.assets[0]);
  assert.throws(() => selectAssets(duplicate), /ambiguous/);
});

test("the optional APK never blocks desktop installers, but a published one is validated", () => {
  const desktop = release();
  desktop.assets = desktop.assets.filter(asset => !asset.name.endsWith(".apk"));
  assert.equal(selectAssets(desktop).length, 8);
  assert.equal(selectAssets(desktop).some(asset => asset.os === "android"), false);
  const missingDesktop = release();
  missingDesktop.assets = missingDesktop.assets.filter(asset => !asset.name.endsWith(".dmg"));
  assert.throws(() => selectAssets(missingDesktop), /Missing/);
  const duplicate = release();
  duplicate.assets.push(duplicate.assets.find(asset => asset.name.endsWith(".apk")));
  assert.throws(() => selectAssets(duplicate), /ambiguous/);
  const empty = release();
  empty.assets.find(asset => asset.name.endsWith(".apk")).size = 0;
  assert.throws(() => selectAssets(empty), /wisp-science_1.12.0_android.apk/);
});

test("Mac detection chooses only OS, Android is its own target, other mobile and unknown platforms are not guessed", () => {
  assert.equal(detectOS("MacIntel", "Macintosh"), "macos");
  assert.equal(detectOS("Win32", "Windows NT"), "windows");
  assert.equal(detectOS("Linux aarch64", "Mozilla/5.0 (Linux; Android 14; Pixel 8)"), "android");
  assert.equal(detectOS("Linux x86_64", "X11; Linux x86_64"), "linux");
  assert.equal(detectOS("iPhone", "iPhone"), "");
  assert.equal(detectOS("", ""), "");
});

test("download paths reject traversal, unknown files and updater manifests", () => {
  const key = `releases/v1.12.0/${"a".repeat(64)}/wisp-science_1.12.0_x64.dmg`;
  assert.ok(validDownloadKey(key));
  assert.ok(validDownloadKey(key.replace("x64.dmg", "android.apk")));
  assert.equal(validDownloadKey(key.replace("x64.dmg", "android-debug.apk")), false);
  for (const value of ["latest.json", "../secret", key + "/extra", key.replace("1.12.0_x64", "1.11.0_x64"), key.replace("x64.dmg", "x64.app.tar.gz")]) {
    assert.equal(validDownloadKey(value), false);
  }
});

async function fixture(t, overrides = {}) {
  const directory = await mkdtemp(path.join(tmpdir(), "wisp-sync-test-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const uploads = [];
  const deps = { directory, download: async (_name, file) => writeFile(file, "package"),
    upload: async (key, file) => uploads.push({ key, body: await readFile(file) }), currentTag: async () => "v1.12.0", ...overrides };
  return { uploads, deps };
}

test("publishes validated, checksummed manifest after every installer", async t => {
  const { uploads, deps } = await fixture(t);
  const manifest = await syncRelease(release(), deps);
  assert.equal(uploads.length, 10);
  assert.equal(uploads.at(-1).key, "latest.json");
  assert.deepEqual(JSON.parse(uploads.at(-1).body), validateManifest(manifest));
  assert.ok(uploads.slice(0, -1).every(({ key }) => validDownloadKey(key)));
  assert.throws(() => validateManifest({ ...manifest, packages: manifest.packages.slice(1) }));
  manifest.packages[0].key = "https://evil.example/installer.exe";
  assert.throws(() => validateManifest(manifest));
});

test("manifest without the APK stays valid; unknown or duplicate packages do not", async t => {
  const { deps } = await fixture(t);
  const manifest = await syncRelease(release(), deps);
  const apk = manifest.packages.find(asset => asset.id === "android-universal-apk");
  const desktop = manifest.packages.filter(asset => asset !== apk);
  assert.equal(validateManifest({ ...manifest, packages: desktop }).packages.length, 8);
  assert.throws(() => validateManifest({ ...manifest, packages: [...manifest.packages, apk] }));
  assert.throws(() => validateManifest({ ...manifest, packages: [...desktop, { ...apk, id: "ios-universal-ipa" }] }), /Invalid download manifest/);
  assert.throws(() => validateManifest({ ...manifest, packages: [...desktop, { ...apk, name: "wisp-science_1.12.0_x64.dmg" }] }), /android-universal-apk/);
});

test("the APK is uploaded with the Android package type, installers as octet streams", async t => {
  const types = new Map();
  const { deps } = await fixture(t, { upload: async (key, _file, type) => types.set(key.split("/").at(-1), type) });
  await syncRelease(release(), deps);
  assert.equal(types.get("wisp-science_1.12.0_android.apk"), "application/vnd.android.package-archive");
  assert.equal(types.get("wisp-science_1.12.0_x64.dmg"), "application/octet-stream");
  assert.equal(contentType("wisp-science_1.12.0_x64-setup.exe"), "application/octet-stream");
});

test("failed installer upload leaves the existing manifest untouched", async t => {
  const uploaded = [];
  const { deps } = await fixture(t, { upload: async key => {
    uploaded.push(key);
    if (uploaded.length === 3) throw new Error("R2 unavailable");
  } });
  await assert.rejects(syncRelease(release(), deps), /R2 unavailable/);
  assert.equal(uploaded.includes("latest.json"), false);
});

test("size mismatch and a newly published release never switch the manifest", async t => {
  const badSize = await fixture(t, { download: async (_name, file) => writeFile(file, "partial") });
  const data = release();
  data.assets[0].size = 100;
  await assert.rejects(syncRelease(data, badSize.deps), /Size mismatch/);
  assert.equal(badSize.uploads.length, 0);
  const changed = await fixture(t, { currentTag: async () => "v1.13.0" });
  await assert.rejects(syncRelease(release(), changed.deps), /Latest release changed/);
  assert.equal(changed.uploads.some(({ key }) => key === "latest.json"), false);
});
