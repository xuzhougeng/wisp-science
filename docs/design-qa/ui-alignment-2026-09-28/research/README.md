# 研究页面验收记录

2026-09-28：完成 UI 对齐计划 E、F、G 第一阶段：研究历程主页面、正确日期布局的研究日历、论文证据只读工作区。[截图画廊](index.html)。

## 实机与合同证据

使用 `/private/tmp/wisp-research-pages-20260928/{native,webview}` 隔离合成数据。原生包由当前工作树构建；WebView 对照包沿用 `9ea5f411` 基线，本轮没有修改 WebView UI。

| 要求 | 已核对的结果 | 证据 |
| --- | --- | --- |
| 按日、会话组织 | 9 月 25/26/28 日分开；25 日 35 次同会话活动合并为一条，其他两个会话与两个产物版本分别保留；WebView 同日也是 3 会话、2 产物 | native-journey-month.jpg、native-journey-grouped.jpg、webview-journey.jpg |
| 准确来源入口 | HTTP 条目打开 HTTP 富文本对比会话；report.md 打开版本 1 及对应版本 ID，显示会话、运行/输入信息与只读内容 | native-journey-session-source.jpg、native-journey-artifact.jpg；Rust 不可变版本和跨项目拒绝测试 |
| 导航与 Escape | 第一次 Escape 仅关闭产物详情，第二次关闭历程并保留原 35 轮会话；最终包验证研究页收起/展开侧栏、文件入口退出研究页且保留会话 | NativeJourneyTests、NativeCalendarTests、NativePublicationTests；实机检查 |
| 日期与活动标记 | 周一表头、9 月 1 日在周二、今天 28 日；日号与活动计数分开；多项目同日、筛选项目、空八月、今天跳转、当日历程入口均核对 | native-calendar.jpg、native-calendar-narrow.jpg、webview-calendar.jpg；日期/DST/闰年/跨年测试 |
| 论文/修订与结构 | qa-paper-a 的 qa-rev-a1/v2 分别 1/2 条证据，章节/论点层级与正文一致；切回 v2 清空旧选中详情 | native-publication-v1.jpg、native-publication.jpg、publication-contract-evidence.json |
| 证据来源及空态 | v1 选中 qa-rev-a1-binding-0，对应 counts.csv 的版本 ID；切换 qa-paper-b 后结构与证据均为 0，无上篇论文遗留 | native-publication-v1.txt、native-publication-empty.jpg；共享 workspace-evidence.json 往返测试 |
| 宽窄布局 | 历程、论文、日历均有实机宽窄截图；窄窗论文结构在证据上方，日历折叠项目区，无横向溢出 | native-*-narrow.jpg；render-*.png 为真实 SwiftUI 组件的离线布局补充证据 |
| 加载/失败/截断 | 读取时显示进度；论文切换先清空详情，错项目/修订及迟到响应拒绝；历程空范围/错误/2000 条截断分开；日历部分项目失败与截断分开 | NativePublicationTests、NativeJourneyTests、NativeCalendarTests；三页视图分别呈现对应状态 |

`publication-contract-evidence.json` 来自对同一隔离 QA 宿主的只读请求，记录 paper/revision/item/binding/source ID；没有保存宿主认证信息。原生与 WebView 对照均使用同 ID 的 fixture。新 `workspace-evidence.json` 同时供 Rust DTO 和 SwiftUI 离线渲染使用，旧 fixture 保留向后兼容覆盖。

## 自动验证

日志保存在 `target/research-pages-20260928/`（不提交构建产物）。

- `cargo fmt --all -- --check`、`python3 scripts/sync_native_design.py --check`、`git diff --check` 通过。
- `WISP_CATALOG_OFFLINE=1 cargo test --workspace`：2,382 项通过、0 失败；最后补充的 DTO 合同测试另行 `cargo test -p wisp-dto native_publication`，2 项通过。
- Swift 全套：321 项执行，302 通过、19 个可选截图测试未启用，0 失败。研究页面定向启用截图运行 29 项通过（包含三页的宽窄真实视图渲染）。
- 原生产物版本查询 Rust 定向 3 项通过；原生论文查询定向 2 项通过。
- `cd ui && cargo check --target wasm32-unknown-unknown` 通过。
- `cd ui-tests && npm ci && npx playwright test`：880 通过、2 跳过、1 失败。失败在 compacted-history 初始化等待 open-session 监听器时超时，尚未执行分页断言；随后整个 `tests/compacted-history.spec.ts` 定向重跑 4 项通过。没有将首次全量运行记为全绿，也未修改无关测试或放宽超时。
- `WISP_CATALOG_OFFLINE=1 bash scripts/build_native_macos.sh --qa` 成功，输出 `target/native-macos-qa/Wisp Science QA.app`。最终导航代码经过实机验证；最后补充受限可见性中文标签后再次构建和 Swift 全套检查。

## 手工复现

1. 在含多天活动、同会话多条消息和两个产物版本的项目打开研究历程；核对日期/星期/会话计数，进入指定会话并核对标题。查看产物版本来源，连续按两次 Escape，核对关闭顺序与原会话保留。
2. 首页打开研究日历，检查 2026 年 9 月第一天在周二；切换八月、今天、项目筛选，进入 25 日的对应项目历程。缩窄窗口，展开/折叠项目筛选。
3. 论文证据选择 qa-paper-a，切换 qa-rev-a1 / qa-rev-a2，分别核对 1 / 2 条证据和选中项来源 ID；切换 qa-paper-b 核对空态。缩窄窗口，检查结构、证据、来源快照可滚动读取。

## 限制与后续

- 本阶段只读：不提供关系图编辑、补充记录、添加证据、版本写入、冻结或重现；定稿检查和版本记录查询按 G 第二阶段处理。
- 历程最多展示 2,000 条活动并提示按日查询；产物内容预览上限 2 MiB，文本/图片支持内联预览，其他二进制仅展示元数据，缺失文件单独报错且保留来源信息。
- WebView 基线从 v2 切到 v1 后证据数更新为 1，但选择器仍显示 v2。这是参考版本自身的已知现象，原生已验证选择器与内容同时切换，未复制该问题。
- 新页面的状态标签已中文化；没有声称完成整应用中英文界面对齐。
- 验收中辅助功能查询曾超时，重置 Computer Use 会话后恢复；没有将其认定为已修复的应用缺陷。

实机截图为 Computer Use 原始 JPEG 字节；render-*.png 是 opt-in NSHostingView 渲染。未发送模型消息或访问真实远程算力。验收进程已停止，两处 QA support 软链接已恢复原值；用户数据库未修改。
