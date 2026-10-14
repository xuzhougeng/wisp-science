# Wisp Science for Android

原生 Kotlin / Jetpack Compose 客户端框架，最低 Android 8.0（API 26）。以现有
[iOS 客户端](../ios/README.md)为参照，复用桌面远程访问协议、DTO 夹具和合成演示数据。
手机不运行 Rust Agent、Python 或 SSH；真实任务仍由在线 Wisp 桌面主机执行。

```text
Android ── WSS + AES-256-GCM ── wisp-relay ── 在线 Wisp 主机
```

## 打开与运行

1. 在 Android Studio 打开 **`apps/android`**，不是仓库根目录。
2. 选择 JDK 21，安装 Android SDK Platform 36 和 Build Tools 36.1.0。
3. 使用随工程提交的 Gradle Wrapper；选择 `app`，在 API 26+ 设备或模拟器运行。
4. 初次打开可选择「体验演示项目」，不需要账号、联机码、网络或真实主机。

已固定 AGP 8.11.1、Gradle 8.13、Kotlin / Compose Compiler 2.1.20；Java / Kotlin
输出目标为 JVM 17。[AGP 官方兼容表](https://developer.android.com/build/releases/agp-8-11-0-release-notes)
支持 API 36 和 Gradle 8.13。Wrapper 来自 Gradle 官方仓库，JAR 下载时已核对官方校验值，
发行包 SHA-256 固定在 `gradle/wrapper/gradle-wrapper.properties` 中。

macOS / Linux（已配置 `JAVA_HOME` 和 `ANDROID_HOME`）：

```bash
cd apps/android
./gradlew :app:assembleDebug
./gradlew :app:installDebug
```

Windows PowerShell：

```powershell
cd apps/android
.\gradlew.bat :app:assembleDebug
.\gradlew.bat :app:installDebug
```

APK 位于 `app/build/outputs/apk/debug/app-debug.apk`。本机 SDK 路径可放在
`local.properties` 的 `sdk.dir`，该文件不会提交。首次构建需要下载依赖；自动测试不连接真实
Wisp 主机、不调用模型，也不依赖任何科研服务或密钥。发布签名和应用商店分发不在本次范围内。

## 当前范围

| 功能 | Android 框架版 |
| --- | --- |
| 工程 | 独立 `app` + 纯 JVM `remote` 模块，中文、明暗外观、字号适配、居中单栏布局 |
| 连接 | 完整远程链接或 HTTPS 地址 + 联机码，保留反向代理路径前缀 |
| 协议 | 与 Rust / iOS 一致的 AES-GCM 帧、方向认证、nonce、递增序列、请求关联、命令能力检查 |
| 生命周期 | 前台轮询，后台断开，回到前台重连；发送和审批要求当前连接的新快照 |
| 项目与对话 | 分组项目卡片、对话数量／待处理状态、跨项目待处理入口（主机一次返回全部项目）、带活动日期的会话列表、新建对话、最近快照、文本消息、停止、一次性批准／拒绝；运行结束后打开的对话会标记为已读，清除未读回复的待处理状态 |
| 问题与 Run | 原生问题文本和选项（选项填入输入框，确认发送）；失败／超时等记录直接显示，成功运行折叠为「已完成的记录」；展开查看日志 |
| Markdown 与文件 | 对话和 Markdown 文件原生渲染标题、强调、列表／任务列表、引用、代码块、GFM 表格与链接；表格／代码横向滚动；目录、文本及图片预览 |
| 科研日历与研究历程 | 全局月历、项目筛选、周一为首列、日期标记、今天／月份切换、当天记录及关联对话；项目内研究历程（独立翻月，不改变日历标签的月份和选中日期）；使用设备时区 |
| 本机研究工具 | 按逾期／今天／接下来／未设日期分组的文字清单、改期、完成／恢复；记录本观察入口、示例、帮助和编辑；原子文件持久化 |
| 演示 | 复用 iOS RNA-seq 数据，浏览文件、日历与待处理对话、发送预设回复、演示审批；与 iOS 演示一致，未读、待审批或待回答的对话显示为待处理；不创建 socket |
| 导航 | 对话／日历／清单／记录本四个底部标签，各自保留导航和滚动位置；项目会话列表保留底栏，对话及编辑页隐藏；系统 Back 和窗口级 Escape 每次只返回一层，标签根页返回对话标签 |

这是 Android 基础框架，尚未达到 iOS 全部功能。明确的后续工作：数学公式排版、
更早的历史分页、图片上传、排队消息、磁盘读取缓存、清单冲突同步、
记录本发送、平板多栏布局。当前运行期间发送按钮停用，不会把输入悄悄排队。

iOS 的 Vision OCR、相机／相册桥接、图像预处理和 Apple 专有交互暂不移植。
`NotebookTextRecognizer` 留有独立接口，当前实现明确报告不可用；不会申请相机或照片权限。
Android 采用自身的 Keystore 和生命周期机制。基础加密属于跨平台协议，已用 JCA 实现，未留空。

## Markdown 阅读

使用固定版本的 [CommonMark Java](https://github.com/commonmark/commonmark-java/tree/commonmark-parent-0.24.0)
解析正文和 `.md` 文件，以 Compose 原生组件渲染，不使用 WebView。支持标题、粗体／斜体／删除线、
行内代码、围栏代码、有序／无序／任务列表、引用、分隔线、GFM 表格、链接和项目图片。
宽表格及代码可以横向滚动；文本可以选择复制。数学公式目前保留为文本。

- 点击项目内链接打开文件预览；Markdown 文件的相对路径以所在目录为基准。
- 项目图片经 `native_conversation_image` 读取。正文中的图片若绑定了资源版本，始终使用该绑定，
  绑定失效会显示读取失败，不退回当前路径的新版图片。演示图表从随包资源读取。
- 外部 HTTP(S) 链接在点击后交给浏览器；外部图片显示说明及打开入口，不自动下载。
  HTML 作为文本展示；不执行脚本，不打开 `javascript:`、`data:`、`file:` 等活动引用。
- 图片读取与当前项目、对话和连接绑定；退出页面或重连时不应用旧响应。解码前检查体积和尺寸。

## 连接与数据边界

主机配置见[远程接入说明](../../docs/remote-access.md)。可粘贴
`https://relay.example.com/wisp/remote#联机码`；项目／对话片段暂不自动导航。
仅部署中继不能代替在线桌面主机。

- 联机码不出现在请求 URL、日志或明文偏好设置中；只传递派生的 rendezvous 标识。
- 首次认证成功后，以 Android Keystore 密钥加密凭据，再原子写入 `noBackupFilesDir`。
  应用禁用备份，不写 SQLite，不保存主机的模型密钥。连接表单也不进入 saved-instance-state。
- 不跟随 WebSocket 重定向；只允许 HTTPS / WSS。断线、超时或取消不会重放操作。
  若结果未知，对话保持禁止操作，用户核对主机后主动选择「已核对，允许继续」。
- 项目和会话读取只在内存保留，尚无 iOS 的离线磁盘缓存。切换服务器或演示会清空远程视图和草稿。
- 文字清单、记录本是**设备级**数据，保存在应用私有目录，暂不绑定远程项目或同步。
  忘记连接会保留这些记录；清除应用数据或卸载会删除它们。
- 新清单事项默认安排在今天，点事项修改文字／日期；日期为空时归入「未设日期」。旧版无日期记录仍保留在该组。完成事项默认折叠，可展开恢复；不发送系统提醒。
- 日历显式查询可见项目，每批最多 50 个。月份标记和所选日期分别读取，避免月记录截断导致漏掉当天记录；缺失／不可用项目和截断均显示提示。重连、切换服务器时清除日历读取结果，不做磁盘缓存。
- 演示选择会跨重启保存，演示期间不读取或改写凭据；本次演示消息和审批只留在内存。
  重置演示恢复原始数据。演示中的记录本仍是用户主动创建的本机记录，不写入合成数据。

## 结构

```text
remote/  RemoteConfiguration / RemoteCrypto / RemoteConnection
         RemoteService / DemoService / RemoteSnapshot / LocalEntries / Markdown
app/     WorkspaceModel / WorkspaceScreen / MainActivity
         ResearchScreens / WispComponents / MarkdownContent
         DeviceCredentials / DeviceEntries
```

共享协议的源头仍是 `crates/wisp-dto` 和 `crates/wisp-sync/src/remote.rs`。Kotlin 仅实现客户端投影，
通过仓库 `contracts/` 的既有 JSON 夹具验证，未新增服务端接口或 wire schema。
Gradle 直接打包 `apps/ios/WispScience/DemoData`，无重复维护的演示副本。
所有操作图标从 `ui/src/app_support/messages.rs` 的 `compose_icon()` 导出：

```bash
python3 apps/android/scripts/export_icons.py
```

该脚本同时同步 iOS 的现有品牌启动图标，不引入第二套操作图标。

## 验证

遵守仓库规则，执行本地测试前需用户明确授权。

```bash
cd apps/android
./gradlew :remote:test :app:testDebugUnitTest :app:lintDebug
./gradlew :app:connectedDebugAndroidTest
```

单元测试使用虚拟 socket、虚拟时钟、共享 DTO 夹具和随包合成数据，覆盖 URL / 密钥派生、
Rust 帧兼容、方向认证、防篡改、请求排序、超时、重连不重发、旧连接操作门禁、项目／路径隔离、
演示回复与审批、本机数据序列化、旧记录日期兼容、清单分组、日历批次／缺失项目／截断／夏令时日期边界、CommonMark／GFM 结构、链接路径与图片版本绑定。仪器测试在启动 Activity 前启用演示，不需要真实主机，覆盖
打开后立即 Escape、父页面保留、独立标签导航、日历筛选与日期切换、清单改期和恢复、记录本示例／帮助、运行折叠、待处理入口、Markdown 表格和图片渲染、报告链接跳转、演示发送与 Activity 重建。

手工冒烟步骤：打开演示 → 项目 → 样本质控与报告，确认标题／表格／图片呈现，点击「打开完整报告」再打开相对路径指标链接；进入项目文件，连续返回确认一次一层；发送文字并确认是本地预设回复；
在「分析流程与审批」批准或拒绝；从首页待处理入口打开对话；切换日历月份／项目／日期后打开关联对话；
在项目列表与其他标签间切换，确认返回项目位置；离线新建清单并改期／完成／恢复；创建记录、查看示例与帮助，按 Escape 确认只返回一层；旋转屏幕和重启应用后查看；检查深色、大字号、
软键盘遮挡与系统 Back。真实连接另需自有在线主机，验证断线重连、旧快照禁用操作、发送中断时不重复提交。
