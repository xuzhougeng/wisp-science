# Wisp Science for iPhone and iPad

原生 SwiftUI 客户端，最低支持 iOS / iPadOS 17。连接现有的 Wisp 网页远程访问服务，
无需另建 iOS 后端，不在手机上启动 Rust Agent、Python 或 SSH 进程。

```text
iPhone / iPad ── WSS、端到端加密 ── wisp-relay ── 在线 Wisp 主机
```

**中继不执行计算。** Agent、模型凭据、项目和 Run 均在 Wisp 主机上。
目前服务端通道由桌面宿主提供；仅部署 `wisp-relay`，或仅运行 CLI，不能代替在线宿主。
主机如何部署和启用远程访问见 [远程接入说明](../../docs/remote-access.md)。

## 打开与安装

1. 用 Xcode 16 或更新版本打开 `apps/ios/WispScience.xcodeproj`，选择 `WispScience` scheme。
2. 选择 iPhone 或 iPad 模拟器。工程不依赖第三方 Swift 包；`WispRemote` 是本地 Swift Package。
3. 真机运行时，在 **Signing & Capabilities** 中选择自己的开发团队，并按需修改 Bundle Identifier。
4. 在 Wisp 主机启用「设置 → 远程接入 → 网页远程访问」，复制远程访问链接。
5. 在 App 中粘贴 `https://relay.example.com/remote#联机码`，或分别填写中继 HTTPS 地址和联机码。
   使用反向代理前缀时，如 `/wisp/remote#…`，App 会保留 `/wisp/`。

手机使用的是**联机码**，不是中继管理员的 `WISP_RELAY_TOKEN`。连接成功后凭据保存在设备
钥匙串中；「连接 → 忘记连接并更换服务器」会删除本机凭据，并清空本次会话的内存数据。
同时删除当前服务器的本机缓存。
App 当前保存一个连接。链接中的项目、会话导航片段不会自动打开对应内容。

## 本机缓存与离线查看

已查看的项目列表、对话列表、对话快照、加载过的历史页、文件目录、文本预览和图片保存在本机缓存。
打开页面时先显示缓存，再请求服务器更新；相同的并发读取合并为一次请求。
断网或主机离线时，可进入已缓存的项目与对话，打开之前查看过的文件；没有缓存的内容会提示联网加载。
缓存视图标明保存时间，项目／任务状态以联网刷新为准。旧快照中的发送、审批、停止和队列操作
保持禁用，重连后拿到当前连接的新快照才恢复；没有离线发送队列，不会自动重放任何变更操作。

- 缓存按中继地址和联机码派生的服务器标识隔离，键还包含项目、会话与文件路径。
- 默认总量上限 50 MB、单条上限 8 MB，最多 500 条；保留 30 天，超限优先淘汰较久未访问的内容。
- 缓存使用系统 Caches 目录，排除备份，并采用 iOS 文件保护；系统可回收，卸载 App 会移除。
- 联机码仍只放钥匙串，不写入缓存；不自动同步完整项目或大型科学数据。
- 「连接 → 离线缓存 → 清除本机缓存」清理所有服务器的缓存，保留联机码及当前页面内容；后续读取会重新缓存。
- 演示模式直接读取随包资源，不使用远程缓存，也不会混入真实服务器的缓存。

## 离线演示与审核体验

连接页点击 **「体验演示项目」**，再选择 **「RNA-seq 质控 · 演示」**。无需填写服务器地址、
联机码或账号；飞行模式下也可使用。入口和 `DemoData` 资源在 Debug、Release 构建中均保留，
与仅用于自动测试的 `PreviewHost` 无关。

- 「样本质控与报告」：6 个合成样本、Markdown 表格、比对率图表、成功 Run 的命令／日志。
- 「分析流程与审批」：失败 Run 示例，以及可批准、可拒绝的本地审批。
- 「问题与选项」：三项可点击的模拟选项，可切换或取消选择、填写补充说明，再提交回答。
- 「文件」：样本 CSV、质控指标 CSV、Markdown 报告、PNG 图表、只供阅读的 Python 示例脚本。
- 可以发送文字、点选建议；回复明确标为本地预设内容，不调用 AI 或执行分析。

页面始终显示「离线演示 · 合成数据」。提示条独立排列在系统导航栏上方，
与「演示设置」「待处理」及返回按钮分开布局；大字号时提示文字可换行。
演示不会联网、上传图片、执行命令、写项目文件，
也不读取或改写钥匙串。进入演示会断开已有远程连接，保留已保存的联机码。
演示模式选择保存在本机 `UserDefaults`；重启后仍进入演示，避免自动连接真实主机。
本次消息、选项回答和审批仅保存在内存；重启、退出或「演示设置 → 重置演示数据」恢复初始内容。
点击顶部「退出演示」返回连接页。演示不提供新建对话、图片上传或真实计算。

演示数据和图表来源于 `scripts/build_demo_dataset.py` 中显式定义的合成数值。修改后可运行：

```bash
uv run --no-project --with matplotlib python apps/ios/scripts/build_demo_dataset.py
```

仅生成资源时需要 Python / matplotlib；安装后的 App 不依赖它们。
`PrivacyInfo.xcprivacy` 中以 `CA92.1` 声明仅用于本 App 模式偏好的 UserDefaults 访问，
对应 [Apple 的 API 使用原因说明](https://developer.apple.com/documentation/bundleresources/app-privacy-configuration/nsprivacyaccessedapitypes/nsprivacyaccessedapitypereasons)。

可在 App Review 备注中说明：

> No account or server is needed to explore the bundled demo. On the connection screen, tap
> “体验演示项目” (Try demo project), then “RNA-seq 质控 · 演示”. The three conversations demonstrate
> reports, charts, run logs, project files, approval interactions and selectable question options. All demo data is synthetic;
> messages receive clearly labeled local preset replies and no commands or AI analysis are executed.
> To leave the demo, tap “退出演示” (Exit demo). Real remote execution requires the user's own Wisp host.

Debug 构建使用 `ONLY_ACTIVE_ARCH=YES`，与本地 Swift Package 在所选模拟器上的架构一致。
Release 保持全架构构建。若旧工程曾显示 `Unable to resolve module dependency: WispRemote`
并伴随 `built for incompatible target`，重新加载更新后的工程再构建；仍有旧诊断时可执行
**Product → Clean Build Folder** 后重建，不需要下载或手动添加 `WispRemote`。

## 与网页端对应的功能

| 功能 | 原生客户端 |
| --- | --- |
| 项目和对话 | 项目列表、对话列表、新建对话、下拉刷新 |
| 对话阅读 | 最新快照、较早消息、Markdown 标题／表格／代码／链接／图片、工具与思考折叠 |
| 发消息 | 文本、最多四张图片、运行期间排队、取消排队、停止 |
| 审批 | 批准一次或拒绝；不提供永久授权 |
| 问题选项 | 原生 `ask_user` 问题卡片、单选、补充说明或直接输入回答；确认后提交 |
| Run | 会话所属 Run 状态、命令与输出尾部 |
| 文件 | 目录、文本和图片预览、服务端已转换的 PDF／Office 文本 |
| 待处理 | 跨项目待处理列表、跳转对话、阅读后标记已读 |
| 设备布局 | iPhone 逐层导航；iPad 项目／对话／内容三栏，支持横竖屏和多任务窗口缩窄 |
| 生命周期 | 前台轮询、后台断开、回到前台重连并重新读取状态 |

iOS 发起的任务默认对修改类工具请求审批。在电脑端为**同一条会话**开启“完全权限”后，正在执行和后续的 iOS 回合都会采用该授权，包括 `run_in_context`，无需每次重新批准。关闭后恢复审批，重启电脑端 Wisp 后授权失效；每轮只使用目标会话的授权。显式禁止、研究职责的“先问我”和课题组来信的审批要求仍然生效。

界面使用中文、系统字体、动态字号和系统明暗外观。
项目列表与对话列表使用单行导航栏，标题与连接、待处理或新对话操作共用一行，减少列表上方留白。
对话采用中性背景与深色正文，Run／工具记录为紧凑的折叠行；
成功、失败和待审批状态分别使用绿色、红色和琥珀色，始终保留文字状态。
连续的工具调用、思考、文件变更路径和已结束轮次的中间说明，默认合并为一条「执行过程」。
摘要显示步骤数、去重后的文件数与失败数；展开后按原顺序浏览，工具输入／输出仍可单独展开。
最终回答、提问、计划及系统提示保持可见；当前运行中的说明和待审批卡片也不会收进执行过程。
生成图片／视频、完成结果和运行监控等工具保留独立入口。普通刷新及追加步骤保留展开状态，
切换对话或快照历史窗口变化时重新收起。
运行记录默认显示失败、进行中等非成功状态；已完成的记录合并为一行，点开后可逐条查看命令和日志。
用量统计默认折叠，展开后显示输入、输出、缓存和上下文等数值；原始 JSON 在二级详情中保留。
`question` 消息解析为问题和选项，不显示协议 JSON 与给 Agent 的内部备注。默认不预选；点选后可以
切换或取消选择，允许自由回答的问题可填写补充说明，也可不选任何选项直接输入回答。
「提交回答」沿用原生会话的消息发送通道，包含选项标题、说明及补充文字，不覆盖主输入框的草稿或图片。
后续用户消息或成功提交会将问题标为「已回答」；提交中禁止重复操作，明确失败后保留选择供重试。
若发送超时或断线而无法确认结果，显示待确认状态，不自动重发；需刷新或到主机确认结果。
缓存、只读、运行中、已过期的问题不能提交；ACP 问题仍需在桌面端回答。无法解析的旧问题保留原文。
长对话标题从中间省略，保留开头和结尾，点击标题可查看并选择复制完整标题及所属项目。
向上翻阅时显示「回到最新」，点击返回底部；自动跟随依据底部是否接近可见区域判断，避免懒加载提前触发跟随。
建议问题左对齐、完整换行，点击仅填入草稿；输入区整合图片、停止和发送按钮。
大字号时 Run 的状态移到标题下方，图片入口与发送操作分行；iPad 对话与输入区居中，内容最大宽度为 780 pt。
图片、发送、折叠及向下箭头从 `ui/src/app_support/messages.rs` 的 `compose_icon()` 导出，
没有引入第二套图标。修改共享图标后运行 `python3 apps/ios/scripts/export_icons.py` 更新 iOS SVG 资源。
硬件键盘 `⌘ Return` 发送消息；窗口级 Escape 关闭最上层 App 面板并保留对话。

与当前网页版相同，ACP 对话只读；终端、Notebook、模型／Plan 设置、Run 取消／收获、
非图片上传和文件下载需要在主机完成。SVG 当前显示源码；PDF／Office 不提供原版排版预览。
没有后台常驻连接或推送通知；离线内容限于已缓存的读取结果，不是完整项目副本。
断线期间输入的草稿保留在内存中，退出 App 后不保留。

## 协议和实现边界

- `Sources/WispRemote`：纯 Foundation / CryptoKit / Security 连接层、值解码、快照分页和 Markdown 分块。
- `RemoteReadCache`：独立 actor 管理有界磁盘／内存读缓存，过期和容量淘汰；清理会使清理前的写入票据失效。
- `WispScience`：SwiftUI 界面、图片缩小、窗口级 Escape，以及前后台生命周期。
- `WispScience.xcodeproj`：独立通用 iOS App，不修改现有 Tauri 或 macOS 工程。
- wire contract 继续由 `crates/wisp-dto` 定义；Swift 从通用 JSON 值读取已有字段，测试读取仓库的 `contracts/` 夹具。
- 握手和帧格式直接对应 `crates/wisp-sync/src/remote.rs`、`remote.js` 和
  `src-tauri/src/channels/remote.rs`。功能必须先出现在主机的 `hello.commands` 中。
- HTTPS / WSS 必需，拒绝凭据嵌入 URL、查询参数和重定向。不增加 ATS 任意加载豁免。
- 联机码通过方向绑定的 AES-256-GCM 加密帧使用，仅派生 rendezvous ID 出现在网络 URL 中。
  联机码在收到经验证的 hello 后才写入 `WhenUnlockedThisDeviceOnly` 钥匙串。
- 每条连接独立 nonce / 递增序号，发送队列串行化。请求超时或断线会向用户报告结果可能未知，
  不自动重发变更操作。重连只由界面重新查询快照。
- 图片先通过 ImageIO 缩小，再编码为最大 560 KiB JPEG，遵循网页端的单帧上传上限。
- 科学数据仍由服务器持有；只按需读取有界快照、预览和图片。

原生导航采用 Apple 的 [NavigationSplitView](https://developer.apple.com/documentation/swiftui/navigationsplitview)，
网络采用 [URLSessionWebSocketTask](https://developer.apple.com/documentation/foundation/urlsessionwebsockettask)。

## 验证（先获用户许可）

仓库 `AGENTS.md` 要求执行本地测试前获得明确许可。以下命令仅为参考，不表示已执行。

```bash
# 协议、加密、生命周期、DTO 夹具、分页、Markdown；不需要模拟器或网络
swift test --package-path apps/ios

# 编译 iPhone / iPad 通用 App，不签名
xcodebuild -project apps/ios/WispScience.xcodeproj -scheme WispScience \
  -destination 'generic/platform=iOS Simulator' -derivedDataPath apps/ios/DerivedData \
  CODE_SIGNING_ALLOWED=NO build

# 使用 Xcode 已安装的具体模拟器名称，分别验证 iPhone 和 iPad
xcodebuild -project apps/ios/WispScience.xcodeproj -scheme WispScience \
  -destination 'platform=iOS Simulator,name=<installed simulator>' \
  -derivedDataPath apps/ios/DerivedData CODE_SIGNING_ALLOWED=NO test
```

协议测试使用内存 FakeSocket，包含和 Rust／WebCrypto 共用的固定向量、方向绑定、篡改拒绝、
功能协商、发送顺序、超时、取消和断线不重发。共享 DTO 夹具覆盖大整数队列 ID、资源和审批。
App 测试覆盖图片上限；UI 测试覆盖导航、文件、打开后立即 Escape、草稿、审批和停止，
以及 Run 展开／收起、长建议填入草稿、暗色和辅助功能大字号布局。外观用例保存截图供审阅。
列表导航用例检查普通与辅助功能大字号下标题和操作按钮处于同一行、互不重叠且按钮可点击。
对话交互用例覆盖用量默认折叠及损坏数据回退、已完成记录分组、失败记录保留、
长标题详情打开后立即 Escape、长对话回到最新以及大字号下的导航操作。
执行过程分组的纯数据测试覆盖工具与文件路径、最终回答、轮次边界、运行中说明、结果工具、
失败计数、数据完整性以及历史分页标识；UI 用例覆盖整组与单工具两级展开、长路径收起和审批仍然可见。
缓存测试使用临时目录和内存宿主，覆盖重启读取、服务器／项目／会话／路径隔离、容量、过期、损坏文件、
禁止缓存变更命令、清理后的旧响应、离线快照恢复和重连后的审批门禁；UI 用例覆盖缓存文件后离线重启及清理。
UI 测试通过 Debug 专用 `WISP_UI_TESTING` 启用内存加密宿主，不接触真实服务器或钥匙串。
Release 构建不含该宿主。

演示相关测试覆盖本地数据源不创建 socket、不触发认证回调、切换时丢弃旧请求、
全部打包文件可读、路径与会话隔离、预设回复、审批重置和凭据不变；UI 用例从连接页进入演示，
浏览报告、发送演示消息、处理审批并退出。新增测试代码不代表已经执行测试。
选项相关测试代码覆盖 JSON 解析降级、自由回答开关、历史已答状态、草稿和图片保留、重复提交、
运行中／只读／断线／过期快照门禁、失败重试与结果未知状态；UI 用例覆盖本地模拟选项、补充说明、
提交后的已答状态，以及深色外观和辅助大字号。

真机手工检查：在 iPhone 和 iPad 上粘贴已有链接，打开项目并收发消息；让主机请求一次审批；
查看文件和 Run，确认失败状态与输出可读、折叠行整行可点；点选长建议后确认不会直接发送；
选取一张大照片；在明暗外观、大字号、横竖屏、iPad 分屏及硬件键盘下操作；锁屏后解锁，确认状态恢复；
关闭主机再重开，确认没有重复发送。需要真实远程主机的检查不属于自动测试。
对话页手工检查：展开用量和已完成记录，确认状态刷新后仍保持展开；向上翻阅时等待新消息，
确认阅读位置不会跳到底部，再点「回到最新」；点开长路径标题，确认全文可读并能返回原对话。
选项手工检查：在演示项目打开「问题与选项」，切换选项并填写说明后提交，确认出现用户回答和本地预设回复；
返回再进入时保持已答；重置演示后可以重新选择。真实主机发出 `ask_user` 问题后，确认回答出现在相同会话，
旧问题不能重复提交；断网或只读会话中仍能阅读，但提交禁用。

本工程用于开发和真机调试。TestFlight／App Store 分发仍需开发团队、签名、导出加密合规声明、
发布图标校验和审核资料；本次不创建分发凭据或发布 App。
