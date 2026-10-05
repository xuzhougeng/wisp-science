using System.Globalization;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public static class NativeToolPresentation
{
    public static bool IsFailure(ConversationItem item) => item.Ok == false || item.Status is "failed" or "error"
        || item.Run?.Status is "failed" or "timed_out" or "lost";
    public static bool RequiresAttention(ConversationItem item) => IsFailure(item) || item.Run?.NeedsReview == true;
    public static bool OwnsTerminalRun(ConversationItem item, int index) => item.Run is { } run
        && run.OwnerIndex == index && WorkspaceConversationModel.RunTerminal(run.Status);
    public static bool InitiallyExpanded(ConversationItem item, int index) => !OwnsTerminalRun(item, index) && RequiresAttention(item);
    public static string State(ConversationItem item) => item.Ok == false || item.Status is "failed" or "error"
        ? "失败"
        : item.Run is { } run ? RunState(run.Status) + (run.NeedsReview ? " · 待审阅" : "")
        : item.ToolName is "monitor_run" or "wisp_monitor_run" ? "运行状态未核实"
        : item.Status is "running" or "pending" or "started" or "in_progress" ? "执行中"
        : item.Status is "cancelled" or "canceled" ? "已取消"
        : item.Status is "interrupted" ? "已中断"
        : item.Ok == true && item.Status is null or "" or "completed" or "done" or "success" or "succeeded" ? "已完成"
        : string.IsNullOrEmpty(item.Text) && item.Status is null or "" ? "执行中"
        : "状态未知";

    public static string Heading(ConversationItem item) => State(item) +
        (item.DurationMs is { } duration ? " · " + (item.Run != null ? "工具耗时 " : "") + Duration(duration) : "");

    public static string RunState(string status) => status switch
    {
        "draft" => "运行待提交", "submitted" => "运行已提交", "running" => "运行中",
        "paused" => "运行已暂停", "cancelling" => "运行正在取消", "succeeded" => "运行已完成",
        "cancelled" => "运行已取消", "failed" => "运行失败", "timed_out" => "运行已超时",
        "lost" => "运行已失联", _ => "运行状态未知"
    };

    public static string Duration(ulong milliseconds) => milliseconds < 1000 ? $"{milliseconds} 毫秒"
        : milliseconds < 60000 ? (milliseconds / 1000d).ToString("0.#", CultureInfo.InvariantCulture) + " 秒"
        : $"{milliseconds / 60000} 分 {(milliseconds % 60000) / 1000} 秒";
}
