# SwiftUI / WebView 实机 UI 审计

2026-09-28，版本 1.15.0，提交 `9ea5f411f7b90f4ba8c44f698f84ce0f7086e7b5`，macOS arm64 debug。以下是首轮审计的历史基线：当时两个应用都在工作树 clean 时构建，只产出审计文档与证据。

随后按用户确定的优先级实施了设置语义、消息排版、公式与产物面板。见[首批实施与验收记录](implementation/README.md)和[更新后画廊](implementation/index.html)。本机同路径应用包已被更新版本覆盖；本页及 `builds-and-evidence.json` 中的哈希仍描述原始审计截图对应的旧包，不代表当前包。

- [配对截图浏览器](index.html)：36 组，可选页面、上一组/下一组、纵向查看、打开原图和辅助功能文本。
- [下一阶段计划](../../superpowers/plans/2026-09-28-swiftui-webview-next-ui-alignment.md)：逐页差异、三批交付、小 PR 范围、验收和剩余审计项。
- [构建与截图证据](builds-and-evidence.json)：提交、包标识、可执行文件/截图哈希、尺寸和会话一致性。
- [页面索引](pages.json)：每组截图路径和差异摘要。

## 构建

以下命令从仓库根目录执行，前端步骤除外。日志保留在本机 `target/ui-alignment-20260928/`，不纳入 Git。模型目录使用离线快照；这不代表依赖首次安装也完全离线。

```bash
WISP_CATALOG_OFFLINE=1 bash scripts/build_native_macos.sh --qa
```

SwiftUI 包：`target/native-macos-qa/Wisp Science QA.app`。日志：`native-build.log`、`swift-build.log`。

WebView 前端：

```bash
cd ui
node sync-vendor.mjs
env -u NO_COLOR -u TRUNK_NO_COLOR trunk build --dist ../target/ui-alignment-20260928/web-dist
```

WebView 宿主，从仓库根目录：

```bash
TAURI_CONFIG='{"identifier":"science.wisp-science.alignment-20260928.webview","build":{"frontendDist":"../target/ui-alignment-20260928/web-dist"}}' \
WISP_CATALOG_OFFLINE=1 cargo build --locked -p wisp-tauri --features custom-protocol
```

将宿主封装到独立 QA bundle，复制 `skills`、`python`、`r`、`browser-extension`、`seed` 和图标，生成 Info.plist 并 ad-hoc 签名。完整本机脚本：`target/ui-alignment-20260928/build-webview.sh`；成品：`target/ui-alignment-20260928/Wisp WebView QA.app`。日志：`web-ui-build.log`、`web-host-build.log`。

两包均通过 `codesign --verify --deep --strict`。它们用于本机审计，不是 notarized 发布安装包。

## 数据与恢复

从已有合成 QA 数据 `/private/tmp/wisp-parity-fixes-20260925/native`，分别复制到 `/private/tmp/wisp-ui-alignment-20260928/native` 和 `webview`。SQLite 使用 backup API 复制，项目工作区分别复制并重绑定数据库位置。没有用正式项目数据做实验。

两端 `toolbar-qa` 项目都有 4 个会话、75 条消息。按 `frame_id,seq,role,content` 排序序列化后的哈希一致，见证据 JSON。富文本会话为 `parity-http-rich`，空会话为 `toolbar-qa-empty`。仅此范围证明会话内容相同，不声称两个数据库所有字段完全相等。

审计时显式设置 `WISP_BROWSER_DATABASE` 指向各自的 `wisp.sqlite`，并让 QA application-support 数据链接指向对应隔离目录，供宿主使用。结束后已经停止本次两个 QA 应用及原生 helper，并将原生 QA 链接恢复到之前的 `/private/tmp/wisp-parity-fixes-20260925/native`。原目标记录在 `/private/tmp/wisp-ui-alignment-20260928/previous-native-data-link.txt`；新的 WebView QA 链接仍指向其隔离目录。

重新复现本轮时，需要在 QA 应用关闭的状态下重新配置隔离数据链接和启动环境；直接打开现有原生 QA 包会使用恢复后的旧 QA 数据。构建包和本轮数据都已保留。

没有发送聊天、登录账号、同步、删除或执行导出。原生“研究归档”打开后尝试准备归档，返回缺少 API key，没有生成成功或确认清理；该截图只代表错误态。

## 覆盖与限制

36 组包括主要工作区和辅助窗口、19 个设置一级入口与订阅子页（20 组设置相关对照）。共 72 张配对截图，另有 1 张原生归档错误截图。设置一级页都已打开，但没有逐个完成编辑器保存验收。

截图来自 Computer Use 实机捕获，保留原始 JPEG 字节；文件扩展名已按实际编码统一为 `.jpg`，没有拼接或修饰。辅助功能文本位于同名 `.txt`；大部分是完整树，早期部分捕获是增量树，不应把文本未出现某元素当成缺失证据。

原生主窗口通常为 1056×768，WebView 为 1100×760；原生 sheet 按自身裁切。浏览器按原比例显示，支持纵向对照。这些证据用于信息结构和任务路径审计，不能用于宣布逐像素对齐。

本轮是中文、浅色。深色、英文、相同尺寸/窄窗、真实 IME、长会话性能、运行审批、成功归档、真实远程主机和科学 viewer 等仍需专项验收。部分设置的选中项目不同，磁盘占用和缓存会自动变化；这些数值差异不作为 UI 缺陷。产物另有同列表模式配对，避免把初始网格偏好差异当缺陷。

运行日志可见原生 AttributeGraph cycle 警告；尚未隔离根因或证明与具体截图差异有关，不在本次文档变更中修复。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| SwiftUI QA 构建、Trunk 前端构建、WebView 宿主构建 | 全部成功 |
| 两包严格签名检查 | 通过 |
| `python3 scripts/sync_native_design.py --check` | 通过 |
| `python3 scripts/sync_native_settings_contract.py --check` | 通过 |
| `cargo fmt --all -- --check` | 通过 |
| `WISP_CATALOG_OFFLINE=1 cargo test --workspace` | 2380 passed，0 failed，0 ignored（41 个成功汇总，含文档测试汇总） |
| `swift test --package-path apps/macos --scratch-path target/native-macos-qa/swift --disable-sandbox` | 278 passed，17 skipped，0 failed；295 个测试条目，跳过的是未启用的可选渲染检查 |
| 证据文件及 SHA-256 校验、画廊页面关联资源、脚本语法检查 | 通过 |
| 对照页实机浏览 | 页面选择、前后切换、图片显示通过 |
| `git diff --check` | 通过 |

测试日志为 `workspace-test.log`、`swift-test.log`，应用观察日志为 `native-run.log`、`webview-run.log`。本轮未修改 UI/Tauri 源码，没有另外运行全套 Playwright 或独立 wasm `cargo check`；Trunk 已编译 wasm 前端，但不替代下一阶段行为改动所需的完整检查。

## 打开对照页

可以直接打开本目录 `index.html`，它的页面数据内嵌，截图与文本均为相对路径，不请求外部服务。也可以从仓库根目录运行：

```bash
python3 -m http.server 8744 --bind 127.0.0.1 --directory docs
```

然后打开 [本机对照页](http://127.0.0.1:8744/design-qa/ui-alignment-2026-09-28/index.html)。HTML 和相邻证据文件需要一起保留；停止服务器后仍可直接打开本地 HTML。

辅助功能文本在提交前仅规范化行尾空白与末尾换行；截图字节保持原样。
