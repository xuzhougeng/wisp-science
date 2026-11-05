using System.Globalization;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public static class NativeToolPresentation
{
    public static bool IsFailure(ConversationItem item) => item.Ok == false || item.Status is "failed" or "error"
        || item.Run?.Status is "failed" or "timed_out" or "lost";
    public static bool RequiresAttention(ConversationItem item) => IsFailure(item) || item.Run?.NeedsReview == true;
    public static bool InitiallyExpanded(ConversationItem item, int index) =>
        item.Role != "acp_tool" && !(item.Run is { } run && WorkspaceConversationModel.RunTerminal(run.Status)) && RequiresAttention(item);
    public static bool PreserveDisclosureChoice(ConversationItem item) => item.Role == "acp_tool"
        || item.Run is { } run && WorkspaceConversationModel.RunTerminal(run.Status) || !RequiresAttention(item);

    /// <summary>Raw Run records belong in details, not the summary header.</summary>
    public static string Preview(ConversationItem item)
    {
        if (item.Run != null) return "";
        var text = item.Text.Trim();
        if (text.StartsWith('{') || text.StartsWith('[')) return "";
        var line = string.Join(" ", text.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries));
        if (line.Length <= 160) return line;
        var limit = char.IsHighSurrogate(line[159]) ? 159 : 160;
        return line[..limit] + "…";
    }
    private static string L(NativeLocalization? locale, string zh, string en) => locale?.Choose(zh, en) ?? zh;
    public static string State(ConversationItem item, NativeLocalization? locale = null) => item.Ok == false || item.Status is "failed" or "error"
        ? L(locale, "失败", "Failed")
        : item.Run is { } run ? RunState(run.Status, locale) + (run.NeedsReview ? L(locale, " · 待审阅", " · Needs review") : "")
        : item.ToolName is "monitor_run" or "wisp_monitor_run" ? L(locale, "运行状态未核实", "Run status unverified")
        : item.Status is "running" or "pending" or "started" or "in_progress" ? L(locale, "执行中", "Running")
        : item.Status is "cancelled" or "canceled" ? L(locale, "已取消", "Cancelled")
        : item.Status is "interrupted" ? L(locale, "已中断", "Interrupted")
        : item.Role == "acp_tool" && item.Status == "completed" ? L(locale, "已完成", "Completed")
        : item.Ok == true && item.Status is null or "" or "completed" or "done" or "success" or "succeeded" ? L(locale, "已完成", "Completed")
        : string.IsNullOrEmpty(item.Text) && item.Status is null or "" ? L(locale, "执行中", "Running")
        : L(locale, "状态未知", "Unknown status");

    public static string Heading(ConversationItem item, NativeLocalization? locale = null) => State(item, locale) +
        (item.DurationMs is { } duration ? " · " + (item.Run != null ? L(locale, "工具耗时 ", "Tool time ") : "") + Duration(duration, locale) : "");

    public static string RunState(string status, NativeLocalization? locale = null) => status switch
    {
        "draft" => L(locale, "运行待提交", "Run draft"), "submitted" => L(locale, "运行已提交", "Run submitted"), "running" => L(locale, "运行中", "Run running"),
        "paused" => L(locale, "运行已暂停", "Run paused"), "cancelling" => L(locale, "运行正在取消", "Run cancelling"), "succeeded" => L(locale, "运行已完成", "Run completed"),
        "cancelled" => L(locale, "运行已取消", "Run cancelled"), "failed" => L(locale, "运行失败", "Run failed"), "timed_out" => L(locale, "运行已超时", "Run timed out"),
        "lost" => L(locale, "运行已失联", "Run lost"), _ => L(locale, "运行状态未知", "Unknown run status")
    };

    public static string Duration(ulong milliseconds, NativeLocalization? locale = null) => locale?.Locale == "en" ? CompactDuration(milliseconds)
        : milliseconds < 1000 ? $"{milliseconds} 毫秒"
        : milliseconds < 60000 ? (milliseconds / 1000d).ToString("0.#", CultureInfo.InvariantCulture) + " 秒"
        : $"{milliseconds / 60000} 分 {(milliseconds % 60000) / 1000} 秒";

    // Same unit/rounding rules as WebView's format_duration_ms.
    public static string CompactDuration(ulong ms) => ms < 1000 ? $"{ms}ms" : ms < 60000 ? $"{ms / 1000}s"
        : ms % 60000 < 1000 ? $"{ms / 60000}m" : $"{ms / 60000}m {(ms % 60000) / 1000}s";

    public static bool Running(ConversationItem item) => !IsFailure(item) && (item.Run is { } run
        ? run.Status is "submitted" or "running" or "cancelling"
        : item.Status is "running" or "pending" or "started" or "in_progress"
            || item.Ok == null && item.Status is null or "" && string.IsNullOrEmpty(item.Text));
    public static bool Completed(ConversationItem item) => !IsFailure(item) && !Running(item)
        && (item.Run == null ? item.ToolName is not ("monitor_run" or "wisp_monitor_run") : item.Run.Status == "succeeded")
        && item.Status is null or "" or "completed" or "done" or "success" or "succeeded"
        && (item.Ok == true || item.Role == "acp_tool" && item.Status == "completed");

    public static (string? BadgeKey, string Title, string Detail) StepLabel(ConversationItem item)
    {
        // ACP titles are agent-provided labels, not native tool identifiers.
        if (item.Role == "acp_tool") return (null, item.ToolName ?? "ACP tool", AcpDetail(item));
        var name = item.ToolName ?? "tool";
        var first = (item.Input ?? "").Split('\n').FirstOrDefault()?.Trim() ?? "";
        var label = name.StartsWith("mcp:", StringComparison.Ordinal) ? ("tool.badge.mcp", name[4..])
            : name == "use_skill" ? ("tool.badge.skill", first.Length > 0 ? first : name) : ((string?)null, name);
        var detail = (item.Input ?? "").Split('\n').FirstOrDefault(line => !string.IsNullOrWhiteSpace(line))?.Trim() ?? "";
        detail = string.Concat(detail.EnumerateRunes().Take(80).Select(rune => rune.ToString()));
        return (label.Item1, label.Item2, detail == label.Item2 ? "" : detail);
    }

    // Mirrors chat_render.rs acp_tool_step_detail/body. Keep literal transport
    // text intact: interpreting JSON here would make the two frontends differ.
    public static bool AcpTerminalStub(string content)
    {
        var trimmed = content.Trim();
        return trimmed.StartsWith('[') && trimmed.Contains("\"terminalId\"", StringComparison.Ordinal) && !trimmed.Contains('\n');
    }
    public static string AcpDetail(ConversationItem item)
    {
        static string FirstLine(string value) => value.Split('\n').FirstOrDefault(line => !string.IsNullOrWhiteSpace(line))?.Trim() ?? "";
        var detail = FirstLine(item.Locations ?? "");
        if (detail.Length == 0) detail = AcpTerminalStub(item.Text) || string.IsNullOrWhiteSpace(item.Text)
            ? item.Kind ?? "" : FirstLine(item.Text);
        return string.Concat(detail.EnumerateRunes().Take(80).Select(rune => rune.ToString()));
    }
    public static string AcpBody(ConversationItem item)
    {
        var parts = new List<string>();
        if (!string.IsNullOrWhiteSpace(item.Locations)) parts.Add(item.Locations.Trim());
        if (!string.IsNullOrWhiteSpace(item.Text) && !AcpTerminalStub(item.Text)) parts.Add(item.Text.Trim());
        return string.Join("\n", parts);
    }
    public static string StepMeta(ConversationItem item, NativeLocalization locale)
    {
        if (item.Run != null || item.ToolName is "monitor_run" or "wisp_monitor_run") return Heading(item, locale);
        // ACP content is structured transport data, not a stdout line count.
        if (item.Role == "acp_tool") return Completed(item) ? "" : State(item, locale);
        var values = new List<string>();
        if (!Completed(item) && !Running(item)) values.Add(State(item, locale));
        if (item.DurationMs is { } duration) values.Add(CompactDuration(duration));
        if (Completed(item) && item.Text.Length > 0)
        {
            // Rust str::lines ignores one final newline but retains blank lines.
            var lines = item.Text.Count(c => c == '\n') + (item.Text.EndsWith('\n') ? 0 : 1);
            values.Add(locale.Format("chat.step_lines", ("n", lines.ToString(CultureInfo.InvariantCulture))));
        }
        return string.Join(" · ", values);
    }
}
