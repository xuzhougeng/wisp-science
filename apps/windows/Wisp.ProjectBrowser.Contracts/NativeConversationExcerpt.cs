using System.Text.Json;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativeSessionExcerpt(string SessionId, string Title, int StartUserIndex, int EndUserIndex,
    string Text, bool Truncated);
public sealed record NativeConversationExcerpt(string ProjectId, NativeTurnIdentity Start, NativeTurnIdentity End,
    NativeSessionExcerpt Excerpt);

public sealed class NativeConversationExcerptClient(INativeSettingsClient transport)
{
    public async Task<NativeSessionExcerpt> ReadAsync(NativeHistoryTarget first, NativeHistoryTarget last, CancellationToken token)
    {
        if (first.ProjectId != last.ProjectId || first.SessionId != last.SessionId || first.Turn.UserIndex > last.Turn.UserIndex)
            throw new InvalidDataException("Invalid excerpt range");
        var node = await transport.InvokeAsync("native_conversation_excerpt", new JsonObject {
            ["session_id"] = first.SessionId,
            ["start"] = JsonSerializer.SerializeToNode(first.Turn, ConversationSnapshot.JsonOptions),
            ["end"] = JsonSerializer.SerializeToNode(last.Turn, ConversationSnapshot.JsonOptions)
        }, first.ProjectId, token).ConfigureAwait(false);
        var value = node?.Deserialize<NativeConversationExcerpt>(ConversationSnapshot.JsonOptions);
        if (value == null || value.ProjectId != first.ProjectId || value.Start != first.Turn || value.End != last.Turn
            || value.Excerpt is not { } excerpt || excerpt.SessionId != first.SessionId
            || excerpt.StartUserIndex != first.Turn.UserIndex || excerpt.EndUserIndex != last.Turn.UserIndex
            || excerpt.Title == null || string.IsNullOrWhiteSpace(excerpt.Text) || excerpt.Text.Length > 128 * 1024)
            throw new InvalidDataException("Quoted conversation response identity mismatch");
        return value.Excerpt;
    }
}
