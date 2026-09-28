# 首批 UI 对齐实施与验收

2026-09-28；基于 `9ea5f411f7b90f4ba8c44f698f84ce0f7086e7b5` 的未提交工作树，版本仍为 1.15.0。本批完成 A、B1、B2、D1，C 只核对输入区能力接口，不包含其后续 UI 接入。

- [更新后截图画廊](index.html)
- [阅读方案与输入区能力契约](../../../superpowers/specs/2026-09-28-native-reading-and-composer-contract.md)
- [完整页面审计与后续计划](../../../superpowers/plans/2026-09-28-swiftui-webview-next-ui-alignment.md)
- [构建与验证清单](evidence.json)

## 本批交付

| 范围 | 实现 | 验收证据 |
| --- | --- | --- |
| 设置语义 | 自动继续明确对应输出 token 截断，关闭后仍显示次数；补模型切换语义压缩与空闲小时；写明保存及即时生效范围 | 实机读取 100/10/24；非法 `abc` 被拦截，0 保存重载后仍为 0，再恢复 24；取消恢复原值，取消同时清除过期错误；默认值、未知字段保留和非法数值有模型测试 |
| 同步选择框 | 标签按实际存值映射，未知值原样显示 | 隔离数据实际为 `folder`，实机显示“同步文件夹”；folder/relay/未知值测试 |
| 消息阅读 | 共享离线 Highlight.js 语法，原生只读任务框；保留单一可选择文本；增加复制、侧聊引用、收藏与真实时间/工具耗时/模型元数据 | 同一富文本 fixture 的高亮、任务框、表格及消息动作实机通过；整条引用保留公式源码且只进入草稿；跨块复制、选区保持、收藏文本定位与 Unicode 自动化通过 |
| 公式 | 固定 SwiftMath 1.7.3，离线原生矢量排版；行内基线与居中块公式；正文和产物详情复用；复制/辅助功能保留源码 | 浅色、深色、窄列渲染图人工查看；分式、上下标、非法/过宽回退、复制、表格内公式、代码/货币避让测试通过；实机公式详情可读 |
| 产物/Agents | 按类型分组计数、表格行列、当前消息页范围、列表/网格、空态与筛选提示；委派关闭说明；宽度约束与窄窗暂收侧栏 | 相同会话为公式 1 + 表格 1（2 行 × 2 列）；切空会话变 0，无残留；约 901 pt 窗口仍可使用输入区；立即 Escape 先关闭公式预览，父面板保留，第二次关闭面板 |
| 输入区 | 核对读取/写入命令、项目/会话范围、只读/运行/ACP 约束与缺失 DTO | 契约表覆盖 Local/Python/R、默认环境、Agent/Plan、Fast、模型、@/#/slash、附件、发送/排队/停止、草稿/IME；没有添加假能力入口 |

主要源码集中在 `apps/macos/Sources/WispProjectBrowserUI/` 的设置、Markdown、消息和面板文件。新增 `NativeCodeHighlight`、`NativeMathContent`、`NativeMessageActions`、`NativeArtifactCollection` 分别承载语法范围、可复制公式附件、消息动作和产物分组；协议在 `wisp-dto` 只追加可选元数据，Rust 宿主从持久化大纲关联所属轮次，旧快照仍可解码。构建脚本打包 SwiftMath 字体资源及许可。

## 验证

完整命令、结果和日志位置见 `evidence.json`。Swift 渲染测试用 `WISP_NATIVE_SNAPSHOT_DIR` 全部启用；运行时不依赖网络或真实科学计算环境。未改 MCP，无须新增 MCP smoke。

| 检查 | 结果 |
| --- | --- |
| Rust 全工作区 | 2381 passed，0 failed，0 ignored |
| Swift 全套（启用全部渲染） | 307 passed（291 UI + 16 client），0 failed，0 skipped |
| wasm check、npm ci | 通过 |
| Playwright 全套 | 首轮 877 passed、4 timeout、2 skipped；4 个失败项以单 worker 原样重跑全部通过。未修改代码或测试规避失败 |
| 格式、资源/设置合同同步、两包严格签名 | 全部通过 |

2 项跳过为需要真实 Motif MCP App 的专项测试。首轮超时属于初始化/教程截图路径，单独重跑未复现；保留首轮和复跑日志，不能称首轮全绿。

本机包：`target/native-macos-qa/Wisp Science QA.app`、`target/ui-alignment-20260928/Wisp WebView QA.app`。两者为 arm64 debug、独立 QA 标识、ad-hoc 签名，供验收使用。

## 数据与实机路径

原生隔离数据为 `/private/tmp/wisp-ui-alignment-20260928/implementation-native`，WebView 为同级 `webview`；项目 `toolbar-qa`，富文本会话 `parity-http-rich`，空会话 `toolbar-qa-empty`。原生克隆的根库和各项目库均重绑定到隔离路径。设置保存仅修改 QA 根库，0 小时测试后恢复为 24；侧聊引用被移除，没有发送消息或启动模型。没有改变委派授权，没有登录、同步、导出或删除正式数据。

截图 `.jpg` 是 Computer Use 原始窗口截图，`.png` 是 Swift 原生渲染测试输出，未经图像修饰。原生普通窗口 1056×768、窄窗约 901×768，WebView 1100×760；不同系统尺寸仅用于结构/交互对照，不声称像素一致。同步选择截图的 AX 文件为增量树，其余为完整树。Agents 图在最终块公式居中微调之前捕获，仅用于说明卡验收；最终正文效果见富文本及窄窗图。

复现流程：关闭 QA 应用；将原生 QA support 数据软链接指向本批隔离目录，用 `WISP_BROWSER_DATABASE` 指定对应根库；打开上述项目/会话；检查设置、产物、引用和 Escape；结束后关闭应用和 helper，恢复软链接。最终清理状态记录于证据 JSON。

## 边界与后续

- SwiftMath 不覆盖所有 KaTeX 宏；非法、不支持、过长或超宽公式显示可换行源码。离线资源已打包，但首次构建拉取依赖可能需要网络。
- 输入区仅完成契约核查。运行环境状态可优先接入；Plan/Fast/结构化引用和完整记录/模型视角需要独立协议变更。其余页面继续按既定后续计划实施。
- 全页深色/英文实机对照、真实 IME、长会话性能和真实远端执行不在本批交付范围。公式的深浅主题与中英混排有渲染和语义测试；不把它们冒充全应用专项验收。
- 实机首次打开项目出现一次窗口/辅助功能查询无响应，重启隔离 QA 应用后，设置与完整消息/面板路径均可完成，后续重复打开未复现。采样停在 SwiftUI accessibility 更新；原始审计已记录 AttributeGraph 警告。未将未定位的系统/布局问题宣称修复；若再次出现，应独立收敛复现条件。

辅助功能文本在提交前仅规范化行尾空白与末尾换行；截图字节保持原样。
