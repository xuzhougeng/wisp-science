# 设置 H1–H2 验收记录

基线 main `10313c96`，分支 `codex/native-settings-h1-h2`。覆盖常规、外观、桌宠、API 模型与订阅账号；不改变登录协议。五页实现、自动化检查及双端实机配对已完成。

[截图画廊](index.html)默认展示 40 组双端实机配对，并可切换到 40 组原生／WebKit 同文档渲染；另附状态截图。构建与渲染方式见 [证据清单](evidence.json)。

## 实现

- 常规偏好行根据可用宽度上下排列；目录选择只接受目录。帮助文字区分草稿保存、立即保存与生效范围。
- 外观预览使用草稿主题、配色、字号及字体；取消恢复持久化偏好。保留原生显式保存策略。
- 桌宠显示已保存资源的名称、描述、ID、版本、目录和第一帧；加载、未配置、已关闭、读取失败分别呈现。关闭但保留目录时不会误报未配置。增加取消和全局保存范围说明。
- 模型行显示完整名称、精确模型 ID、协议、图片能力、默认标识与编辑操作；长名称及窄窗英文可换行。订阅模型编辑只展示适用字段，不显示 API 密钥和测试操作；保存参数保持原有契约。
- 订阅状态沿用共享类型，区分加载、未登录、已保存及读取失败。显示账号 ID 和添加模型禁用原因，拒绝过期返回，重新载入刷新账号状态。没有更改认证流程。
- 补齐本阶段范围内英文文案，包括保存成功提示与本地环境说明；订阅卡补充与 WebView 一致的套餐适用范围，操作按钮靠右。
- 修复 WebView 已保存 `send_with_modifier=true` 时下拉框仍显示 Enter 的初始选择问题；增加重新打开设置后的选择值和实际发送行为回归。不改变快捷键存储或发送规则。
- 不扩展到 H3–H5。

## 实机结果

原生 QA 使用隔离数据库和离线模型，不连接真实模型 API、不执行登录。

| 页面/交互 | 观察结果 |
| --- | --- |
| 常规 | 切换恢复上次会话后取消恢复原值；保存后 SQLite 中 `resume_last_session=true`。双端语言改为英文、保存、重开设置仍为英文；最终恢复中文。发送快捷键均显示 Cmd/⌘Enter |
| 桌宠 | 启用时显示合成测试图、名称、版本和目录；关闭并保存后显示“桌宠已关闭”，目录保留 |
| 目录选择器 | 原生目录弹窗可取消，返回桌宠设置；首次连发 Escape 早于弹窗就绪，不能据此声称首次按键已关闭。弹窗出现后的 Escape 关闭且父设置保留 |
| 外观 | 深色及 Georgia 字体草稿反映在预览；取消恢复浅色和默认字体。保存深色后实机及 SQLite 均确认生效，重新载入仍为深色，再恢复浅色并保存；保存失败/草稿保留另有单元测试 |
| API 模型 | 两个离线模型的完整 ID、协议和能力可见；切换默认立即更新，随后恢复原默认；编辑页出现后立即 Escape，仅关闭编辑页。输入不存在的名称显示无匹配状态，清除后恢复两条模型 |
| 订阅 | ChatGPT/SuperGrok 未登录、添加按钮禁用且有原因；两端打开登录页后立即 Escape，仅关闭登录页，设置父页保留。不启动授权 |

最终实机矩阵已完成，命名为 `desktop-{native|webview}-{page}-{zh|en}-{light|dark}-{wide|narrow}.jpg`，共 80 张、40 组配对。实际窗口截图宽度为 1100/680、高度 760；宽窄布局保留各平台导航方式。模型截图显式选择 API 分类并确认完整测试模型 ID，避免保留订阅页选中状态而误标。

实机两套数据库含相同合成模型和设置偏好，工作区/桌宠目录因隔离分为 native/webview 路径；这里比较全局设置，不混用项目级值。实机账号均未登录；已保存账号、失败及空态使用下述 mock。常规最终语言行和右侧取消/保存按钮已重新拍摄。早期约 732 px 截图只用于独立交互证据，不作为最终配对基线。

离线配对使用真实 `NativeSettingsView` 的 NSHostingView 和真实 Leptos 页面在 Playwright WebKit 中渲染，共 5 页 × 中英 × 明暗 × 1100/680 = 40 组；另有 9 张原生空态/失败/关闭场景。原生 49 张归档于 `renders/`，WebKit 40 张归档于 `web-renders/`。两端使用相同可见设置、模型 ID/名称和合成账号状态；这些渲染使用 mock，不证明真实认证。已审阅中英文窄窗、明暗、桌宠预览/关闭、模型与账号读取失败等代表场景；生成成功不等于逐张人工验收。长页首屏不代表下方内容，外观预览另附实机滚动图。

WebView 早期有辅助功能树与截图不一致，曾临时禁用动画诊断。Mac 解锁后使用原始动画重新启动，五页及所有实机组合均正常显示；最终 80 张实机图没有自定义动画覆盖。此前观察异常的底层原因未定位，不声称修复了动画代码。WebKit 八组测试另在原始动画下确认 opacity=1 和无横向内容溢出，仅截图时禁用动画。

## 验证

日志目录：`target/settings-h1-h2-20260928/`。

- 最终源码 `WISP_NATIVE_SNAPSHOT_DIR=... swift test --package-path apps/macos`：328 项通过、0 失败，启用设置渲染并刷新归档（`swift-completion.log`）。最后两处漏译的定向 17 项亦通过（`native-localization-tests.log`）。
- 19 项定向检查包含 5 项新增行为测试、12 项原有设置模型测试及 2 项渲染测试，覆盖账号错误/重试/迟到响应、桌宠第一帧、未知字段保留、跨页草稿/取消、保存失败。
- `cargo test --workspace` 最终完整运行退出 0：2,383 项通过、0 失败、0 忽略，包含 workspace 文档测试（`rust-final-workspace.log`）。早期 E0463 依赖元数据错误未在最终运行复现。
- wasm 检查通过（`wasm-final-check.log`）；fmt、设计资源同步和 `git diff --check` 最终复查通过。
- 原生完整构建通过（`native-complete-build.log`）；最后文案调整后 Swift 可执行文件与资源重新构建（`native-localization-build.log`）、打包和签名验证通过，沿用未修改的 Rust 宿主。
- WebView 当前前端及 QA 包重建通过（`webview-complete-build.log`）。
- 固定构建的基线 Playwright 全套：881 通过、2 跳过、0 失败（`playwright-full-static.log`）。包含快捷键修复及新增 8 项矩阵的最终全套：889 通过、2 跳过、0 失败，命令退出 0（`playwright-latest-full.log`）。
- WebKit 既有设置/订阅测试 17 项通过（`webkit-test.log`）；最终配对矩阵 8 项通过（`webkit-final-matrix.log`）；快捷键保存、重新打开和实际发送回归 1 项通过（`shortcut-fixed.log`）。
- 早期动态开发服务器全套出现首页启动超时，固定构建全套未复现；没有证据将其归因于本次产品改动，也未宣称动态服务器根因已修复。

## 配对截图复现

在仓库根目录运行原生渲染；生成的 PNG 使用当前屏幕 backing scale，比较布局时按 1100/680 pt 归一化，不直接拿 PNG 像素宽度与 CSS px 比较。

```bash
WISP_NATIVE_SNAPSHOT_DIR="$PWD/target/settings-h1-h2-renders" swift test --package-path apps/macos --filter NativeSettingsAlignmentRenderTests
```

Web 测试以普通 Playwright 配置运行时使用 Chromium，也纳入全套 CI。要复现本画廊的 WebKit 渲染，在仓库根目录准备当前前端的静态构建，并在独立终端启动服务器：

```bash
cd ui
trunk build --dist ../target/settings-h1-h2-web-dist
cd ..
python3 -m http.server 1429 --bind 127.0.0.1 --directory target/settings-h1-h2-web-dist
```

另一个终端在仓库根目录创建临时 WebKit 配置并运行八组场景：

```bash
mkdir -p target/settings-h1-h2-qa
cat > target/settings-h1-h2-qa/webkit.config.ts <<'EOF'
import config from '../../ui-tests/playwright.config';
export default {
  ...config,
  testDir: '../../ui-tests/tests',
  outputDir: './results',
  use: { ...config.use, browserName: 'webkit' },
  webServer: undefined,
};
EOF
cd ui-tests
npm ci
npx playwright install webkit
UI_TEST_PORT=1429 WISP_WEB_SETTINGS_SNAPSHOTS="$PWD/../target/settings-h1-h2-web-renders" npx playwright test --config=../target/settings-h1-h2-qa/webkit.config.ts settings-alignment
```

静态服务器仅服务本地测试构建；完成后停止该终端进程。fixture 来自 `NativeSettingsAlignmentRenderTests.swift` 与 `settings-alignment.spec.ts`，不依赖真实账号或模型服务。

## 完成审计与后续范围

| H1–H2 要求 | 完成证据 |
| --- | --- |
| 偏好行、标签、帮助与响应式布局 | 40 组实机 + 40 组同文档渲染，长名称和英文窄窗复核 |
| 草稿/即时保存范围、取消、未知字段保留 | 页面说明、原生实机保存/取消、SettingsModel 与 Alignment 行为测试 |
| 主题/字体草稿预览、保存和重新载入 | 草稿及已保存实机图、Swift 保存失败测试、深浅色实机矩阵 |
| 桌宠资源预览、目录选择、启用及状态 | 合成资源实机图、关闭保留目录、目录弹窗取消、第一帧测试、空态/失败渲染 |
| API 模型 ID/能力/默认/编辑及列表状态 | API 分类实机长名称、默认切换恢复、无匹配截图、编辑 Escape、模型空态/错误渲染 |
| 订阅状态、适用范围、禁用原因与按钮 | 未登录实机、已保存/错误 mock、全局读取/重试/迟到响应测试、双端登录子页 Escape |
| 构建和全套检查 | 上述 Swift/Rust/wasm/Playwright/WebKit 日志与退出结果；资源、fmt、diff 检查 |
| 隔离及清理 | QA 进程停止、链接恢复、无关图片恢复；真实模型与账号未使用 |

本阶段完成视觉与状态语义对齐，保留原生目录选择器、侧栏、显式保存和静态桌宠预览。未承诺逐像素一致、真实订阅登录或修改认证协议；网络代理控件重设计和其余设置属于 H4，技能/插件/专家/快捷动作/记忆属于下一批 H3。后续按独立 PR 推进。

## 隔离和清理

数据在 `/private/tmp/wisp-settings-h1-h2-20260928/{native,webview}`，只使用既有合成数据；模型 URL 为 localhost:9，桌宠为几何测试图。没有使用真实账号、调用授权或模型服务。

本轮原生和 WebView QA 进程已停止，WebView 临时动画 CSS 已从 `webview-appearance-before-animation-qa.json` 恢复；两条 QA Application Support 软链接已恢复原目标。Playwright 静态服务器已停止，无关 research-journey 图片已恢复。实机最后恢复中文、浅色、宽窗，两个隔离库的 custom CSS 均为空。最新版 QA 包和隔离数据保留以供复核。
