# Wisp 命令行

熟悉桌面中的项目与对话后，如果希望在系统终端里使用 Wisp，或者把一次任务接入脚本与日志流程，可以使用独立的 `wisp-science` CLI。

这篇教程介绍 CLI 的准备、模型环境变量、交互模式与单次任务。Wisp 会以当前目录作为工作区，因此开始前先确认终端所在的项目目录。桌面中的 SSH 环境与交互终端操作，见[服务器环境配置](wisp-science-servers-cli.md)。

> 本文中的 API 地址、模型 ID 和文件路径用于演示，请替换为自己的实际配置；示例命令不代表已经执行的分析结果。

**先确认独立 CLI 可以运行。**

如果已经构建或安装了独立 CLI，并且 `wisp-science` 在 PATH 中，可以在项目目录运行它。桌面应用已安装，不一定意味着这个命令已经加入了系统 PATH。

**为当前终端配置模型。**

CLI 使用环境变量配置模型，不会自动把桌面密钥环里的配置变成终端环境变量。以兼容接口为例，先把下面的占位内容换成自己的实际信息。

macOS / Linux：

```bash
export WISP_PROVIDER="openai"
export WISP_API_URL="https://your-api.example.com"
export WISP_MODEL="your-model-id"
read -s WISP_API_KEY
export WISP_API_KEY
wisp-science
```

执行 `read -s WISP_API_KEY` 后，在终端输入密钥并回车，输入不会回显。示例中的 URL 和模型 ID 都是占位值；不要原样用于连接。

Windows PowerShell：

```powershell
$env:WISP_PROVIDER = "openai"
$env:WISP_API_URL = "https://your-api.example.com"
$env:WISP_MODEL = "your-model-id"
$credential = Get-Credential -UserName "api" -Message "在密码字段输入 API Key"
$env:WISP_API_KEY = $credential.GetNetworkCredential().Password
wisp-science
```

`WISP_PROVIDER` 可按服务协议选择 `openai`、`openai_responses`、`openai_codex` 或 `anthropic`。`openai_codex` 使用 ChatGPT Plus/Pro 订阅：先运行 `wisp-science login codex`（远程或 WSL 可加 `--method device`），之后不需要 `WISP_API_KEY`。

**用交互模式，在同一个项目目录中继续对话。**

运行 `wisp-science` 后，在交互模式中输入自然语言任务。例如：

> 请只读检查当前项目的目录，列出可能的数据文件、分析脚本和结果目录。先不要安装依赖或修改文件。

`/help` 查看帮助，`/new` 开始新会话，`/compact` 压缩上下文，`/quit` 退出。

**只执行一次任务，或输出结构化事件。**

需要单次执行时，可以在项目目录运行：

```bash
wisp-science run "只读列出当前项目的顶层文件，并说明可能的数据、脚本和结果目录"
wisp-science run --output jsonl "只读检查 data/example.csv 的列名和缺失值"
```

`jsonl` 按行输出结构化事件，适合接入日志或脚本。它仍然会调用实际模型；命令能启动，并不代表示例路径存在或分析必然成功。

**从源码使用时，留意当前工作目录。**

源码开发者可在仓库根目录使用 `cargo run -p wisp-cli -- run "任务"`。这种写法默认以当前仓库目录为工作区；要分析另一个目录中的项目，先构建 CLI，再到目标项目目录运行可执行文件。构建和更多参数见[开发文档](../development.md)。

**遇到问题，先检查命令、配置和目录。**

| 现象 | 优先检查 |
| --- | --- |
| `wisp-science` 命令不存在 | 是否已构建／安装独立 CLI，PATH 是否包含可执行文件目录 |
| CLI 缺少模型密钥 | 环境变量是否在当前终端会话中设置，是否误以为它会继承桌面配置 |
| 模型请求失败 | API 地址、协议、模型 ID 和当前账号权限是否匹配 |
| 找不到输入文件 | 当前终端是否位于目标项目，示例路径是否已经替换 |
| 需要的 Python／R 包不可用 | 实际使用的解释器和依赖环境是否准备好 |

第一次尝试，可以先在一个小型练习目录中启动 CLI，让它只读列出文件。确认模型能回答、目录正确，再尝试数据检查或单次任务输出。

> 配置和构建细节参见 [CLI 开发文档](../development.md)与[模型配置文档](../model-configuration.md)。本文依据撰写时的项目实现整理，不同版本的参数和提示文字可能略有差异。
