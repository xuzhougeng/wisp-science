import { DOWNLOAD_ORIGIN, RELEASES, PACKAGES, detectOS, validateManifest } from "./download-catalog.mjs";

const os = document.querySelector("#download-os");
const arch = document.querySelector("#download-arch");
const format = document.querySelector("#download-format");
const primary = document.querySelector("#download-primary");
let manifest;
let failed = false;
const text = (zh, en) => document.documentElement.dataset.lang === "en" ? en : zh;
const setText = (id, value) => { document.getElementById(id).textContent = value; };

function options(select, items, selected) {
  select.disabled = !items.length;
  if (!items.length) items = [["", text("请先选择操作系统", "Choose an operating system first")]];
  select.replaceChildren(...items.map(([value, label]) => new Option(label, value)));
  select.value = items.some(([value]) => value === selected) ? selected : items[0]?.[0] || "";
}

function render() {
  const architectures = os.value === "macos"
    ? [["arm64", "Apple Silicon (M1 / M2 / M3 / M4 / …)"], ["x64", "Intel (x86_64)"]]
    : os.value === "linux" ? [["x64", "Intel / AMD (x86_64)"], ["arm64", "ARM64 (aarch64)"]]
    : os.value === "windows" ? [["x64", "Intel / AMD (64-bit)"]]
    : os.value === "android" ? [["universal", text("通用 · 手机与平板", "Universal · phones and tablets")]] : [];
  options(arch, architectures.length > 1 ? [["", text("请选择芯片", "Choose a processor")], ...architectures] : architectures, arch.value);
  const formats = os.value === "windows" ? [["exe", text("常规版 · EXE · 推荐", "Standard · EXE · Recommended")], ["msi", text("常规版 · MSI", "Standard · MSI")],
      ["winui-self-contained", text("WinUI 原生版 · 含运行时", "WinUI native · Runtimes included")]]
    : os.value === "macos" ? [["dmg", text("常规版 · DMG · 推荐", "Standard · DMG · Recommended")], ["swiftui", text("SwiftUI 原生版 · DMG", "SwiftUI native · DMG")]]
    : os.value === "linux" ? [["deb", "DEB · Debian / Ubuntu"], ["appimage", "AppImage"]]
    : os.value === "android" ? [["apk", "APK"]] : [];
  options(format, formats, format.value);
  setText("architecture-help", os.value === "macos" ? text("请在“关于本机”中确认芯片；浏览器不能可靠识别 Mac 芯片。", "Check About This Mac; browsers cannot reliably identify your Mac processor.")
    : os.value === "windows" ? text("仅适用于 Intel / AMD 64 位 Windows。", "For Intel / AMD 64-bit Windows.")
    : os.value === "linux" ? text("在终端运行 uname -m 确认架构。", "Run uname -m in a terminal to confirm your architecture.")
    : os.value === "android" ? text("需要 Android 8.0 或更高版本。手机版是远程客户端，需配合已开启远程接入的 Wisp 桌面主机使用。", "Requires Android 8.0 or later. The phone app is a remote client for a Wisp desktop host with remote access enabled.")
    : text("Android 手机可选择 Android；iPhone 和 iPad 用户请选择目标电脑的系统。", "Android phones can choose Android; on an iPhone or iPad, choose the system of your target computer."));
  // Static bilingual attributes are removed for this dynamic status.
  const status = document.getElementById("release-status");
  status.removeAttribute("data-text-zh");
  status.removeAttribute("data-text-en");
  const pkg = PACKAGES.find(item => item.os === os.value && item.arch === arch.value && item.format === format.value);
  const asset = manifest?.packages.find(item => item.id === pkg?.id);
  const tag = asset?.tag ?? manifest?.tag;
  status.textContent = manifest ? (tag === manifest.tag
    ? text(`可下载版本 ${tag}`, `Available release ${tag}`)
    : text(`可下载版本 ${tag} · 最新发布 ${manifest.tag}`, `Available release ${tag} · Latest release ${manifest.tag}`))
    : failed ? text("暂时无法读取下载清单，请使用 GitHub 备用下载。", "Downloads are temporarily unavailable. Please use GitHub below.")
    : text("正在读取可下载版本…", "Loading the available release…");
  primary.removeAttribute("href");
  primary.setAttribute("aria-disabled", String(!asset));
  // Optional mobile/native installers can be absent from all published releases.
  const absent = manifest && pkg && !asset;
  document.getElementById("download-github").href = asset
    ? `${RELEASES}/download/${tag}/${encodeURIComponent(asset.name)}` : absent ? RELEASES : `${RELEASES}/latest`;
  setText("package-name", asset?.name || (absent
    ? text("此版本暂未提供该安装包，请在 GitHub 查看其他版本。", "This release does not include this package. Check GitHub for other releases.")
    : text("选择系统与芯片后显示对应安装包。", "Choose a system and processor to see your installer.")));
  setText("package-size", asset ? `${(asset.size / 1024 / 1024).toFixed(1)} MB` : "");
  document.getElementById("checksum-details").hidden = !asset;
  setText("package-checksum", asset?.sha256 || "");
  if (asset) primary.href = `${DOWNLOAD_ORIGIN}/downloads/${asset.key}`;
  setText("install-help", !pkg ? "" : pkg.format === "swiftui"
    ? text("需要 macOS 13 或更高版本。打开 DMG，将 Wisp Science SwiftUI 拖入 Applications；可与常规版并存，更新需手动下载。", "Requires macOS 13 or later. Open the DMG and drag Wisp Science SwiftUI into Applications. It installs alongside the standard app and updates by manual download.")
    : pkg.format === "winui-self-contained" ? text("无需另装 WebView 版。已包含 .NET、Windows App SDK 和应用服务。", "The WebView edition is not required. Includes .NET, Windows App SDK and app services.") + " " + (asset?.name.startsWith("Wisp-Science-WinUI-Preview_")
      ? text("此历史 Preview 安装包需要系统已有 Microsoft WebView2 Runtime。", "This historical Preview installer requires Microsoft WebView2 Runtime to be installed already.")
      : text("安装时自动检测 Microsoft WebView2 Runtime，缺失时联网下载安装；已有运行时则直接使用。", "Setup detects Microsoft WebView2 Runtime and automatically downloads and installs it if missing; internet is required only for a missing runtime."))
    : pkg.os === "macos"
    ? text("打开 DMG，将 Wisp Science 拖入 Applications。", "Open the DMG and drag Wisp Science into Applications.")
    : pkg.os === "windows" ? text("运行安装包并按提示完成安装。", "Run the installer and follow the setup steps.")
    : pkg.os === "android" ? text("下载后打开 APK；首次安装需允许浏览器“安装未知应用”。升级时直接覆盖安装，无需卸载。", "Open the downloaded APK and allow your browser to install unknown apps the first time. Install updates over the existing app; no uninstall needed.")
    : pkg.format === "deb" ? text("使用软件安装器打开 DEB，或运行 sudo apt install ./文件名.deb。", "Open the DEB with your software installer, or run sudo apt install ./filename.deb.")
    : text("为 AppImage 添加执行权限后运行：chmod +x 文件名.AppImage。", "Make the AppImage executable with chmod +x filename.AppImage, then run it."));
}

os.value = /MacIntel/.test(navigator.platform) && navigator.maxTouchPoints > 1 ? "" : detectOS(navigator.platform, navigator.userAgent);
os.addEventListener("change", () => { arch.value = ""; format.value = ""; render(); });
arch.addEventListener("change", render);
format.addEventListener("change", render);
new MutationObserver(render).observe(document.documentElement, { attributes: true, attributeFilter: ["data-lang"] });
render();
try {
  const response = await fetch(`${DOWNLOAD_ORIGIN}/downloads/latest.json`, { signal: AbortSignal.timeout(8000) });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  manifest = validateManifest(await response.json());
} catch {
  failed = true;
}
render();
