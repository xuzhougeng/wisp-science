using System.Text.Json;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeToolPresentationTests
{
    public static void Run()
    {
        var old = JsonSerializer.Deserialize<ConversationItem>("""
            {"role":"tool","text":"result","tool_name":"python","ok":true}
            """, ConversationSnapshot.JsonOptions)!;
        Check(old.DurationMs is null && NativeToolPresentation.Heading(old) == "已完成", "old snapshots do not invent tool duration");
        var timed = JsonSerializer.Deserialize<ConversationItem>("""
            {"role":"tool","text":"result","tool_name":"python","ok":true,"duration_ms":1200,"model_name":"model-a","timestamp":123}
            """, ConversationSnapshot.JsonOptions)!;
        Check(timed.ModelName == "model-a" && timed.Timestamp == 123 && NativeToolPresentation.Heading(timed) == "已完成 · 1.2 秒",
            "existing host metadata survives decoding and elapsed time appears once");
        foreach (var status in new[] { "failed", "error" })
            Check(NativeToolPresentation.IsFailure(timed with { Ok = null, Status = status }), "explicit error forces expanded presentation without an ok flag");
        Check(NativeToolPresentation.State(timed with { Status = "running" }) == "执行中", "running status cannot look completed because of a stale ok flag");
        Check(NativeToolPresentation.State(timed with { Status = "cancelled" }) == "已取消", "cancelled tools do not appear successful");
        Check(NativeToolPresentation.State(timed with { Status = "future_state" }) == "状态未知", "unknown statuses remain unknown");
        Check(NativeToolPresentation.Heading(old with { DurationMs = 0 }) == "已完成 · 0 毫秒", "recorded zero differs from absent duration");
        Check(NativeToolPresentation.Duration(61000) == "1 分 1 秒", "long operations have readable elapsed time");
        var monitor = timed with { ToolName = "monitor_run", Run = new("run-a", "running", 0, false) };
        Check(NativeToolPresentation.Heading(monitor) == "运行中 · 工具耗时 1.2 秒", "successful monitor call does not claim a running compute job is complete");
        Check(NativeToolPresentation.State(monitor with { Run = null }) == "运行状态未核实", "old hosts do not invent run completion from monitor success");
        Check(NativeToolPresentation.RequiresAttention(monitor with { Run = monitor.Run with { Status = "succeeded", NeedsReview = true } }), "review-needed runs retain an attention state after successful completion");
        foreach (var status in new[] { "failed", "timed_out", "lost" })
            Check(NativeToolPresentation.IsFailure(monitor with { Run = monitor.Run with { Status = status } }), "failed run retains failure styling despite a successful monitoring call");
        Check(NativeToolPresentation.State(monitor with { Run = monitor.Run with { Status = "cancelled" } }) == "运行已取消", "cancelled run is not a successful tool outcome");
        Check(NativeToolPresentation.State(monitor with { Run = monitor.Run with { Status = "timed_out" } }) == "运行已超时", "timeout remains distinct from a failed monitor call");
        var reviewed = monitor with { Run = monitor.Run with { Status = "succeeded", NeedsReview = true }, Text = "{\"command\":\"raw script\"}" };
        Check(!NativeToolPresentation.InitiallyExpanded(reviewed, 5) && NativeToolPresentation.Preview(reviewed) == ""
            && NativeToolPresentation.Heading(reviewed).Contains("待审阅"),
            "terminal monitor retains review status without exposing raw JSON even when it is not the owner row");
        Check(NativeToolPresentation.Preview(old with { Text = "{\"raw\":1}" }) == "", "unlinked JSON results are absent from headers");
        Check(NativeToolPresentation.Preview(old with { Text = "done\nnext\tline" }) == "done next line",
            "plain output previews normalize newlines");
        Check(NativeToolPresentation.Preview(old with { Text = new string('x', 10000) }).Length == 161,
            "long output cannot make the summary header arbitrarily tall");
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>> {
            ["en"] = new() { ["chat.step_lines"] = "{n} lines" }, ["zh"] = new() { ["chat.step_lines"] = "{n} 行" }
        }, "en");
        Check(NativeToolPresentation.State(timed, locale) == "Completed" && NativeToolPresentation.State(timed with { Status = "running" }, locale) == "Running", "English state labels retain explicit lifecycle precedence");
        Check(NativeToolPresentation.Heading(monitor, locale) == "Run running · Tool time 1s", "job state and tool elapsed stay distinct in English");
        Check(!NativeToolPresentation.Completed(monitor) && NativeToolPresentation.Running(monitor), "successful polling does not give a running job a completion tick");
        Check(!NativeToolPresentation.Completed(monitor with { Run = null }), "unverified monitor has no success tick");
        Check(!NativeToolPresentation.Completed(timed with { Status = "cancelled" }) && !NativeToolPresentation.Completed(timed with { Status = "future" }), "cancelled and unknown rows use neutral indicators");
        Check(NativeToolPresentation.Completed(timed with { Role = "acp_tool", Ok = null, Status = "completed" }), "completed ACP lifecycle works without a legacy ok field");
        var acp = timed with { Role = "acp_tool", Ok = null, Status = "completed", Text = "[{\"type\":\"content\"}]" };
        Check(NativeToolPresentation.State(acp, locale) == "Completed" && NativeToolPresentation.StepMeta(acp, locale) == "", "ACP completion label agrees with its icon and does not count JSON transport lines");
        Check(NativeToolPresentation.StepMeta(acp with { Status = "in_progress" }, locale) == "Running", "ACP active lifecycle remains visible");
        Check(!NativeToolPresentation.Running(timed with { Ok = null, Text = "", Run = new("r", "failed", 1, false) }), "empty failed Run cannot appear active");
        Check(NativeToolPresentation.CompactDuration(999) == "999ms" && NativeToolPresentation.CompactDuration(1999) == "1s"
            && NativeToolPresentation.CompactDuration(60000) == "1m" && NativeToolPresentation.CompactDuration(61000) == "1m 1s", "step durations use WebView's units and rounding");
        Check(NativeToolPresentation.StepMeta(timed with { Text = "line one\nline two\n" }, locale) == "1s · 2 lines", "settled output counts ignore a final newline");
        Check(NativeToolPresentation.StepMeta(timed with { Text = "\n\n", DurationMs = 0 }, locale) == "0ms · 2 lines", "blank lines and recorded zero remain factual");
        Check(NativeToolPresentation.StepMeta(timed with { Ok = false }, locale) == "Failed · 1s", "failed output omits success-style line count");
        Check(NativeToolPresentation.StepMeta(timed with { DurationMs = null, Text = "" }, locale) == "", "missing metadata is not fabricated");
        locale.Preview("zh");
        Check(NativeToolPresentation.StepMeta(timed, locale) == "1s · 1 行", "shared line-count localization follows locale changes");
        Check(NativeToolPresentation.StepLabel(timed with { ToolName = "mcp:papers/search", Input = "find evidence\nnext" }) == ("tool.badge.mcp", "papers/search", "find evidence"), "MCP prefix becomes the shared badge");
        Check(NativeToolPresentation.StepLabel(timed with { ToolName = "use_skill", Input = "  qc workflow  \nbody" }) == ("tool.badge.skill", "qc workflow", ""), "skill title replaces tool name without duplicating detail");
        var longDetail = NativeToolPresentation.StepLabel(timed with { Input = string.Concat(Enumerable.Repeat("🔬", 90)) }).Detail;
        Check(longDetail.Length == 160 && !char.IsHighSurrogate(longDetail[^1]), "input preview caps Unicode scalar values without breaking surrogate pairs");
        AcpLabels(acp);
        Console.WriteLine("Native tool status, elapsed time and backward-compatible metadata passed.");
    }
    private static void AcpLabels(ConversationItem acp)
    {
        Check(!NativeToolPresentation.InitiallyExpanded(acp with { Status = "failed", Ok = false }, 0)
            && NativeToolPresentation.InitiallyExpanded(acp with { Role = "tool", Status = "failed", Ok = false }, 0),
            "settled ACP steps default closed like WebView while ordinary tool attention behavior remains intact");
        Check(NativeToolPresentation.PreserveDisclosureChoice(acp with { Status = "failed", Ok = false })
            && !NativeToolPresentation.PreserveDisclosureChoice(acp with { Role = "tool", Status = "failed", Ok = false }),
            "manual ACP expansion survives failure refreshes without suppressing ordinary tool attention behavior");
        acp = acp with { ToolName = "mcp:agent-provided title", Input = "ignored native input", Kind = "read",
            Locations = " \r\n data/source.csv:12 \r\n data/other.csv:24 ", Text = " \n <script>literal output</script> \nsecond line\n" };
        Check(NativeToolPresentation.StepLabel(acp) == (null, "mcp:agent-provided title", "data/source.csv:12"),
            "ACP keeps its title literal and previews the first nonempty location before content or native input");
        Check(NativeToolPresentation.AcpBody(acp) == "data/source.csv:12 \r\n data/other.csv:24\n<script>literal output</script> \nsecond line",
            "ACP body joins trimmed location and output in source order without rendering markup or extra sections");
        Check(NativeToolPresentation.AcpDetail(acp with { Locations = " \r\n" }) == "<script>literal output</script>", "ACP falls back to the first content line");
        var stub = acp with { Locations = null, Text = " [{\"type\":\"terminal\",\"terminalId\":\"terminal-7\"}] " };
        Check(NativeToolPresentation.AcpTerminalStub(stub.Text) && NativeToolPresentation.AcpBody(stub) == ""
            && NativeToolPresentation.AcpDetail(stub) == "read", "terminal identity alone produces a kind label without an expandable empty body");
        Check(NativeToolPresentation.AcpBody(stub with { Locations = " source.py:4 " }) == "source.py:4"
            && NativeToolPresentation.AcpDetail(stub with { Locations = " source.py:4 " }) == "source.py:4", "terminal stubs retain real locations");
        Check(!NativeToolPresentation.AcpTerminalStub("[\n{\"terminalId\":\"t\"}\n]")
            && !NativeToolPresentation.AcpTerminalStub("{\"terminalId\":\"t\"}"), "only the compact array stub matches the WebView rule");
        Check(NativeToolPresentation.AcpBody(stub with { Text = "\n", Kind = null }) == ""
            && NativeToolPresentation.AcpDetail(stub with { Text = "\n", Kind = null }) == "", "empty ACP payload never invents content");
        var unicode = string.Concat(Enumerable.Repeat("🔬", 90));
        foreach (var row in new[] { acp with { Locations = unicode }, acp with { Locations = null, Text = unicode }, stub with { Kind = unicode } })
        {
            var detail = NativeToolPresentation.AcpDetail(row);
            Check(detail.Length == 160 && !char.IsHighSurrogate(detail[^1]), "ACP previews cap 80 Unicode scalar values for every source");
        }
        Check(NativeToolPresentation.AcpDetail(acp with { Locations = "[\n{\"path\":\"data.csv\"}\n]" }) == "[", "structured location transport stays literal like the WebView source");
    }
    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
}
