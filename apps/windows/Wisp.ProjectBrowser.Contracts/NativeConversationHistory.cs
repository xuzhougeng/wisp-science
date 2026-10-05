using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativeTurnIdentity(int UserIndex, long UserSeq, string Digest);
public sealed record NativeHistoryState(string Revision, bool CanBranch, bool Reviewing, NativeTurnIdentity[] Turns);
public sealed record NativeHistoryTarget(string ProjectId, string SessionId, NativeTurnIdentity Turn, string Revision,
    string Role, string Draft);
public sealed record NativeMemoryOption(string Id, string Content);
public sealed record NativeTurnMemoryProposal(string SessionId, int TurnIndex, string Scope, string Content,
    string Trigger, int ToolCalls, int FailedToolCalls, double FailureRate, NativeMemoryOption[] GlobalMemories);
public sealed record NativeTurnUndoPreview(
    [property: JsonPropertyName("restoreFiles")] string[] RestoreFiles,
    [property: JsonPropertyName("removeFiles")] string[] RemoveFiles,
    [property: JsonPropertyName("removeArtifacts")] string[] RemoveArtifacts,
    [property: JsonPropertyName("unsupportedFiles")] string[] UnsupportedFiles,
    [property: JsonPropertyName("conflicts")] string[] Conflicts);

public sealed class NativeConversationHistoryClient(INativeSettingsClient transport)
{
    public async Task<JsonNode?> ActAsync(NativeHistoryTarget target, JsonObject action, CancellationToken token)
    {
        var result = await transport.InvokeAsync("native_conversation_history_action", new() {
            ["session_id"] = target.SessionId, ["target"] = JsonSerializer.SerializeToNode(target.Turn, ConversationSnapshot.JsonOptions),
            ["revision"] = target.Revision, ["action"] = action.DeepClone()
        }, target.ProjectId, token).ConfigureAwait(false);
        if (result?["session_id"]?.GetValue<string>() != target.SessionId
            || result?["target"]?.Deserialize<NativeTurnIdentity>(ConversationSnapshot.JsonOptions) != target.Turn)
            throw new InvalidDataException("Historical action response identity mismatch");
        return result["result"]?.DeepClone();
    }
}
