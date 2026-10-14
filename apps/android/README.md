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
| 项目与对话 | 项目列表、对话列表、新建对话、最近快照、文本消息、停止、一次性批准／拒绝 |
| 问题与 Run | 原生问题文本和选项（选项填入输入框，确认发送）；展开运行记录及日志 |
| 文件 | 目录与服务端文本预览，文本可选择复制 |
| 研究历程 | 项目内按月读取、前后月份、截断提示、跳转关联对话；使用设备时区 |
| 本机研究工具 | 离线文字清单、完成状态、文字记录本；原子文件持久化 |
| 演示 | 复用 iOS RNA-seq 数据，浏览文件、发送预设回复、演示审批；不创建 socket |
| 导航 | 完整页面的统一返回栈，系统 Back 和窗口级 Escape 每次只返回一层 |

这是 Android 基础框架，尚未达到 iOS 全部功能。明确的后续工作：富 Markdown／表格与图片渲染、
更早的历史分页、图片上传、排队消息、跨项目待处理、全局科研日历、磁盘读取缓存、清单冲突同步、
记录本发送、平板多栏布局。当前运行期间发送按钮停用，不会把输入悄悄排队。

iOS 的 Vision OCR、相机／相册桥接、图像预处理和 Apple 专有交互暂不移植。
`NotebookTextRecognizer` 留有独立接口，当前实现明确报告不可用；不会申请相机或照片权限。
Android 采用自身的 Keystore 和生命周期机制。基础加密属于跨平台协议，已用 JCA 实现，未留空。

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
- 演示选择会跨重启保存，演示期间不读取或改写凭据；本次演示消息和审批只留在内存。
  重置演示恢复原始数据。演示中的记录本仍是用户主动创建的本机记录，不写入合成数据。

## 结构

```text
remote/  RemoteConfiguration / RemoteCrypto / RemoteConnection
         RemoteService / DemoService / RemoteSnapshot / LocalEntries
app/     WorkspaceModel / WorkspaceScreen / MainActivity
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
演示回复与审批、本机数据序列化。仪器测试在启动 Activity 前启用演示，不需要真实主机，覆盖
打开后立即 Escape、父页面保留、演示发送、Activity 重建、离线记录。

手工冒烟步骤：打开演示 → 项目 → 对话 → 文件，连续返回确认一次一层；发送文字并确认是本地预设回复；
在「分析流程与审批」批准或拒绝；离线新建清单／记录，旋转屏幕和重启应用后查看；检查深色、大字号、
软键盘遮挡与系统 Back。真实连接另需自有在线主机，验证断线重连、旧快照禁用操作、发送中断时不重复提交。
