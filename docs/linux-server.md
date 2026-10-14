# Linux Server：部署一次，通过手机使用 Wisp

`wisp-server` 是无桌面服务端：同一个进程提供手机网页、加密远程协议和 Agent，
在这台 Linux 上读写项目文件、执行工具和管理 Run。无需 Tauri、WebKit、X11、
桌面登录，也无需另外部署 `wisp-relay`。手机通过浏览器打开网页即可使用。

```text
手机浏览器 ── HTTPS/WSS ── 同机 Caddy ── wisp-server ── 本地项目 / Agent / Run
```

Caddy 只负责 HTTPS 证书与转发，与 Wisp 放在同一台机器上。手机仍需能通过公网、
局域网或 VPN 到达这台服务器。把原来的 relay 镜像换成配置参数不能获得计算能力；
请使用这里的独立服务端构建。

## Docker Compose 部署

以下操作在 Linux 服务器上进行。需要 Docker Compose、指向服务器的域名，以及
可用的模型 API 配置。当前配置从源码构建本地镜像；不假定已有公开的 server 镜像。

1. 在本仓库中进入部署目录，并建立本机配置：

   ```bash
   cd deploy/server
   cp .env.example .env
   chmod 600 .env
   openssl rand -hex 16
   ```

2. 编辑 `.env`：填写域名、上一步生成的 `WISP_SERVER_CODE`、模型名称、API 地址和
   API Key。模型支持 `openai`（兼容 Chat Completions）、`openai_responses`、
   `anthropic`。API Key 与连接码只通过进程环境注入，不写入 SQLite。
   首版没有网页模型设置页；修改模型配置后重建容器即可生效。

3. 开放服务器 TCP 80、443，启动：

   ```bash
   docker compose up -d --build
   docker compose logs --tail=50 wisp caddy
   ```

4. 手机上打开 `https://你的域名/remote`，输入 `.env` 中的连接码。也可以生成带连接码
   的完整链接；该命令明确输出凭证，请仅在自己的终端使用：

   ```bash
   docker compose exec wisp wisp-server --connection-url
   ```

5. 选择自动建立的 **Server** 项目，新建会话并发送指令。写文件、编辑文件、运行命令
   等操作会在手机上显示审批卡片，包含工具参数；批准后才执行。Safari 或 Chrome
   可以把网页添加到主屏幕。

第一次构建需要下载 Rust 依赖和基础镜像。容器以 UID 10001 运行，项目和数据库存放在
`wisp-data` 卷中，默认项目目录是 `/data/workspace`。镜像包含 Python、uv、Git、SSH
客户端和 Python/R worker；Python 分析环境按任务需要准备，R 本体和额外系统依赖需
在自定义镜像中安装。安装依赖可能需要网络；模型调用也需要能到达配置的 API。

模型上下文/输出上限使用与桌面版相同的 models.dev 快照和精确 ID 查询。已知模型的
环境覆盖值不能超过目录上限。自定义模型必须同时填写 `WISP_MAX_CONTEXT` 和
`WISP_MAX_TOKENS`，不会按模型名称前缀猜测。

## 已有 HTTPS 反向代理

可以只运行 `wisp` 服务，将它的 8787 端口映射到服务器回环地址：

```yaml
# compose.override.yaml
services:
  wisp:
    ports:
      - "127.0.0.1:8787:8787"
```

```bash
docker compose up -d --build wisp
```

让现有代理把 HTTPS 与 WebSocket Upgrade 转发到 `127.0.0.1:8787`。路径前缀也可使用，
例如 `https://example.com/wisp/`，此时显式将服务的 `WISP_SERVER_PUBLIC_URL` 改为该地址，
代理去掉 `/wisp/` 前缀。`GET /healthz` 表示服务已打开数据库并开始监听，不检查模型 API。

手机浏览器使用 WebCrypto 加密，因此即使在局域网里也需要受手机信任的 HTTPS 证书。
`http://localhost` 仅用于在服务所在机器上开发，手机访问服务器 IP 不属于这个例外。

## 原生二进制与 systemd

也可在 Linux 源码目录构建，编译依赖包括 Rust、C 编译器、pkg-config、D-Bus 开发库：

```bash
cargo build --release --locked -p wisp-server
```

安装 `target/release/wisp-server` 到 `/usr/local/bin/wisp-server`，建立 `wisp` 系统用户，
把 [服务配置](../deploy/server/wisp-server.service) 安装到
`/etc/systemd/system/wisp-server.service`。创建权限为 `0600`、由 root 持有的
`/etc/wisp-server.env`，例如：

```ini
WISP_SERVER_PUBLIC_URL=https://science.example.com
WISP_SERVER_CODE=替换为32位随机十六进制连接码
WISP_PROVIDER=openai
WISP_API_URL=https://api.openai.com/v1
WISP_MODEL=你的模型ID
WISP_API_KEY=你的API密钥
WISP_SKILLS_PATH=/opt/wisp/skills
WISP_KERNEL_WORKER=/opt/wisp/python/kernel_worker.py
WISP_R_KERNEL_WORKER=/opt/wisp/r/kernel_worker.R
```

将仓库的 `skills/`、`python/`、`r/` 安装到上述对应目录，并在系统 PATH 中提供 Python、uv
及任务需要的程序。启动服务：

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now wisp-server
```

同机 Caddy 配置：

```caddyfile
science.example.com {
    reverse_proxy 127.0.0.1:8787
}
```

可选环境变量：`WISP_SERVER_DATA`（独立数据目录）、`WISP_SERVER_WORKSPACE`
（默认项目目录）、`WISP_SERVER_BIND`（默认 `127.0.0.1:8787`）、`WISP_MAX_ITER`
（每轮最大迭代，默认 200）、`WISP_REASONING_EFFORT`。数据库目录不能放在手机文件浏览器
所能访问的项目目录内。同一数据目录只允许一个服务进程持有，不要把它与桌面版共享运行。

## 断线、重启和升级

- 手机锁屏、关闭页面或断线不会取消已接收的 Agent 任务；重新连接可看到当前进度和审批。
  同一会话一次执行一轮，整个服务最多同时执行四轮。
- 如果任务在读取会话快照时完成，服务会重新读取历史，确保“已完成”的快照包含最终回复。
- 提交请求先记录到 SQLite。同一请求 ID 再次到达不会重复执行；同 ID 携带不同内容会报错。
  请求 ID 随会话保留，页面本身也不会自动重试写操作。
- 服务重启后保留项目、会话记录和 Run 记录。未完成的 Agent 轮次标记为中断，不自动重新
  执行。正在等待的审批失效。Python/R 进程内变量不跨服务重启保留；进程运行期间同一会话
  复用运行时。已有 Run 使用共享的恢复机制，不能假定容器内的计算进程跨重启存活。
- 升级前等待任务结束并备份数据卷，然后更新源码并执行 `docker compose up -d --build`。
  保留原来的连接码，手机连接地址不变。更换连接码并重建容器可以撤销旧手机的访问。
- 一个连接码对应一个受信任的使用者空间，不提供多人账号或只读访客角色。持有连接码的人
  可以操作该服务用户的工作空间。默认日志不打印连接码、API Key 或带凭证的链接。

## 首版范围

包含默认项目、会话创建和历史、流式回复、手机一次性审批、停止 Agent、Run 状态卡片、
本地目录和文本/Markdown 预览。底层复用现有 Agent、工具、运行时、Run 管理和 SQLite
存储，手机接入复用现有加密帧、连接码和网页。服务端只公布实际支持的命令。

暂不包含网页模型/项目设置、图片上传及预览、PDF/Office 渲染、消息排队、桌面终端、
ACP、订阅 OAuth 登录和完整桌面功能。模型与工作目录在部署时配置；首版没有额外中转
服务器、桌面预配置步骤或桌面进程依赖。独立 relay 的项目同步与 Wisp Lab 功能不随本服务启用。

## 验证

相关离线测试使用临时数据库和假模型，不需要 API Key、SSH、GPU 或模型网络访问。
开发者须先取得仓库要求的测试许可，再执行：

```bash
cargo check -p wisp-server --offline
cargo test -p wisp-server --offline
cargo test -p wisp-sync direct::tests --offline
cargo test -p wisp-store server_turns::tests --offline
```

实际 Linux 部署的手动检查：手机打开 HTTPS 网页；创建会话；让 Agent 写一个小文件；
确认批准前文件不存在、批准后能预览；执行中锁屏再连接；等待审批时停止；完成后重启服务，
确认历史仍在。首次部署还应检查域名证书、数据卷权限及所用模型的连接配置。
