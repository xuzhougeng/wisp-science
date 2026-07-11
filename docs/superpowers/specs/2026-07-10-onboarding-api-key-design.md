# 首次打开引导：加入 API key 配置页

## 目标

首次打开 app 的引导流程，把 **API key 配置作为第一页**，默认 DeepSeek，并给一个链接方便用户去获取 DeepSeek 密钥。

## 现状

- 引导已存在：`OnboardingOverlay`（`ui/src/overlays.rs`），3 步 `welcome → connect → features`，靠后端 `onboarding_done` 设置只弹一次（`get_onboarding_state` / `dismiss_onboarding`）。
- 第 1 步 `connect` 只是文字提示"去设置里加 key"，没有输入框。
- 设置里的 Provider 表单默认值已经是 DeepSeek：`provider_defaults("openai")` → `https://api.deepseek.com` / `deepseek-v4-pro`（`ui/src/text.rs`）。保存走 `save_model` 命令，key 存进系统 keyring。
- `open_external_url(url)`（`ui/src/bindings.rs`）可跳转外部浏览器。

## 方案（精简版）

仍 **3 步**，把原 `connect` 文字页换成真正的密钥表单并挪到首页：

0. **配置模型密钥**（首页，替换原 `connect`）
1. welcome（原样）
2. features（原样）

### 第 0 页内容（精简）

- **Provider 下拉**：默认 `openai`（DeepSeek/OpenAI 兼容），可切 `openai_responses` / `anthropic`。
- **API key 输入框**（password）。
- **"获取 DeepSeek API 密钥 →" 链接**：`open_external_url` 打开 `https://platform.deepseek.com/api_keys`。链接文案随 provider 变：DeepSeek 指 deepseek，Anthropic 指 `https://console.anthropic.com/settings/keys`，OpenAI 指 `https://platform.openai.com/api-keys`。
- URL / Model / max_tokens / reasoning effort **不显示**，用 `provider_defaults(provider)` 的默认值，之后可在设置里改。

### 交互

- **下一步**：若 key 非空 → 用 `provider_defaults(provider)` 组装 profile（空 label）+ key 调 `save_model`，成功后 `onboard_step += 1`；key 为空 → 直接前进（允许跳过，之后能在设置补）。
- **上一步 / Esc**：沿用现有 `onboard_step` 回退逻辑。
- 步数不变（3 步），圆点 `0..3`、`step < 2` 判断都不动；末页仍是"开始使用"→ `dismiss_onboard`。match 三个分支：0=密钥表单，1=welcome，2=features。

### 状态与保存

新增两个信号（`main.rs`）：`onboard_provider: RwSignal<String>`（默认 `"openai"`）、`onboard_key: RwSignal<String>`。新增回调 `save_onboard_key`：非空时按 `save_model_form` 同样的 profile JSON 结构调 `save_model`（api_url/model 取 `provider_defaults`）。二者传入 `OnboardingOverlay`。不复用设置的 `model_form`，避免污染设置的"添加模型"表单。

## 改动文件

- `ui/src/overlays.rs` — `OnboardingOverlay` 加 step 0 表单、参数。
- `ui/src/main.rs` — 新信号 + `save_onboard_key` 回调 + 传参 + `0..4`/`step<3`。
- `ui/src/i18n.rs` — 新增 En/Zh key：`onboard.apikey.title/body/provider/key/get_key/skip_hint`；删 `onboard.connect.*`（或留着不用）。
- `ui/src/styles/overlay.css` — 第 0 页表单左对齐等小样式（`.onboard` 目前 center，表单项需 left）。

## 验证

- `?mock=1` 或首次运行看引导第 0 页渲染、下拉切换、链接跳转、下一步保存（mock 下可只验证渲染 + 前进）。
- 填 key → 下一步 → 进设置 Models 应能看到新模型且 has_api_key。

## 不做（YAGNI）

- 不做完整 provider 表单（url/model/tokens/effort）——需要时去设置改。
- 不做 key 校验/valid 按钮——设置里已有。
- 不强制填 key 才能继续。
