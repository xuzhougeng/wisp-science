# macOS #1250：保存面板重绘重入

Windows #1265 的 Tao 0.37.0 升级不覆盖这个问题。macOS 补丁现在在
`vendor/tao`：`handle_redraw` 在已处于 callback 时改为 `queue_redraw`，
由后续 `cleared` 投递 `RedrawRequested`。复现、补丁前栈和运行方式见
[macos-redraw-reentrancy-reproduction.md](macos-redraw-reentrancy-reproduction.md)。

#1250 当前虽已关闭，仍应保留原生回归；不能只因为升级到 0.37.0 就标记已修复。

## 当前证据

原报告来自 macOS 27.0 / Apple M5 Max。spindump 明确显示 `main -> main`
自锁，外层为 `NSSavePanel` sheet 弹出，内层为 AppKit/CA 重绘回调。
原发布包没有匹配 dSYM，因此具体 Tao 函数名仍是根据源码与调用结构推断，
不是原始地址已经完成符号化。

在 2026-09-16 核对的 Tao 0.37.0 与 dev 提交
`27fe28c73ace307eb0b1d1abf457508410f1340e` 中，macOS `app_state.rs` 相同：

```text
AppState::cleared
  HANDLER.handle_user_events
    callback.lock()                    // 外层持有
      Tauri run_on_main_thread task
        rfd / NSSavePanel beginSheetModalForWindow
          nested AppKit run loop / CoreAnimation flush
            Tao view::draw_rect
              AppState::handle_redraw
                HANDLER.handle_nonuser_event
                  callback.lock()      // 同一线程再次申请
```

源码入口：

- [app_state.rs:203](https://github.com/tauri-apps/tao/blob/tao-v0.37.0/src/platform_impl/macos/app_state.rs#L203)：`handle_nonuser_event` 的 callback 锁。
- [app_state.rs:214](https://github.com/tauri-apps/tao/blob/tao-v0.37.0/src/platform_impl/macos/app_state.rs#L214)：`handle_user_events` 的 callback 锁。
- [app_state.rs:366](https://github.com/tauri-apps/tao/blob/tao-v0.37.0/src/platform_impl/macos/app_state.rs#L366)：上游 `handle_redraw` 缺少 `in_callback` 防重入；Wisp 的 `vendor/tao` 补丁已补上。
- 同文件的 `wakeup` / `cleared` 已检查 `in_callback`，`queue_redraw` 已提供去重的待重绘队列。
- Wisp 入口包括 `src-tauri/src/session_export.rs`、`app_commands.rs`、
  `artifact_commands.rs` 和 `project_transfer.rs` 的原生文件面板调用。

## 1. 在 Mac 上保留匹配符号

拉取 Windows 修复分支后，先确认新依赖在 macOS 可编译：

```bash
export WISP_CATALOG_OFFLINE=1
cargo tree --locked -p wisp-tauri -i tao
cargo build --locked -p wisp-tauri
```

预期依赖链为本地 Tao 0.37.0（`vendor/tao`）→ runtime-wry 2.12.0 → Tauri 2.12.0。
使用 debug 构建复现，保留**同一构建**的可执行文件和调试产物、Git SHA、
Cargo.lock、OS build、CPU 架构及操作步骤。完整 UI 可用 `cargo tauri dev`；
该命令会自动使用 `tauri.macos.conf.json` 的前端命令。

如问题仅在优化构建发生，保留 release 调试信息并禁止 strip 后再复现：

```bash
CARGO_PROFILE_RELEASE_DEBUG=2 CARGO_PROFILE_RELEASE_STRIP=none \
  cargo tauri build --no-bundle
dsymutil target/release/wisp-tauri -o target/release/wisp-tauri.dSYM
dwarfdump --uuid target/release/wisp-tauri
dwarfdump --uuid target/release/wisp-tauri.dSYM
```

UUID 必须匹配；新构建的 dSYM 不能用来解析旧报告中的裸地址。
冻结后在另一个终端采样，手动把 `WISP_PID` 设为被测进程的 PID：

```bash
mkdir -p test-results/macos-1250
sample "$WISP_PID" 10 -file test-results/macos-1250/sample.txt
lldb -p "$WISP_PID" -o 'thread backtrace all' -o detach -o quit
```

若系统限制调试器附加，可先用 Activity Monitor 的 Sample Process / Spindump
获取栈。将符号化后的锁和回调摘要贴到 issue；完整报告和应用日志保留本地。

## 2. 先建立两个层次的复现

1. **确定性的 Tao 重入回归**：在隔离原生窗口的 user-event callback 内，
   让 Tao 所属 `NSView` 标记并立即执行 display，或在 Tao 内部测试中直接触发
   `AppState::handle_redraw`。确认日志确实进入 Tao `draw_rect`，且外层
   `in_callback=true`。这样可验证 callback 锁自重入，不依赖低概率 sheet 动画。
   设置独立工作线程截止时间；UI 自锁时不能依靠 `app.exit` 完成退出。
2. **真实 AppKit sheet 回归**：使用 `tauri-plugin-dialog` 打开保存、打开文件和
   目录选择面板，覆盖确认/取消、连续开关、窗口切换、缩放与退出。
   同时触发原生视图重绘并记录重入是否发生；仅让 WKWebView 的 JS/CSS 高频动画
   运行不能证明触达 Tao 的 `draw_rect`。

最小程序使用独立 identifier/profile，不加载用户数据库或后台分析任务。
旧版本必须捕获 callback 锁的自等待；补丁后同一触发序列必须完成，不能仅依据
“这次手动没卡住”判断有效。

## 3. 建议补丁边界

先在 Tao 0.37.0 上处理 `handle_redraw`，保持这次 Windows 依赖组合：

- 进入 redraw dispatch 前检查并设置 `in_callback`。已经在 callback 内时，
  **不要再次获取 callback mutex**，也不能清除外层的 `in_callback` 标记。
- 重入时用待重绘队列记录窗口并唤醒后续事件循环；退出外层 callback 后再投递。
  直接 `try_lock` 失败就丢弃事件可能带来空白/缺帧，不应作为未经验证的完整修复。
- 正常分支可靠恢复 guard，核对退出、销毁窗口、重复 redraw 的去重与清理。
  确认队列不会自旋，也不会因面板关闭后没有新输入而永远不刷新。
- 不要把 callback mutex 简单换成递归锁：其内部还访问 `FnMut` / `RefCell`
  和 control-flow 状态，允许再次调用回调可能转成借用冲突或破坏事件顺序。
- 使用固定提交或明确的本地 Tao patch，保留独立测试，再向 Tao 上游提交最小复现
  和补丁。以后正式版本包含修复时移除临时覆盖。

这些是待在 macOS 验证的实现方向，本 Windows 分支没有修改 Tao 的 macOS 源码。

## 4. 完成标准与 issue 更新

至少应记录以下结果：

- 确定性原生重入：补丁前失败，补丁后多次独立进程通过；主线程和面板均可响应。
- 保存/打开/目录选择的确认与取消正常，窗口切换后继续绘制，没有递归借用 panic。
- 证明被延迟的 redraw 最终送达；多窗口、窗口销毁和退出时没有事件积压或崩溃。
- macOS 完整 Wisp 构建、相关测试和真实面板操作通过；原报告的 macOS 27 环境优先。
- 回跑 Windows #1265 原生回归，保留 Linux/MSRV CI 结果；分别记录未验证的系统。

更新 #1250 时关联 Windows #1265，说明二者同属原生事件重入问题，但锁、触发点、
补丁和实机证据不同。不能因升级到 0.37.0 或 issue 处于 closed 就标记 macOS 已修复。
