const WISP_PAGES_I18N = {
  zh: {
    "meta.home.title": "Wisp Science | 开源科学计算 Agent",
    "meta.home.desc":
      "Wisp Science — 开源、本地优先的 Windows / macOS / Linux 科研 Agent 工作台。对接任意 LLM，运行 Python / R，调用 80+ 科研数据库与 34 个内置 SKILL。",
    "meta.models.title": "模型配置 | Wisp Science",
    "meta.models.desc": "Wisp Science 模型配置说明：OpenAI 兼容、OpenAI Responses 和 Anthropic API。",
    "meta.acp.title": "ACP Agent 配置 | Wisp Science",
    "meta.acp.desc":
      "Wisp Science ACP Agent 配置说明：在 Settings → Models → ACP Agents 下接入 Codex / Claude 等本地 ACP v1 agent。",
    "nav.aria": "页面导航",
    "nav.features": "功能",
    "nav.usecases": "场景",
    "nav.stack": "生态",
    "nav.models": "模型配置",
    "nav.acp": "ACP",
    "nav.faq": "FAQ",
    "nav.docs": "文档",
    "nav.downloadFull": "下载桌面版",
    "nav.downloadShort": "下载",
    "lang.aria": "语言",
    "hero.eyebrow": "开源 · 本地优先 · Windows / macOS / Linux · v1.5.0",
    "hero.title": "严谨科研的<br>本地 Agent 工作台",
    "hero.lead":
      "Wisp Science 在本地运行分析、检索数据库、调用 Python / R 与 MCP 工具，从数据整理到报告输出全程可追溯——把时间留给科学本身。",
    "hero.download": "下载桌面安装包",
    "hero.source": "从源码构建",
    "hero.mockUser": "检索 PubMed 上 CRISPR 筛选的最新方法，并画一张流程图。",
    "hero.mockAssistant":
      "已调用 mcp_pubmed 检索 12 篇文献，生成 Markdown 报告与 Python 绘图代码。表格与公式已提取为 artifact。",
    "trusted.heading": "Wisp Science 用户来自高校与科研团队",
    "trusted.aria": "Wisp Science 用户所在团队",
    "voices.kicker": "From real research questions",
    "voices.heading": "他们这样用 Wisp Science",
    "voices.lead": "不是一句“帮我分析”，而是把手头真实、具体的科研任务交给它继续推进。",
    "voices.q1":
      "“能不能先帮我把这批单细胞数据做完 QC，再看看这几个 cluster 到底是什么细胞？每一步的参数和图都留下来，我明天组会要讲。”",
    "voices.a1":
      "以前最怕中途改参数，现在可以沿着同一段对话接着跑，图、代码和判断也都找得到。",
    "voices.role1": "生物信息方向博士生",
    "voices.q2":
      "“把最近五年和这个靶点有关的临床前研究梳理一下。别只给结论，把检索来源、相互矛盾的结果和还缺什么证据一起列出来。”",
    "voices.a2": "它更像一个愿意把依据摊开的研究搭档，而不是给我一段看起来很确定的摘要。",
    "voices.role2": "药物研发团队研究员",
    "voices.q3":
      "“这个蛋白的几个突变位点可能影响稳定性吗？先查数据库和文献，再给我一个能在本地复现的分析方案，不要动原始数据。”",
    "voices.a3": "数据留在电脑上很重要；更省心的是检索、分析和最后的 Methods 草稿能在一个项目里串起来。",
    "voices.role3": "结构生物学研究者",
    "voices.q4":
      "“我把 20 篇 PDF 放进项目了。能按实验体系整理它们各自用了什么对照吗？有数字对不上的地方先标出来，别替作者猜。”",
    "voices.a4": "做文献表格时最有用的不是快，而是它会把出处带回来，我能立刻核对原文。",
    "voices.role4": "分子生物学博士后",
    "voices.q5":
      "“这份代谢组结果先别急着解释。帮我检查缺失值、批次效应和异常样本，再把你建议的统计步骤写成一份可重复运行的脚本。”",
    "voices.a5": "它不会只扔给我一张漂亮图，前面的数据检查和后面的脚本都在，交接给同事也方便。",
    "voices.role5": "转化医学实验室成员",
    "voices.q6":
      "“根据这组候选分子查一下 ChEMBL 和 PubChem，活性、选择性和已知风险分开列。最后告诉我下一轮最值得补哪三个实验。”",
    "voices.a6": "数据库检索和本地分析不用来回切工具，尤其适合先把问题收敛，再决定昂贵的实验往哪做。",
    "voices.role6": "计算化学研究员",
    "features.kicker": "Built for scientific research",
    "features.heading": "为科研而生",
    "features.lead":
      "独立开源的 Rust/Tauri 科研工作台：Agent 核心、生物工具链与桌面 UI 全部本地运行，数据不出机器。",
    "features.artTitle": "科学产物，完整可追溯",
    "features.artBody":
      "对话中的表格、代码块、LaTeX 公式与文件路径自动提取为 artifact，侧边栏即时预览。Markdown 渲染支持 KaTeX 与代码高亮。",
    "features.artPre":
      "{{artifact:abc123}}\n→ Table · 8 rows × 4 cols\n→ figure_umap.py · 42 lines\n→ $\\hat{\\beta}$ · LaTeX equation\n\n每个 artifact 绑定生成它的对话上下文，\n便于复现、编辑与版本回溯。",
    "features.pyTitle": "持久 Python / R 内核",
    "features.pyBody":
      "长驻内核子进程保持变量与 DataFrame 在内存中，且按会话隔离。跨轮次迭代分析无需重复加载数据，并行会话也不会互相覆盖状态。",
    "features.pyPre":
      ">>> import scanpy as sc\n>>> adata = sc.read_h5ad(\"pbmc.h5ad\")\n>>> adata.shape\n(2700, 32738)\n\n# 变量在后续 tool call 中仍然可用\n>>> sc.pp.neighbors(adata)\n>>> sc.tl.umap(adata)",
    "features.skillTitle": "34 个内置 SKILL 工作流",
    "features.skillBody": "从文献综述、分析模块、作图、Python/R 环境到远程 SSH 计算，开箱即用。",
    "features.skillPre":
      "skills/\n├─ literature-review/\n├─ analysis-workflow/\n├─ remote-compute-ssh/\n├─ figure-composer/\n└─ paper-narrative/ …\n\nAgent 通过 use_skill 工具按需加载 SKILL.md",
    "features.llmTitle": "任意 LLM 后端",
    "features.llmBody":
      "支持 OpenAI 兼容、OpenAI Responses、Anthropic API，以及本地 <a href=\"acp-agents.html\">ACP Agent</a>。<a href=\"model-configuration.html\">查看 HTTP 模型配置</a>。",
    "features.ctxTitle": "三层上下文压缩",
    "features.ctxBody":
      "完整历史先归档，安全裁剪 tool/图片噪声；必须移除语义轮次时，先基于原历史生成增量摘要检查点，并保留有界近期上下文。",
    "features.sqlTitle": "SQLite 持久化",
    "features.sqlBody": "项目、会话帧、消息与 artifact 写入本地数据库，重启应用完整恢复历史。",
    "local.kicker": "Works wherever your data lives",
    "local.heading": "数据留在本地",
    "local.lead":
      "Windows（WebView2）、macOS（Apple Silicon / Intel 分架构）与 Linux 桌面，或 headless CLI；原生生物工具无需 Python；可选运行环境可在启动后按需配置。",
    "local.pkgTitle": "自包含安装包",
    "local.pkgBody":
      "skills、Python、MCP 与 demo 会话打包进 Windows MSI/NSIS、macOS dmg 与 Linux .deb / AppImage，无需源码树。",
    "local.mcpTitle": "统一 bio MCP",
    "local.mcpBody": "默认启动 mcp_bio，约 240 个工具覆盖 PubMed、UniProt、ChEMBL 等 80+ 数据库。",
    "local.remoteTitle": "远程计算",
    "local.remoteBody": "SKILL 支持 SSH、WSL 与 GPU 主机；Agent 可编写并提交带实时日志的长 Run。",
    "usecases.kicker": "How researchers use Wisp Science",
    "usecases.heading": "研究者如何使用",
    "usecases.lead": "预置生命科学多领域工作流；跨学科项目可在同一会话中串联文献、分析与可视化。",
    "usecases.tabScrna": "单细胞 RNA-seq",
    "usecases.tabProtein": "蛋白结构",
    "usecases.tabChem": "化学信息学",
    "usecases.tabLit": "文献与写作",
    "usecases.scrnaTitle": "单细胞与转录组分析",
    "usecases.scrnaBody":
      "用 analysis-workflow 把 QC、聚类、注释收成可复现模块；在持久 Python 内核中完成 Scanpy 流程与 UMAP 可视化。",
    "usecases.scrnaLi1": "调用 MCP 查询 GEO / CellxGene 元数据",
    "usecases.scrnaLi2": "表格与图形自动进入 artifact 面板",
    "usecases.scrnaLi3": "代码与对话绑定，便于复现",
    "usecases.promptLabel": "示例提示",
    "usecases.scrnaPrompt": "对 PBMC 3k 数据做标准 Scanpy 流程，标注 major cell types，并导出 marker 基因热图。",
    "usecases.proteinTitle": "蛋白结构与序列",
    "usecases.proteinBody":
      "从 UniProt / PDB 拉取序列与实验结构，结合文献综述与作图，生成可引用的位点表和 Methods 草稿。",
    "usecases.proteinLi1": "从 UniProt / PDB MCP 拉取序列与结构",
    "usecases.proteinLi2": "3D 结构查看器在 Roadmap 中",
    "usecases.proteinLi3": "与文献综述 SKILL 串联假设生成",
    "usecases.proteinPrompt": "获取 TP53 预测结构，叠加 ClinVar 致病突变位点，并生成 Methods 段落草稿。",
    "usecases.chemTitle": "化学信息学与分子设计",
    "usecases.chemBody":
      "通过 ChEMBL、PubChem MCP 检索活性数据，计算理化性质与相似性，用 figure-composer 出 SAR 图。",
    "usecases.chemLi1": "计算理化性质与相似性",
    "usecases.chemLi2": "右侧面板预览 SMILES / MOL / SDF（RDKit）",
    "usecases.chemLi3": "figure-composer 统一出图风格",
    "usecases.chemPrompt": "检索靶点 EGFR 的 IC50 数据，筛选 oral bioavailability 较好的候选，并绘制 SAR 热力图。",
    "usecases.litTitle": "文献检索与稿件草稿",
    "usecases.litBody": "literature-review、paper-narrative、pdf-explore SKILL 帮助从 PDF 到结构化综述与叙事。",
    "usecases.litLi1": "PubMed / Semantic Scholar MCP 检索",
    "usecases.litLi2": "Markdown + LaTeX 实时预览",
    "usecases.litLi3": "indication-dossier 适应症档案模板",
    "usecases.litPrompt": "基于附件 PDF 写一段 Discussion，附引用列表，并检查数字是否与表格一致。",
    "demo.kicker": "See it in action",
    "demo.heading": "内置 Demo 会话",
    "demo.lead":
      "桌面应用「Open demo」可打开只读示例：CRISPR 筛选、酶工程、极端微生物与免疫治疗——无需 API 额度即可感受工作流。",
    "demo.crisprTitle": "CRISPR Screen",
    "demo.crisprBody": "从 raw counts 到 hit calling 与可视化报告的完整 Agent 轨迹。",
    "demo.enzymeTitle": "Enzyme Engineering",
    "demo.enzymeBody": "序列分析、突变建议与活性预测的多轮工具调用示例。",
    "demo.extTitle": "Extremophile",
    "demo.extBody": "宏基因组检索与功能注释的跨数据库工作流。",
    "demo.immTitle": "Immunotherapy",
    "demo.immBody": "肿瘤免疫相关文献整合与假设生成演示。",
    "stack.kicker": "Works with your stack",
    "stack.heading": "对接你的工具链",
    "stack.lead": "MCP 协议连接生物数据库与自定义服务器；SKILL.md 扩展可复用流水线。Agent 把它们当作一等公民工具调用。",
    "stack.skill": "34 bundled workflows",
    "stack.python": "uv-managed venv · isolated R",
    "stack.browse": "浏览 MCP 服务器",
    "faq.heading": "常见问题",
    "faq.q1": "Wisp Science 是新模型吗？",
    "faq.a1":
      "不是。Wisp Science 是开源桌面/CLI 应用，使用你自备 API Key 对接的任意兼容 LLM。新的是围绕模型的 Agent 循环、工具、MCP 与 Python / R 内核。",
    "faq.q2": "与通用 AI 助手有何不同？",
    "faq.a2":
      "它能真正执行：读写本地文件、运行 Shell、调用持久 Python / R REPL、通过 MCP 查询 PubMed/UniProt 等数据库，并在 SQLite 中保存完整会话。内置 34 个领域 SKILL，而非仅生成文本。",
    "faq.q3": "研究数据是否私密？",
    "faq.a3":
      "原始数据与计算在本地进行；会话与 artifact 存于本机 SQLite。发送至 LLM 提供商的仅为 prompt 与 model 响应，遵循你所用 API 的隐私政策。",
    "faq.q4": "支持哪些平台？",
    "faq.a4":
      "Windows（已签名 MSI/NSIS + WebView2）、macOS（Apple Silicon / Intel 分架构 dmg，已签名并公证）与 Linux（.deb / AppImage，x86_64 与 aarch64）均提供安装包，随 GitHub Release 一同发布。macOS 双击即可打开；Windows 如仍触发 SmartScreen，选择「仍要运行」。",
    "faq.q5": "macOS 为什么反复弹出钥匙串密码框？",
    "faq.a5":
      "API Key 存放在 macOS 登录钥匙串，条目会绑定写入时应用的代码签名身份。若你先用过未签名的 v0.4.x 填过 Key，那个条目被绑到了旧身份；升级到已签名版本后身份改变，系统便反复要求输入登录密码来重新授权。解决办法：打开「钥匙串访问」，搜索并删除名为 <code>wisp</code> 的条目，重开 wisp 重新填一次 API Key（点一次「始终允许」）即可。",
    "faq.q6": "需要什么前置依赖？",
    "faq.a6":
      "安装包用户：Windows 需 WebView2（Win10/11 通常已带），macOS 用系统 WebKit，Linux 用 WebKitGTK；都需自备 API Key。可选安装 R（需 jsonlite）以使用持久 r 工具。从源码构建需 Rust、Trunk、Tauri CLI v2（macOS 另需 Xcode 命令行工具）。首次配置时会后台检测环境路径并保存到 Local；缺少工具不影响启动，也不会自动安装。需要时可在启动后通过 local-env-setup 配置。",
    "faq.q7": "Wisp Science 是独立项目吗？",
    "faq.a7":
      "是的。我们最初关注过 Claude Science 一类封闭产品，但发现其对部分地区用户不友好、且生态封闭。Wisp Science 由此起步：学习其 Skills 与 MCP 工具选型思路，并以 Rust/Tauri 独立实现本地优先的科研工作台（AGPL-3.0）。Agent 架构、多项目工作流、Run 管理、插件体系、ACP 等核心能力均为自主设计；可对接任意模型提供商，任何人都可使用、研究与改进。",
    "faq.q8": "当前版本稳定吗？",
    "faq.a8":
      "v1.5.0 是面向本地科研工作流的活跃预览版。核心 Agent、流式、工具、Python / R、MCP、分享导出与桌面 UI 可运行；关键方法与输出仍应人工复核，并以 Release 说明确认当前签名和更新状态。",
    "footer.copy": "© 2026 Wisp Science · 开源科学计算 Agent",
    "models.lead":
      "Wisp Science 的桌面版使用模型 profile 管理远程 API 后端。每个 profile 可以配置 provider、API URL、模型 ID、高级参数和独立 API key。",
    "models.download": "下载最新版",
    "models.acpCta": "ACP Agent 配置",
    "models.md": "查看 Markdown 文档",
    "models.compatBody": "DeepSeek、GLM、本地网关，或任何兼容 /chat/completions 的服务。需要 API URL、Model ID 和 API key。",
    "models.respBody": "通过 /v1/responses 使用 OpenAI reasoning / tool-call 模型。需要 API URL、Model ID 和 API key。",
    "models.anthBody": "通过 /v1/messages 使用 Claude API。需要 API URL、Model ID 和 API key。",
    "models.keyBody":
      "API key 存入系统 keyring；profile 名称、provider、URL、模型 ID 和能力标注存入项目的 .wisp/wisp.sqlite。",
    "models.fieldsHeading": "常用字段",
    "models.fieldsLead": "桌面版在 Settings -> Models 中配置。多个 profile 可以并存，随时切换 active profile。",
    "models.thField": "字段",
    "models.thBackend": "适用后端",
    "models.thHelp": "说明",
    "models.all": "全部",
    "models.displayHelp": "在 UI 中显示的 profile 名称。",
    "models.providerHelp": "选择 OpenAI-compatible、OpenAI Responses 或 Anthropic。",
    "models.modelHelp": "远程 API 的模型名。",
    "models.urlHelp":
      "远程 API 的 base URL 和密钥。无需追加 /v1、/chat/completions、/responses 或 /v1/messages；Wisp 会根据 provider 自动补全请求路径，并为 OpenAI 兼容服务探测常见路径。桌面版密钥不写入 SQLite。",
    "models.cliLead": "wisp-science headless CLI 使用环境变量配置远程 API provider。",
    "acp.lead":
      "Wisp 可作为 ACP Client，启动本机已安装的 Codex / Claude 等 ACP v1 Agent。它与 HTTP 模型 profile 分开：Models 里的「添加模型」管 API，「ACP Agents」管本地 stdio agent 进程。",
    "acp.start": "开始配置",
    "acp.httpCta": "HTTP 模型配置",
    "acp.md": "Markdown 文档",
    "acp.splitHeading": "两套后端，不要混用",
    "acp.splitLead": "Settings → Models 用两个分类切换：Models（HTTP API）与 ACP Agents（本地进程）。",
    "acp.httpTitle": "HTTP 模型",
    "acp.httpBody": "Settings → Models → 添加模型。DeepSeek / OpenAI / Anthropic 等远程 API，走内置 Wisp Agent。",
    "acp.agentTitle": "ACP Agent",
    "acp.agentBody": "Settings → Models → ACP Agents。本地 stdio 进程，Agent 自带会话、工具与鉴权。",
    "acp.cliTitle": "不要填裸 CLI",
    "acp.cliBody": "不要把 codex / claude 直接填进 ACP。应使用官方 ACP 适配器。",
    "acp.credTitle": "凭证不进 SQLite",
    "acp.credBody": "登录与 API key 由外部 Agent 自己管理；Wisp 只保存启动命令与参数。",
    "acp.whereHeading": "入口在哪里",
    "acp.whereLead": "ACP 与「添加模型」同级，都在 Models 层级下。",
    "acp.step1Body": "打开设置侧边栏的 Models，用顶部分类标签切换 Models / ACP Agents。",
    "acp.step2Title": "2. Add / Edit 同一套",
    "acp.step2Body": "右侧 Add model 或 Add ACP Agent 进入面包屑子页表单；点列表行即可编辑。",
    "acp.step3Title": "3. 聊天框快捷入口",
    "acp.step3Body": "模型菜单底部的 Add model / Add ACP Agent 打开同一套表单。",
    "acp.fillHeading": "填写与启用",
    "acp.fillLead": "先装好适配器，再在 Wisp 里保存、测试、选中。",
    "acp.thValue": "值",
    "acp.labelHelp": "显示名，例如 Codex ACP",
    "acp.cmdHelp": "只填可执行文件，不要把参数拼进这一行",
    "acp.argsHelp": "每个参数单独一行（Windows 用 npx.cmd 时尤需注意）",
    "acp.testBody": "成功表示进程能启动并完成 ACP initialize。",
    "acp.authTitle": "Authenticate（如有）",
    "acp.authBody": "测试后若出现登录按钮，按 Agent 提示完成；凭证不写入 Wisp。",
    "acp.emptyTitle": "空会话中选用",
    "acp.emptyBody": "新建空会话，在模型菜单选该 ACP Agent，再发消息。首条消息后锁定，换后端请开新会话。",
    "acp.exHeading": "示例：Codex / Claude",
    "acp.exLead": "推荐先全局安装；也可用 npx，参数必须分行。",
    "acp.orNpx": "或用 npx（Windows 建议 Command 填 npx.cmd）：",
    "acp.empty": "（空）",
    "acp.orPath": "（或绝对路径）",
    "acp.repos": "适配器仓库：",
    "acp.notesHeading": "使用中注意",
    "acp.permTitle": "权限与配置",
    "acp.permBody": "权限卡显示 Agent 返回的选项；会话级 model/mode 默认收进发送按钮旁的 ACP 模型菜单。",
    "acp.sciTitle": "科学工具",
    "acp.sciBody": "Wisp 会注入 MCP bridge，外部 Agent 可在项目目录内调用内置科学工具。",
    "acp.limitTitle": "当前限制",
    "acp.limitBody": "仅本地 stdio；无远程 / WSL / SSH ACP；无应用内安装市场；编辑 Command/Arguments 后需新开会话。",
    "acp.troubleHeading": "排错",
    "acp.thSymptom": "现象",
    "acp.thFix": "处理",
    "acp.failHelp": "Command 不在 PATH、Windows 应用 npx.cmd，或参数误写进 Command",
    "acp.authFail": "先在系统里完成 Codex / Claude 登录或 API key，再测",
    "acp.locked": "预期行为；换后端请新建空会话",
    "acp.pathChanged": "改过启动参数或项目路径后需新开会话",
  },
  en: {
    "meta.home.title": "Wisp Science | Open-source scientific computing agent",
    "meta.home.desc":
      "Wisp Science — an open-source, local-first scientific agent workbench for Windows, macOS, and Linux. Bring your own LLM, run Python / R, and call 80+ scientific databases plus 34 bundled SKILLs.",
    "meta.models.title": "Model configuration | Wisp Science",
    "meta.models.desc":
      "Wisp Science model setup: OpenAI-compatible, OpenAI Responses, and Anthropic APIs.",
    "meta.acp.title": "ACP agent setup | Wisp Science",
    "meta.acp.desc":
      "Connect local ACP v1 agents such as Codex and Claude under Settings → Models → ACP Agents.",
    "nav.aria": "Page navigation",
    "nav.features": "Features",
    "nav.usecases": "Use cases",
    "nav.stack": "Stack",
    "nav.models": "Models",
    "nav.acp": "ACP",
    "nav.faq": "FAQ",
    "nav.docs": "Docs",
    "nav.downloadFull": "Download desktop",
    "nav.downloadShort": "Download",
    "lang.aria": "Language",
    "hero.eyebrow": "Open source · Local-first · Windows / macOS / Linux · v1.5.0",
    "hero.title": "A local agent workbench<br>for rigorous research",
    "hero.lead":
      "Wisp Science runs analysis locally, queries scientific databases, and calls Python / R and MCP tools. From data wrangling to the report, the trail stays in one project—so you can spend the time on the science.",
    "hero.download": "Download the desktop app",
    "hero.source": "Build from source",
    "hero.mockUser": "Search PubMed for recent CRISPR screen methods and draft a flowchart.",
    "hero.mockAssistant":
      "Called mcp_pubmed on 12 papers and drafted a Markdown report plus Python plotting code. Tables and equations are already extracted as artifacts.",
    "trusted.heading": "Used by researchers from universities and research teams",
    "trusted.aria": "Organizations using Wisp Science",
    "voices.kicker": "From real research questions",
    "voices.heading": "How researchers move work forward",
    "voices.lead":
      "Not a vague “analyze this” prompt, but a concrete research task advanced with evidence, code, and a traceable record.",
    "voices.q1":
      "“Start by QCing this single-cell dataset, then help me identify those clusters. Keep every parameter, figure, and rationale—I need to walk through it at lab meeting tomorrow.”",
    "voices.a1": "Keeps QC parameters, plots, code, and annotation decisions together for review.",
    "voices.role1": "PhD student in bioinformatics",
    "voices.q2":
      "“Map the preclinical studies on this target from the past five years. Don’t just give me a conclusion—show the sources, conflicting findings, and evidence that is still missing.”",
    "voices.a2": "Builds an auditable evidence map instead of a confident-looking summary.",
    "voices.role2": "Drug discovery researcher",
    "voices.q3":
      "“Could these mutations affect protein stability? Check the databases and literature first, then give me a locally reproducible analysis plan without moving the raw data off my computer.”",
    "voices.a3": "Connects database evidence and literature to a reproducible local analysis.",
    "voices.role3": "Structural biology researcher",
    "voices.q4":
      "“I added 20 papers to the project. Organize the controls by experimental system, flag inconsistent numbers, and make every finding traceable to the original text.”",
    "voices.a4": "Compares experimental designs across papers while preserving source-level traceability.",
    "voices.role4": "Postdoctoral researcher in molecular biology",
    "voices.q5":
      "“Don’t interpret these metabolomics results yet. Check missing values, batch effects, and outliers first, then turn the recommended statistical steps into a rerunnable script.”",
    "voices.a5": "Prioritizes data-quality checks before producing a reproducible analysis workflow.",
    "voices.role5": "Translational medicine researcher",
    "voices.q6":
      "“Search ChEMBL and PubChem for these candidate molecules, compare activity, selectivity, and known risks, then identify the three most valuable experiments to run next.”",
    "voices.a6": "Combines database evidence with local analysis to prioritize costly follow-up experiments.",
    "voices.role6": "Computational chemistry researcher",
    "features.kicker": "Built for scientific research",
    "features.heading": "Built for research",
    "features.lead":
      "An independent open-source Rust/Tauri workbench: the agent core, scientific toolchain, and desktop UI all run locally. Your data stays on the machine.",
    "features.artTitle": "Scientific artifacts, fully traceable",
    "features.artBody":
      "Tables, code blocks, LaTeX, and file paths in a conversation become artifacts with a live sidebar preview. Markdown rendering includes KaTeX and syntax highlighting.",
    "features.artPre":
      "{{artifact:abc123}}\n→ Table · 8 rows × 4 cols\n→ figure_umap.py · 42 lines\n→ $\\hat{\\beta}$ · LaTeX equation\n\nEach artifact stays bound to the conversation\nthat produced it—for replay, edits, and provenance.",
    "features.pyTitle": "Persistent Python / R kernels",
    "features.pyBody":
      "Long-lived kernel workers keep variables and DataFrames in memory, isolated per conversation. Iterate across turns without reloading data, and parallel sessions never share state.",
    "features.pyPre":
      ">>> import scanpy as sc\n>>> adata = sc.read_h5ad(\"pbmc.h5ad\")\n>>> adata.shape\n(2700, 32738)\n\n# Variables remain available in later tool calls\n>>> sc.pp.neighbors(adata)\n>>> sc.tl.umap(adata)",
    "features.skillTitle": "34 bundled SKILL workflows",
    "features.skillBody":
      "Literature review, analysis modules, figures, Python/R environments, and remote SSH compute—ready to load.",
    "features.skillPre":
      "skills/\n├─ literature-review/\n├─ analysis-workflow/\n├─ remote-compute-ssh/\n├─ figure-composer/\n└─ paper-narrative/ …\n\nThe agent loads SKILL.md on demand via use_skill",
    "features.llmTitle": "Any LLM backend",
    "features.llmBody":
      "OpenAI-compatible, OpenAI Responses, Anthropic API, and local <a href=\"acp-agents.html\">ACP agents</a>. <a href=\"model-configuration.html\">See HTTP model configuration</a>.",
    "features.ctxTitle": "Three-layer context compression",
    "features.ctxBody":
      "Full history is archived first, then tool/image noise is trimmed safely. When semantic turns must go, an incremental summary checkpoint is written from the original history, and a bounded recent window remains.",
    "features.sqlTitle": "SQLite persistence",
    "features.sqlBody":
      "Projects, session frames, messages, and artifacts land in a local database. Restart the app and the history is back.",
    "local.kicker": "Works wherever your data lives",
    "local.heading": "Keep the data local",
    "local.lead":
      "Windows (WebView2), macOS (Apple Silicon / Intel, separate builds), and Linux desktops, or a headless CLI. Native biological tools work without Python; optional runtimes can be configured after launch.",
    "local.pkgTitle": "Self-contained installers",
    "local.pkgBody":
      "Skills, Python, MCP, and demo sessions ship inside Windows MSI/NSIS, macOS dmg, and Linux .deb / AppImage packages. No source tree required.",
    "local.mcpTitle": "Unified bio MCP",
    "local.mcpBody":
      "mcp_bio starts by default: about 240 tools covering PubMed, UniProt, ChEMBL, and 80+ other databases.",
    "local.remoteTitle": "Remote compute",
    "local.remoteBody":
      "SKILLs cover SSH, WSL, and GPU hosts. The agent can write and submit long Runs with live logs.",
    "usecases.kicker": "How researchers use Wisp Science",
    "usecases.heading": "How researchers use it",
    "usecases.lead":
      "Bundled life-science workflows; interdisciplinary projects can chain literature, analysis, and figures in one session.",
    "usecases.tabScrna": "Single-cell RNA-seq",
    "usecases.tabProtein": "Protein structure",
    "usecases.tabChem": "Cheminformatics",
    "usecases.tabLit": "Literature & writing",
    "usecases.scrnaTitle": "Single-cell and transcriptome analysis",
    "usecases.scrnaBody":
      "Use analysis-workflow to package QC, clustering, and annotation as reproducible modules. Run Scanpy and UMAP in the persistent Python kernel.",
    "usecases.scrnaLi1": "Query GEO / CELLxGENE metadata through MCP",
    "usecases.scrnaLi2": "Tables and figures land in the artifact panel",
    "usecases.scrnaLi3": "Code stays bound to the conversation for replay",
    "usecases.promptLabel": "Example prompt",
    "usecases.scrnaPrompt":
      "Run a standard Scanpy workflow on PBMC 3k, label major cell types, and export a marker-gene heatmap.",
    "usecases.proteinTitle": "Protein structure and sequence",
    "usecases.proteinBody":
      "Pull sequences and experimental structures from UniProt / PDB, then combine literature review and figures into citable site tables and Methods drafts.",
    "usecases.proteinLi1": "Fetch sequences and structures via UniProt / PDB MCP",
    "usecases.proteinLi2": "A 3D structure viewer is still on the roadmap",
    "usecases.proteinLi3": "Chain literature-review SKILLs for hypothesis generation",
    "usecases.proteinPrompt":
      "Fetch a predicted TP53 structure, overlay ClinVar pathogenic variants, and draft a Methods paragraph.",
    "usecases.chemTitle": "Cheminformatics and molecule design",
    "usecases.chemBody":
      "Search ChEMBL and PubChem for activity data, compute properties and similarity, and compose SAR figures with figure-composer.",
    "usecases.chemLi1": "Compute physicochemical properties and similarity",
    "usecases.chemLi2": "Preview SMILES / MOL / SDF in the right pane (RDKit)",
    "usecases.chemLi3": "figure-composer for a consistent figure style",
    "usecases.chemPrompt":
      "Retrieve EGFR IC50 data, filter candidates with better oral bioavailability, and plot a SAR heatmap.",
    "usecases.litTitle": "Literature search and manuscript drafts",
    "usecases.litBody":
      "literature-review, paper-narrative, and pdf-explore SKILLs take you from PDFs to structured reviews and narrative.",
    "usecases.litLi1": "PubMed / Semantic Scholar MCP search",
    "usecases.litLi2": "Live Markdown + LaTeX preview",
    "usecases.litLi3": "indication-dossier templates",
    "usecases.litPrompt":
      "Write a Discussion from the attached PDFs, include a reference list, and check numbers against the tables.",
    "demo.kicker": "See it in action",
    "demo.heading": "Bundled demo sessions",
    "demo.lead":
      "Open demo in the desktop app for read-only traces: CRISPR screens, enzyme engineering, extremophiles, and immunotherapy—no API credits required.",
    "demo.crisprTitle": "CRISPR Screen",
    "demo.crisprBody": "A full agent trace from raw counts to hit calling and a visualization report.",
    "demo.enzymeTitle": "Enzyme Engineering",
    "demo.enzymeBody": "Multi-turn tool use for sequence analysis, mutation proposals, and activity reasoning.",
    "demo.extTitle": "Extremophile",
    "demo.extBody": "Cross-database metagenomic search and functional annotation.",
    "demo.immTitle": "Immunotherapy",
    "demo.immBody": "Tumor-immunology literature synthesis and hypothesis generation.",
    "stack.kicker": "Works with your stack",
    "stack.heading": "Fits the tools you already use",
    "stack.lead":
      "MCP connects biological databases and custom servers; SKILL.md extends reusable pipelines. The agent treats both as first-class tools.",
    "stack.skill": "34 bundled workflows",
    "stack.python": "uv-managed venv · isolated R",
    "stack.browse": "Browse MCP servers",
    "faq.heading": "Frequently asked questions",
    "faq.q1": "Is Wisp Science a new model?",
    "faq.a1":
      "No. It is an open-source desktop and CLI app that talks to any compatible LLM with the API key you supply. What is new is the agent loop, tools, MCP, and Python / R kernels around that model.",
    "faq.q2": "How is it different from a generic AI assistant?",
    "faq.a2":
      "It actually executes: read and write local files, run a shell, call persistent Python / R REPLs, query PubMed/UniProt through MCP, and store the full session in SQLite. It ships 34 domain SKILLs instead of only generating text.",
    "faq.q3": "Does research data stay private?",
    "faq.a3":
      "Raw data and compute stay local; sessions and artifacts live in on-disk SQLite. Only prompts and model responses go to your LLM provider, under that API’s privacy policy.",
    "faq.q4": "Which platforms are supported?",
    "faq.a4":
      "Windows (signed MSI/NSIS + WebView2), macOS (signed and notarized Apple Silicon / Intel dmgs), and Linux (.deb / AppImage for x86_64 and aarch64) all ship with GitHub Releases. macOS opens with a double-click. If Windows SmartScreen still appears, choose Run anyway.",
    "faq.q5": "Why does macOS keep asking for the keychain password?",
    "faq.a5":
      "API keys live in the macOS login keychain, bound to the code-signing identity that wrote the item. If you saved a key in unsigned v0.4.x, that item is bound to the old identity; a signed build looks like a different app, so macOS keeps asking you to re-authorize. Fix: open Keychain Access, delete the <code>wisp</code> item, reopen Wisp, paste the key once, and click Always Allow.",
    "faq.q6": "What are the prerequisites?",
    "faq.a6":
      "Installer users: Windows needs WebView2 (usually present on Windows 10/11), macOS uses system WebKit, Linux uses WebKitGTK; all need your own API key. Optionally install R with jsonlite for the persistent r tool. Building from source needs Rust, Trunk, and Tauri CLI v2 (plus Xcode command-line tools on macOS). First-run setup detects executable paths in the background and saves them to Local. Missing tools do not block startup or trigger installs; use local-env-setup after launch when needed.",
    "faq.q7": "Is Wisp Science an independent project?",
    "faq.a7":
      "Yes. We originally looked at closed products such as Claude Science, but they were unfriendly to some regions and locked down. Wisp Science started from that gap: it learned from their Skills and MCP tool choices, then implemented a local-first research workbench in Rust/Tauri (AGPL-3.0). The agent architecture, multi-project workflow, Run manager, plugins, and ACP support are original; it can talk to any model provider, and anyone can use, study, and improve it.",
    "faq.q8": "Is the current release production-stable?",
    "faq.a8":
      "v1.5.0 is an active preview for local scientific workflows. The core agent, streaming, tools, Python / R, MCP, share export, and desktop UI run; still review critical methods and outputs, and check the release notes for current signing and update status.",
    "footer.copy": "© 2026 Wisp Science · Open-source scientific computing agent",
    "models.lead":
      "The desktop app manages remote API backends as model profiles. Each profile can set a provider, API URL, model ID, advanced parameters, and its own API key.",
    "models.download": "Download the latest release",
    "models.acpCta": "ACP agent setup",
    "models.md": "View Markdown docs",
    "models.compatBody":
      "DeepSeek, GLM, a local gateway, or any service compatible with /chat/completions. Needs an API URL, model ID, and API key.",
    "models.respBody":
      "Use OpenAI reasoning / tool-call models through /v1/responses. Needs an API URL, model ID, and API key.",
    "models.anthBody": "Use the Claude API through /v1/messages. Needs an API URL, model ID, and API key.",
    "models.keyBody":
      "API keys go in the OS keyring. Profile name, provider, URL, model ID, and capability tags go in the project’s .wisp/wisp.sqlite.",
    "models.fieldsHeading": "Common fields",
    "models.fieldsLead":
      "Configure these in Settings -> Models on desktop. Multiple profiles can coexist; switch the active one at any time.",
    "models.thField": "Field",
    "models.thBackend": "Backends",
    "models.thHelp": "Notes",
    "models.all": "All",
    "models.displayHelp": "The profile name shown in the UI.",
    "models.providerHelp": "Choose OpenAI-compatible, OpenAI Responses, or Anthropic.",
    "models.modelHelp": "The model name on the remote API.",
    "models.urlHelp":
      "Remote API base URL and secret. Do not append /v1, /chat/completions, /responses, or /v1/messages; Wisp completes the path from the provider and probes common OpenAI-compatible routes. Desktop keys are not written to SQLite.",
    "models.cliLead": "The wisp-science headless CLI configures the remote API provider with environment variables.",
    "acp.lead":
      "Wisp can act as an ACP client and launch local ACP v1 agents such as Codex or Claude. This is separate from HTTP model profiles: Add model covers APIs; ACP Agents covers local stdio agent processes.",
    "acp.start": "Start setup",
    "acp.httpCta": "HTTP model setup",
    "acp.md": "Markdown docs",
    "acp.splitHeading": "Two backends—do not mix them",
    "acp.splitLead":
      "Settings → Models uses two categories: Models (HTTP APIs) and ACP Agents (local processes).",
    "acp.httpTitle": "HTTP models",
    "acp.httpBody":
      "Settings → Models → Add model. Remote APIs such as DeepSeek / OpenAI / Anthropic run through the built-in Wisp agent.",
    "acp.agentTitle": "ACP agent",
    "acp.agentBody":
      "Settings → Models → ACP Agents. A local stdio process; the agent owns its session, tools, and auth.",
    "acp.cliTitle": "Do not paste a bare CLI",
    "acp.cliBody": "Do not put codex / claude directly into ACP. Use the official ACP adapters.",
    "acp.credTitle": "Credentials stay out of SQLite",
    "acp.credBody": "Login and API keys are owned by the external agent. Wisp only stores the launch command and arguments.",
    "acp.whereHeading": "Where to click",
    "acp.whereLead": "ACP sits beside Add model, both under Models.",
    "acp.step1Body": "Open Models in the settings sidebar, then use the top tabs to switch Models / ACP Agents.",
    "acp.step2Title": "2. The same Add / Edit flow",
    "acp.step2Body":
      "Add model or Add ACP Agent on the right opens a breadcrumb subpage form. Click a row to edit.",
    "acp.step3Title": "3. Composer shortcut",
    "acp.step3Body": "Add model / Add ACP Agent at the bottom of the model menu opens the same forms.",
    "acp.fillHeading": "Fill in and enable",
    "acp.fillLead": "Install the adapter first, then save, test, and select it in Wisp.",
    "acp.thValue": "Value",
    "acp.labelHelp": "Display name, for example Codex ACP",
    "acp.cmdHelp": "Executable only—do not put arguments on this line",
    "acp.argsHelp": "One argument per line (especially when Windows uses npx.cmd)",
    "acp.testBody": "Success means the process started and completed ACP initialize.",
    "acp.authTitle": "Authenticate (if shown)",
    "acp.authBody": "If a login button appears after the test, follow the agent prompt. Credentials are not stored in Wisp.",
    "acp.emptyTitle": "Select it in an empty session",
    "acp.emptyBody":
      "Create an empty session, pick the ACP agent in the model menu, then send a message. The backend locks after the first message; start a new session to switch.",
    "acp.exHeading": "Examples: Codex / Claude",
    "acp.exLead": "Prefer a global install; npx also works if arguments are split across lines.",
    "acp.orNpx": "Or npx (on Windows, set Command to npx.cmd):",
    "acp.empty": "(empty)",
    "acp.orPath": "(or an absolute path)",
    "acp.repos": "Adapter repos:",
    "acp.notesHeading": "While you use it",
    "acp.permTitle": "Permissions and settings",
    "acp.permBody":
      "The permission card shows options the agent returns. Session-level model/mode defaults live in the ACP model menu next to Send.",
    "acp.sciTitle": "Scientific tools",
    "acp.sciBody": "Wisp injects an MCP bridge so the external agent can call bundled scientific tools inside the project directory.",
    "acp.limitTitle": "Current limits",
    "acp.limitBody":
      "Local stdio only; no remote / WSL / SSH ACP; no in-app marketplace; changing Command/Arguments requires a new session.",
    "acp.troubleHeading": "Troubleshooting",
    "acp.thSymptom": "Symptom",
    "acp.thFix": "What to do",
    "acp.failHelp": "Command is not on PATH, Windows should use npx.cmd, or arguments were pasted into Command",
    "acp.authFail": "Finish Codex / Claude login or API key in the system first, then retest",
    "acp.locked": "Expected; start a new empty session to change backends",
    "acp.pathChanged": "After changing launch arguments or the project path, start a new session",
  },
};

const WISP_PAGES_LANG_KEY = "wisp-pages-lang";

function wispPagesLangFromUrl() {
  const value = new URLSearchParams(location.search).get("lang");
  return value === "en" || value === "zh" ? value : null;
}

function wispPagesLang() {
  const fromUrl = wispPagesLangFromUrl();
  if (fromUrl) return fromUrl;
  try {
    const stored = localStorage.getItem(WISP_PAGES_LANG_KEY);
    if (stored === "en" || stored === "zh") return stored;
  } catch {
    /* ignore closed storage */
  }
  return "zh";
}

function wispPagesApply(lang) {
  const pack = WISP_PAGES_I18N[lang] || WISP_PAGES_I18N.zh;
  const page = document.documentElement.dataset.page || "home";
  document.documentElement.lang = lang === "en" ? "en" : "zh-CN";
  document.documentElement.dataset.lang = lang;
  const title = pack[`meta.${page}.title`];
  const desc = pack[`meta.${page}.desc`];
  if (title) document.title = title;
  const meta = document.querySelector('meta[name="description"]');
  if (meta && desc) meta.setAttribute("content", desc);

  document.querySelectorAll("[data-i18n]").forEach((el) => {
    const text = pack[el.dataset.i18n];
    if (text == null) return;
    el.textContent = text;
  });
  document.querySelectorAll("[data-i18n-html]").forEach((el) => {
    const html = pack[el.dataset.i18nHtml];
    if (html == null) return;
    el.innerHTML = html;
  });
  document.querySelectorAll("[data-i18n-aria]").forEach((el) => {
    const text = pack[el.dataset.i18nAria];
    if (text != null) el.setAttribute("aria-label", text);
  });
  document.querySelectorAll(".lang-switch [data-lang]").forEach((btn) => {
    btn.setAttribute("aria-pressed", btn.dataset.lang === lang ? "true" : "false");
  });
  document.documentElement.classList.add("i18n-ready");
}

function wispPagesSetLang(lang) {
  try {
    localStorage.setItem(WISP_PAGES_LANG_KEY, lang);
  } catch {
    /* ignore */
  }
  const url = new URL(location.href);
  url.searchParams.set("lang", lang);
  history.replaceState(null, "", url);
  wispPagesApply(lang);
}

document.addEventListener("DOMContentLoaded", () => {
  wispPagesApply(wispPagesLang());
  document.querySelectorAll(".lang-switch [data-lang]").forEach((btn) => {
    btn.addEventListener("click", () => wispPagesSetLang(btn.dataset.lang));
  });
});

globalThis.WISP_PAGES_I18N = WISP_PAGES_I18N;
globalThis.wispPagesLang = wispPagesLang;
globalThis.wispPagesApply = wispPagesApply;
globalThis.wispPagesSetLang = wispPagesSetLang;
