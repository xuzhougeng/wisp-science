# Wisp Science高级用法：用 map_items 批量处理文献

下载了 100 篇 PDF，要逐篇判断“是不是随机对照试验、样本量多少、是否符合纳入标准”。在一段对话里让 Agent 一篇篇读，读到二十来篇上下文就满了；拆成 100 段对话，又没法汇总。

这类“同一个任务重复 N 次”的工作，交给 `map_items`：一条指令、N 个条目，每个条目在自己独立的短上下文里处理，结果写成一张表。主对话只收到计数和简短清单，不会被全文淹没。

这篇教程用“筛选 100 篇文献”走一遍完整流程：怎么让 Agent 调用它、审批时看什么、结果在哪、怎么续跑和二次筛选。

> 本文示例中的文件名、运行编号、数字和回答均为示意，用于说明交互方式，不代表已经执行的筛选结果。

**先弄清一件事：`map_items` 是 Agent 的工具，不是你要敲的命令。**

你不需要手写 JSON。在项目对话里用自然语言描述批量任务，Agent 会自己组织参数并调用；你要做的是把任务说清楚，并在审批时确认一次。下文出现的 JSON 是 Agent 实际发出的调用，放在这里是为了让你看懂审批提示和工具卡片，也方便在 Agent 选错工具时直接点名“用 map_items”。

它和另外几种“分出去做”的方式分工不同：

| 场景 | 用什么 |
| --- | --- |
| 同一个任务重复很多次：筛 100 篇文献、从 50 个 PDF 抽同一张表 | `map_items` |
| 少数几个互不相同的任务，最多 8 个 | `delegate_tasks`，或 [Agent Workflow](wisp-science-agent-workflow.md) |
| 一次开放式的调查 | `explore` |

**准备：把条目放进项目，按需配置 TypeSafe 密钥。**

- 把文件放在项目目录下，例如 `papers/`。PDF、Office 文档、EPUB 和纯文本都在本机转成文字，这一步不消耗 token。
- 扫描版 PDF 没有可提取的文字，暂不支持 OCR；这样的文件只会让它自己那一行失败。
- 一次最多 1000 个条目。超过会直接报错，不会悄悄截断。
- 只做信息抽取不需要额外配置。想让每一行自动得到“通过 / 剔除 / 不确定”的判定，需要在“设置 → 凭据”里填写 TypeSafe（Jev）的 API key；命令行则设置环境变量 `TYPESAFE_API_KEY`。没有密钥时，Agent 的工具里不会出现判定这一步。

**第一步：把“对一个条目做什么”说清楚。**

在项目对话里发送：

> 用 map_items 处理 `papers/` 下所有 PDF。每篇提取：研究设计（rct、cohort、case-control 或 other）、样本量、主要结局指标。先只抽取，不做判定。

好的批量指令有三个特点：针对**单个条目**；自包含，处理条目的模型看不到你们之前聊过什么；字段固定。Agent 会把它整理成这样的调用：

```json
{
  "items": { "glob": "papers/*.pdf" },
  "worker": {
    "instruction": "Read this paper. Report its study design, sample size and primary outcome.",
    "output_schema": {
      "type": "object",
      "required": ["design", "sample_size", "primary_outcome"],
      "properties": {
        "design": { "enum": ["rct", "cohort", "case-control", "other"] },
        "sample_size": { "type": "integer" },
        "primary_outcome": { "type": "string" }
      }
    }
  }
}
```

`worker` 是每个条目上的一次抽取，`output_schema` 规定一行结果长什么样。某一篇的回答不符合这个结构，那一行记为失败并保留原因，不会混进后面的汇总。

**第二步：看懂审批提示，确认一次。**

开始消耗 token 之前，Wisp 只问一次（项目的工具审批模式设为“完全放行”时不会询问）。提示大致如下：

```text
map_items will process 100 item(s) — 100 item(s) from glob 'papers/*.pdf'.
Worker: model your-model-id — up to ~3150k input / 100k output tokens; stops and pauses at 3250k total.
Concurrency 4; pauses after 20 min per call (resume continues). Results: .wisp/map-runs/20261006-081530-a1b2c3
```

确认四件事：条目数对不对、用的是哪个模型、token 上限、结果写到哪里。上限按“每篇都读满 60000 个字符”估算，是保守的上界，不是预计花费；到达上限时运行会暂停，而不是截断。

条目清单在这一刻冻结。运行期间往 `papers/` 里新增文件，不会被加进这次运行。

**第三步：读结果。**

运行结束后，Agent 收到一份简短报告，并据此向你汇报：

```text
[map_items run 20261006-081530-a1b2c3: complete — 97 ok, 3 failed, 0 not yet processed, of 100]
failed (3): papers/scan_017.pdf (no extractable text (a scanned PDF? OCR is not supported)), ...
tokens (worker/reduce): 812k in / 21k out; 412s elapsed
rows: .wisp/map-runs/20261006-081530-a1b2c3/rows.jsonl
table: .wisp/map-runs/20261006-081530-a1b2c3/results.csv
continue: map_items {"resume": "20261006-081530-a1b2c3"} (retries the failed items)
```

完整结果保存在项目的 `.wisp/map-runs/<运行编号>/` 下：

| 文件 | 内容 |
| --- | --- |
| `manifest.json` | 冻结的条目清单和参数、运行状态 |
| `rows.jsonl` | 逐条追加的记录；同一条目以最后一行为准 |
| `results.csv` | 每个条目一行：条目、状态、判定、抽取出的字段、各问题的概率、错误原因 |
| `reduce.md` | 汇总结果，只有要求了汇总才会生成 |

`results.csv` 可以直接在项目文件预览中打开。

**第四步：需要筛选时，把判定标准一起交代。**

抽取回答“这篇文献里写了什么”，判定回答“它符不符合要求”。配置了 TypeSafe 密钥后，可以在任务里把标准一并说出来：

> 用 map_items 筛选 `papers/` 下所有 PDF。纳入标准：随机对照试验，且样本量不少于 100。每篇先提取研究设计和样本量，再判定是否纳入。

调用里会多出 `decide`：

```json
"decide": {
  "questions": {
    "is_rct": { "type": "noul", "instructions": "Is `design` a randomized controlled trial?" },
    "large_enough": { "type": "noul", "instructions": "Is `sample_size` at least 100?" }
  }
}
```

`noul` 是“是 / 否”问题，问题里用反引号引用上一步抽取出的字段。每个问题都要写成**回答“是”代表符合要求**。Jev 对每个问题返回一个概率，判定规则是固定的：

| 判定 | 条件 |
| --- | --- |
| `pass` 通过 | 每个是否问题的概率都不低于 `pass_at`，默认 0.8 |
| `reject` 剔除 | 任何一个是否问题的概率不高于 `reject_at`，默认 0.2 |
| `uncertain` 不确定 | 其余情况，包括概率缺失 |

判定只是标签，不会删除或移动任何文件。还有两点需要知道：

- **数据会离开本机。** 使用判定时，每个条目抽取出的那一行会发送给 TypeSafe；如果没有抽取这一步，发送的是条目开头的一段文字。审批提示会写明这一点。
- **推荐“抽取 + 判定”一起用。** 抽取读全文并整理出事实，判定只看这一行紧凑的结果。只用判定适合对标题和摘要做一次便宜的初筛。

**第五步：处理“不确定”和没跑完的条目。**

这几种情况都用一句话交代，Agent 会带上对应的参数：

| 你想做什么 | 可以这样说 | 调用里的关键参数 |
| --- | --- | --- |
| 运行被停止、到了时间或 token 上限、有条目失败 | “继续刚才那次 map_items 运行” | `{"resume": "<运行编号>"}` |
| 觉得阈值太严或太松 | “把通过阈值改成 0.7，重新贴标签” | `resume` 加上 `decide.pass_at` |
| 只复核不确定的那些 | “对上次不确定的文献再跑一遍，这次允许查阅项目里的补充材料” | `"items": {"run": "<运行编号>", "verdict": ["uncertain"]}` |

续跑会跳过已经完成的条目，并重试失败的条目。如果某一条只是判定这一步失败，已经付费得到的抽取结果会被复用，只重新询问 Jev。改阈值后重新贴标签不需要调用模型，因为每一行都保存了原始概率。

续跑只能调整并发、上限和阈值。要更换条目、抽取指令或判定问题，需要开始一次新的运行；用 `items.run` 可以把上一次的结果行作为新的输入。

**按需加上汇总，或换一种条目来源。**

想在表格之外得到一段总结，直接说“最后汇总纳入的研究各用了哪些结局指标”。调用里会多出 `reduce`，它在全部条目完成后只读取结构化的结果行，不读全文；有判定时，默认只汇总通过和不确定的行。

条目不一定是文件夹里的文件：

| 来源 | 写法 | 说明 |
| --- | --- | --- |
| 匹配的文件 | `{"glob": "papers/*.pdf"}` | 相对项目根目录 |
| 指定的文件 | `{"paths": ["a.pdf", "b.docx"]}` | 逐个列出 |
| JSONL 文件 | `{"jsonl": "records.jsonl", "id_field": "pmid", "where": {"year": 2024}}` | 每行一个条目；`where` 按字段相等过滤 |
| 上一次运行 | `{"run": "<运行编号>", "verdict": ["uncertain"]}` | 二次处理 |

四种来源每次只能选一种。

**在命令行里使用。**

交互模式下的用法和桌面一致，审批会以 `[y/n]` 的形式出现在终端里：

```bash
export TYPESAFE_API_KEY="your-typesafe-key"   # 只在需要判定时设置
wisp-science
```

`wisp-science run --output jsonl` 无法应答审批，请求会被自动拒绝，因此批量任务不适合在这个输出模式下启动。模型环境变量的配置见 [Wisp 命令行](wisp-science-cli.md)。

**运行不符合预期时，先看这张表。**

| 现象 | 先检查什么 |
| --- | --- |
| Agent 一篇篇手动读文件，没有调用工具 | 在消息里直接点名“用 map_items”；确认任务确实是同一件事重复多次 |
| 调用里没有判定这一步 | 是否配置了 TypeSafe 密钥；命令行是否设置了 `TYPESAFE_API_KEY` |
| 提示条目数超过上限 | 一次最多 1000 个，缩小匹配范围或分成几次运行 |
| 某些 PDF 失败，原因是没有可提取的文字 | 多为扫描版；先自行 OCR 成文本或带文字层的 PDF |
| 很多行失败，原因是回答不符合结构 | 指令是否写清每个字段的含义和取值；结构是否过严 |
| 运行显示暂停 | 到了时间或 token 上限，或被手动停止；说“继续”即可续跑 |
| 运行很快以失败结束 | 连续 5 次服务错误会熔断，检查密钥、接口地址和网络后再续跑 |
| 长论文的中间部分没被读到 | 单个条目默认只送 60000 个字符，保留开头和结尾；可以要求调高 `max_chars` |

第一次使用，建议先拿 5 到 10 篇试跑：确认字段抽得对、判定问题的方向没写反，再对全部文献运行。

> 全部参数与实现细节见 [map_items 参考文档](../map-items.md)；与其他委派方式的关系见 [Agent delegation](../agent-delegation.md)。本文依据撰写时的项目实现整理，不同版本的提示文字可能略有差异。
