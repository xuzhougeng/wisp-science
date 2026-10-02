# 首批原生阅读对齐与输入区能力契约

日期：2026-09-28。范围来自 [下一阶段计划](../plans/2026-09-28-swiftui-webview-next-ui-alignment.md) 的 A、B1、B2、D1；C 在本轮只核对接口，不新增输入区模式或执行动作。

## 阅读与公式方案

保留 `NativeSelectableMessage` 的单一 AppKit 文档，标题、引用、代码和表格仍可跨块选择。代码高亮通过系统 JavaScriptCore 执行仓库已有的 Highlight.js 11.9.0，只取 token 范围并设置原生字色，不创建网页，不执行消息内代码。未知语言和过大的代码块保留普通等宽文本。语法库由 `sync_native_design.py` 同步；BSD-3-Clause 许可随资源打包。

公式采用固定版本 [SwiftMath 1.7.3](https://github.com/mgriebling/SwiftMath/tree/1.7.3)，MIT 许可，字体与许可随其资源包离线分发。它提供原生 LaTeX 排版；本项目将排版结果作为矢量文本附件绘制，保留同一原生文本选区，不生成截图或依赖远程渲染服务。首次安装构建依赖可能需要网络，运行和测试无需网络。

支持 `$…$` / `\(…\)` 行内公式、`$$…$$` / `\[…\]` 块级公式。预处理避开 fenced/indented/inline code，避免 Markdown 消耗 LaTeX 转义；上下标与分式沿字体基线排版。消息和产物预览使用同一渲染器。所有公式保留原始 LaTeX：整段复制和划线/侧聊引用恢复带定界符的源码，公式复制动作复制表达式，辅助功能值不暴露内部占位符。任务列表显示不可交互的原生复选框，复制保留 `[x]` / `[ ]`。

非法、未完成、当前排版器不支持或宽于容器的公式显示可换行源码。宽公式可在更宽的产物预览中重新排版；不缩成难以阅读的小图，不声称与 KaTeX 所有宏完全兼容。超长输入/过深分组也走源码回退。引擎与解析器的边界测试覆盖分式缺项、代码中的美元符号、货币、Unicode、深浅主题和窄列。

消息动作是复制、侧聊引用、收藏，均只使用当前消息内容，不触发发送。消息时间使用持久化对话大纲的 `sent_at` / `response_at`，按 `user_offset` 与所属用户轮次关联，与 WebView 的规则一致；缺少真实时间就不显示。工具耗时和模型名称沿用宿主已有字段。协议只追加可选字段，旧快照保持可解码。

## 设置与面板行为

自动继续改为明确的“输出 token 截断”语义；关闭时仍展示禁用的次数上限。补充切换模型语义压缩及空闲提示小时数。最大迭代/空闲小时接受 0，上限要求至少 1；非法、空白、负数和溢出输入在宿主请求前拒绝并保留草稿。未知设置字段随整份文档回传，不丢失。原有保存、取消和离开未保存提示继续生效。

通用/对话/桌宠/同步共用 `get_settings` 草稿，保存会提交这些页的全部设置草稿；通用页还保存发送/划选偏好。网络另存。更新开关立即保存；自动审核立即保存且只设置新会话默认值。界面明确这些边界，不将全局设置标成项目设置。同步选择器按实际保存值选标签，未知值原样显示，不回退到首项。

产物按类型分组并显示计数，消息表格显示数据行数（不含表头）和列数。注册文件属于当前会话；表格/公式投影只代表当前显示的消息页，历史分页不冒充全会话总量。筛选无结果、无产物、加载和错误状态分开呈现。原生面板宽度默认 380 pt，并根据窗口剩余宽度限制拖拽结果，保留输入区空间。窗口不足 960 pt 且右面板打开时暂时收起侧栏，重新展开侧栏会关闭右面板；列表/网格及标签偏好维持现有客户端级持久化，面板宽度只在当前工作区视图内保留。切换会话以项目/会话 ID 重建面板，翻页关闭消息产物预览。

Agents 增加说明卡和委派关闭状态卡，保留现有审批逻辑；开启委派不代表自动批准。读取失败不同时显示“暂无工作流”。

## 输入区接口核查

已对照 `ui/src/main.rs`、`ui/src/app_support/{composer,runtime}.rs`、`NativeConversationModel.swift`、`NativeMessageInput.swift`、`src-tauri/src/native_{conversations,panels}.rs` 和 `wisp-dto/src/native_conversations.rs`。不能把普通 Tauri 命令在原生宿主可调用性当作默认成立：原生请求有显式命令白名单和项目/会话范围检查。

| 能力 | WebView / 已有后端 | 原生可用接口与当前约束 | 后续接入结论 |
| --- | --- | --- | --- |
| Local 与附加环境 | `list_execution_contexts`、`list_session_execution_context_ids`、`set_session_execution_context_enabled` | `native_conversation_panel_contexts` 返回 contexts/enabled_ids/read_only；`…_context_enabled` 写会话绑定。Local 始终可用，不能取消 | 可先复用环境面板读状态、跳转和附加入口；不要把“附加”当成“默认执行” |
| 默认执行环境 | 全局 `get/set_default_execution_context` 与会话 `get/set_session_default_execution_context` | 设置已有全局默认入口；原生 PanelContexts 不含会话默认值，也没有对应会话写命令 | 接入前补独立会话默认查询/写入 DTO，并保持继承语义 |
| Python / R | `list_runtimes`、`start/stop/restart_runtime`、`inspect_runtime`、`execute_runtime` | `…_panel_activity`、`…_runtime_start/stop/restart/dismiss/inspect/execute` 已有，参数含 session/context/language/runtime generation；写操作校验只读状态 | 可展示未启动/starting/ready/busy/dead/unavailable 并跳转现有运行时面板。查看状态不得顺便启动运行时 |
| Agent / Plan | 会话 `set_session_plan_mode`，未建会话时前端 pending；ACP 有自己的 session mode | 原生 snapshot/send 无 plan_mode 字段；Agents 的 delegation 开关只是任务委派，不是 Plan 模式 | 必须新增范围受限的模式协议及快照状态；不可用委派开关代替 |
| Fast | `get/set_session_service_tier`；profile 能力决定入口，null 继承、空字符串明确关闭、priority 开启；运行中禁用 | 原生没有 session service tier 查询/写入命令，也不在 send 请求中 | 需要可空 override 的准确协议和支持能力判定，不能只按模型名画开关 |
| 模型 | `list_models`、`set_session_model`，ACP 单独会话绑定 | `list_models`、`native_conversation_model` 与 snapshot.model_id/acp_agent_id 已有；原生禁用 ACP 会话的普通模型切换 | 沿用现有接口，切换后重新确认能力、作用域与草稿，不迁移 ACP 会话 |
| `@` 文件/产物/计算对象 | composer picker 支持 FilePath、Artifact、Context、Runtime 等结构化引用 | 原生可读文件/产物/环境，但 SendRequest 只有 message 与 attachments，没有引用类型/来源元数据 | 需定义结构化引用协议与读取范围；不能只把 `@name` 拼成文本宣称支持 |
| `#` 对话/项目 | `search_sessions` 和项目列表；reference chip 带稳定 ID 与项目名称 | 原生普通会话快照不是搜索接口；inbox 是待查看列表，不覆盖全局搜索；send 无 Session/Project reference | 新增搜索/引用合同、跨项目边界和失效目标处理 |
| `/` 技能/工作流/内置命令 | picker 的 Skill/Workflow/Command；`parse_slash_command` 区分命令与路径，部分填草稿、部分打开界面 | 原生设置能列技能/工作流，部分分享/轨迹已有动作；输入框不识别 slash，发送接口没有 skill/workflow 引用 | 先按单个命令列映射，具备实际能力才展示。技能与 workflow 引用另补协议 |
| 附件 | 上传后绑定工作区资源，发送携带稳定路径 | `native_conversation_attach` 拷贝并绑定，send/enqueue 的 attachments 只接收已上传项目相对路径；取消从当前会话草稿移除 | 已接入；不得将本机绝对路径或取消选择当作已发送附件 |
| 发送与停止 | 普通发送、分支、GuideAppend、InterruptReplace 等 | 原生 `send` 有 request_id 防重复；`enqueue` 仅一个待处理后续；`stop` 已有；不支持全部 WebView 引导/分支动作 | 保持现有正常发送、停止与排队；缺失策略按独立命令接入 |
| 草稿和快捷键 | 会话草稿、附件、配置发送方式、IME | 原生按会话保存 draft/stagedFiles；NativeMessageInput 保留 marked text；Return 由当前编辑器处理 | 本轮不改写输入策略。后续必须覆盖切会话、取消附件、断连与真实 IME |

第一项后续输入区实现应只呈现环境与运行时真实状态及现有面板入口；Agent/Plan、Fast、结构化引用必须在协议具备后分开实现。运行、只读、断连、审批和 ACP 的限制分别保留。

完整记录/模型视角也已核查：WebView 用 `load_session_context_view` 获取模型工作集；原生 `SessionRequest` 只有 session_id/before_seq，历史分页不是模型视角。本轮不显示无效切换按钮，后续需扩展独立的只读查询与来源标识。
