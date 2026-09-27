# 订阅账号页面验证

本次同步到 origin/main 的 28a8b0ef（v1.15.0），复用远程 SuperGrok / xAI 实现。

- [模型 → 订阅账号](subscriptions-zh.png)：与 API 模型、ACP Agents 并列，ChatGPT 和 SuperGrok 各自管理账号与模型列表。
- [添加模型](subscription-model-zh.png)：只有模型 ID、别名；支持使用同一订阅连续添加多个模型。

专项测试覆盖中英文标签分类、两个订阅来源、保存账号不创建模型、多个模型与别名、编辑时保留订阅适配器和参数、浏览器回调输入保持、设备码错误与重试、保存失败重试、迟到挑战取消、Escape 返回层级和窄窗口。截图使用模拟账号与模拟后端。

原生 macOS 同步了分类、xAI 设备码授权、仅两项模型编辑和账号独立保存；WinUI 预览尚未提供订阅登录页，Windows 正式 WebView 使用本次共用实现。

真实环境复验：在「模型 → 订阅账号」登录 ChatGPT / SuperGrok，保存账号，添加两个模型并分别发起对话。若出现 HTTP 403，按消息检查 Wisp 的代理与账号访问权限。模拟授权成功不能证明真实账号权限或外网可达性。

## 验证结果

- `cargo fmt --all -- --check`、WebView wasm 编译、原生资源与接口同步检查通过。
- `WISP_CATALOG_OFFLINE=1 cargo test --workspace`：2,372 通过，0 失败。
- `swift test`：280 项 UI 测试（17 项可选渲染跳过）与 15 项 core 测试，0 失败。
- `npm ci` 完成；Playwright 全量 882 项：878 通过、2 跳过、2 首次超时。两项超时分别是 `chat-responsive` 的 Inspector 布局与 `compacted-history` 的旧历史加载；未改代码，单独复跑 2 项均通过。
- 本次新增的 9 项订阅测试在全量运行中全部通过。
