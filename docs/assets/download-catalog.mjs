export const DOWNLOAD_ORIGIN = "https://wisp-science.sfl.bio";
export const RELEASES = "https://github.com/xuzhougeng/wisp-science/releases";
export const PACKAGES = [
  { id: "windows-x64-exe", os: "windows", arch: "x64", format: "exe", suffix: "x64-setup.exe" },
  { id: "windows-x64-msi", os: "windows", arch: "x64", format: "msi", suffix: "x64_en-US.msi" },
  { id: "macos-arm64-dmg", os: "macos", arch: "arm64", format: "dmg", suffix: "aarch64.dmg" },
  { id: "macos-x64-dmg", os: "macos", arch: "x64", format: "dmg", suffix: "x64.dmg" },
  { id: "linux-x64-deb", os: "linux", arch: "x64", format: "deb", suffix: "amd64.deb" },
  { id: "linux-arm64-deb", os: "linux", arch: "arm64", format: "deb", suffix: "arm64.deb" },
  { id: "linux-x64-appimage", os: "linux", arch: "x64", format: "appimage", suffix: "amd64.AppImage" },
  { id: "linux-arm64-appimage", os: "linux", arch: "arm64", format: "appimage", suffix: "aarch64.AppImage" },
  // Optional: Android Release runs only once signing is configured, and desktop
  // downloads must never wait for (or be withdrawn by) a missing APK.
  { id: "android-universal-apk", os: "android", arch: "universal", format: "apk", suffix: "android.apk", optional: true },
  { id: "macos-arm64-swiftui", os: "macos", arch: "arm64", format: "swiftui", prefix: "Wisp-Science-SwiftUI", suffix: "aarch64.dmg", optional: true },
  { id: "macos-x64-swiftui", os: "macos", arch: "x64", format: "swiftui", prefix: "Wisp-Science-SwiftUI", suffix: "x86_64.dmg", optional: true },
  { id: "windows-x64-winui-self-contained", os: "windows", arch: "x64", format: "winui-self-contained", prefix: "Wisp-Science-WinUI", suffix: "x64-setup.exe", optional: true,
    legacy: { prefix: "Wisp-Science-WinUI-Preview", suffix: "x64-self-contained-setup.exe" } },
];
// Existing immutable links and cached manifests remain readable during rollout.
// This retired package is never selected for a new manifest or offered by the UI.
const RETIRED_PACKAGES = [
  { id: "windows-x64-winui-framework", prefix: "Wisp-Science-WinUI-Preview", suffix: "x64-framework-dependent-setup.exe", optional: true },
];
const READABLE_PACKAGES = [...PACKAGES, ...RETIRED_PACKAGES];

export function validTag(tag) {
  return typeof tag === "string" && /^v\d+\.\d+\.\d+$/.test(tag);
}

export function assetName(tag, pkg) {
  return `${pkg.prefix || "wisp-science"}_${tag.slice(1)}_${pkg.suffix}`;
}

export function assetNames(tag, pkg) {
  return [assetName(tag, pkg), ...(pkg.legacy ? [assetName(tag, pkg.legacy)] : [])];
}

// A package may still be building for the latest release. Its last published
// version remains downloadable, with its own tag shown by the picker.
export function compareTags(a, b) {
  const left = a.slice(1).split(".").map(Number);
  const right = b.slice(1).split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    if (left[i] !== right[i]) return left[i] - right[i];
  }
  return 0;
}

export function downloadKey(tag, name, sha256) {
  return `releases/${tag}/${sha256}/${name}`;
}

export function validDownloadKey(key) {
  const [prefix, tag, digest, name, ...rest] = key.split("/");
  return rest.length === 0 && prefix === "releases" && validTag(tag) &&
    /^[a-f0-9]{64}$/.test(digest) && READABLE_PACKAGES.some(pkg => assetNames(tag, pkg).includes(name));
}

export function validateManifest(manifest) {
  if (manifest?.schema !== 1 || !validTag(manifest.tag) || !Array.isArray(manifest.packages) ||
      manifest.packages.some(asset => !READABLE_PACKAGES.some(pkg => pkg.id === asset?.id))) throw new Error("Invalid download manifest");
  for (const pkg of READABLE_PACKAGES) {
    const matches = manifest.packages.filter(asset => asset.id === pkg.id);
    const asset = matches[0];
    if (pkg.optional && matches.length === 0) continue;
    const tag = asset?.tag ?? manifest.tag;
    if (matches.length !== 1 || !validTag(tag) || compareTags(tag, manifest.tag) > 0 || !assetNames(tag, pkg).includes(asset.name) ||
        !/^[a-f0-9]{64}$/.test(asset.sha256) || !Number.isSafeInteger(asset.size) || asset.size <= 0 ||
        asset.key !== downloadKey(tag, asset.name, asset.sha256)) {
      throw new Error(`Invalid package: ${pkg.id}`);
    }
  }
  return manifest.packages.some(asset => RETIRED_PACKAGES.some(pkg => pkg.id === asset.id))
    ? { ...manifest, packages: manifest.packages.filter(asset => PACKAGES.some(pkg => pkg.id === asset.id)) }
    : manifest;
}

// Browser strings cannot reliably distinguish Intel and Apple Silicon Macs.
// Android reports a Linux platform, so the user agent is checked first.
export function detectOS(platform, userAgent) {
  if (/Android/i.test(userAgent)) return "android";
  if (/iPhone|iPad|iPod/i.test(userAgent)) return "";
  if (/Win/i.test(platform)) return "windows";
  if (/Mac/i.test(platform)) return "macos";
  if (/Linux/i.test(platform)) return "linux";
  return "";
}

export function contentType(name) {
  return name.endsWith(".apk") ? "application/vnd.android.package-archive" : "application/octet-stream";
}
