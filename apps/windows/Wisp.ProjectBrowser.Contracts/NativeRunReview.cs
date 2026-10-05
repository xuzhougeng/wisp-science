using System.Text.Json;
using System.Text.Json.Nodes;
namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativeRunWorkspaceEntry(string Path, string Kind, ulong SizeBytes, ulong? FileCount);
public sealed record NativeRunWorkspaceListing(NativeRunWorkspaceEntry[] Entries, bool Truncated);
public sealed record NativeRunReviewReply(string RunId, bool ReadOnly, bool Cleaned, NativeRunWorkspaceListing? Listing, int? Downloaded, bool Acknowledged, bool? ShouldPrompt = null);
public interface INativeRunReviewClient
{
    Task<NativeRunReviewReply> InvokeAsync(string project, string session, string runId, JsonObject operation, CancellationToken token = default);
}
public sealed class NativeRunReviewClient(INativeSettingsClient transport) : INativeRunReviewClient
{
    public async Task<NativeRunReviewReply> InvokeAsync(string project, string session, string runId, JsonObject operation, CancellationToken token = default)
    {
        var result = await transport.InvokeAsync("native_conversation_panel_run_review", new()
        { ["session_id"] = session, ["run_id"] = runId, ["operation"] = operation.DeepClone() }, project, token);
        var reply = result?.Deserialize<NativeRunReviewReply>(ConversationSnapshot.JsonOptions);
        if (reply == null || reply.RunId != runId || !reply.Acknowledged
            || operation["action"]?.GetValue<string>() == "check_prompt" && reply.ShouldPrompt == null
            || operation["action"]?.GetValue<string>() == "list" && reply.Listing?.Entries == null
            || operation["action"]?.GetValue<string>() == "download" && reply.Downloaded is not >= 0
            || operation["action"]?.GetValue<string>() == "cleanup" && !reply.Cleaned)
            throw new InvalidDataException("运行审阅响应未确认或身份不匹配。");
        return reply;
    }
}
