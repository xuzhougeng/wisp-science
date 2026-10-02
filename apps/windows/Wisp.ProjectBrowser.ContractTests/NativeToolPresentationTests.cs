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
        Check(NativeToolPresentation.RequiresAttention(monitor with { Run = monitor.Run with { Status = "succeeded", NeedsReview = true } }), "review-needed runs expand even after successful completion");
        foreach (var status in new[] { "failed", "timed_out", "lost" })
            Check(NativeToolPresentation.IsFailure(monitor with { Run = monitor.Run with { Status = status } }), "failed run expands despite successful monitoring call");
        Check(NativeToolPresentation.State(monitor with { Run = monitor.Run with { Status = "cancelled" } }) == "运行已取消", "cancelled run is not a successful tool outcome");
        Check(NativeToolPresentation.State(monitor with { Run = monitor.Run with { Status = "timed_out" } }) == "运行已超时", "timeout remains distinct from a failed monitor call");
        Console.WriteLine("Native tool status, elapsed time and backward-compatible metadata passed.");
    }
    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
}
