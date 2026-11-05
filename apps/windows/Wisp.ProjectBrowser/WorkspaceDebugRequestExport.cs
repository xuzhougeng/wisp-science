using System.Text;
using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Capture once, then save exactly those UTF-8 bytes after the user
/// chooses a file. Navigation, cancellation and close suppress late writes.</summary>
public sealed class WorkspaceDebugRequestExport(INativeSettingsClient client, BrowserSession source) : WorkspaceActionModel
{
    public string? SavedPath { get; private set; }
    public bool WriteUnconfirmed { get; private set; }
    protected override string UnconfirmedFailure(Exception exception) => exception.Message;
    public Task<bool> ExportAsync(Func<string, Task<string?>> pick, Func<string, byte[], Task> write, Func<bool> isCurrent)
    {
        if (Closed || Busy || WriteUnconfirmed || SavedPath != null) return Task.FromResult(false);
        return RunAsync(async () =>
        {
            var file = (await client.InvokeAsync("native_conversation_debug_request", new() { ["session_id"] = source.Id }, source.ProjectId))
                ?.Deserialize<NativeDebugRequestFile>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing debug request export");
            Validate(file);
            if (Closed || !isCurrent()) return null;
            var destination = await pick(file.Filename);
            if (Closed || !isCurrent() || string.IsNullOrEmpty(destination)) return null;
            if (!Path.IsPathFullyQualified(destination) || !destination.EndsWith(".json", StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("Choose an absolute JSON file destination");
            try { await write(destination, Encoding.UTF8.GetBytes(file.Contents)); }
            catch { WriteUnconfirmed = true; throw; }
            return destination;
        }, path => SavedPath = path);
    }
    private void Validate(NativeDebugRequestFile file)
    {
        if (file.Schema != NativeDebugRequestFile.SchemaName || file.ProjectId != source.ProjectId || file.SessionId != source.Id
            || string.IsNullOrWhiteSpace(file.Filename) || Path.GetFileName(file.Filename) != file.Filename
            || !file.Filename.StartsWith("wisp-debug-request-", StringComparison.Ordinal) || !file.Filename.EndsWith(".json", StringComparison.OrdinalIgnoreCase)
            || string.IsNullOrEmpty(file.Contents)) throw new InvalidDataException("Debug request export does not match the selected conversation");
        using var json = JsonDocument.Parse(file.Contents);
        var root = json.RootElement;
        if (root.ValueKind != JsonValueKind.Object || !root.TryGetProperty("session_id", out var id) || id.GetString() != source.Id
            || !root.TryGetProperty("source", out var origin) || origin.GetString() is not ("live-agent" or "stored-messages")
            || !root.TryGetProperty("message_count", out var count) || !count.TryGetInt64(out var total) || total <= 0
            || !root.TryGetProperty("messages", out var messages) || messages.ValueKind != JsonValueKind.Array || messages.GetArrayLength() != total)
            throw new InvalidDataException("Incomplete debug request snapshot");
    }
}
