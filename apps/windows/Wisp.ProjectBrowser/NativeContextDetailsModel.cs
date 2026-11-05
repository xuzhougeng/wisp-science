using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Reads the existing native context-view contract. Only the six display sections
/// are displayed here; the scoped view also supplies the epoch summary. The
/// transcript toggle has its own lifetime and never depends on this panel cache.
public sealed class NativeContextDetailsModel(INativeSettingsClient? transport)
{
    private string? project, session, revision;
    private int generation;
    public event Action? Changed;
    public bool Busy { get; private set; }
    public bool Loaded { get; private set; }
    public bool Available => transport != null && session != null;
    public string? Error { get; private set; }
    public NativeModelContext? Context { get; private set; }
    public IReadOnlyDictionary<string, string> Sections { get; private set; } = new Dictionary<string, string>();

    public void Bind(ConversationSnapshot? snapshot, bool available = true)
    {
        if (!available || snapshot?.AcpAgentId != null || snapshot?.ModelId.StartsWith("acp:", StringComparison.Ordinal) == true) snapshot = null;
        var next = snapshot == null ? null : snapshot.Epoch + "/" + snapshot.ModelId + "/"
            + snapshot.Items.LastOrDefault(item => item.Role is "usage" or "compaction")?.Text;
        if (project == snapshot?.ProjectId && session == snapshot?.SessionId && revision == next) return;
        project = snapshot?.ProjectId; session = snapshot?.SessionId; revision = next;
        Invalidate();
    }

    public void Invalidate()
    {
        generation++; Busy = false; Loaded = false; Error = null;
        Sections = new Dictionary<string, string>(); Context = null; Changed?.Invoke();
    }

    public async Task LoadAsync(CancellationToken token = default)
    {
        if (!Available || Busy || Loaded || Error != null) return;
        var current = generation; var p = project!; var s = session!;
        Busy = true; Changed?.Invoke();
        try
        {
            var response = await transport!.InvokeAsync("native_conversation_context", new() { ["session_id"] = s }, p, token);
            if (current != generation || token.IsCancellationRequested) return;
            var sections = Decode(response, p, s);
            var context = response?["state"] != null && response["items"] != null ? NativeModelContext.Decode(response, p, s) : null;
            Sections = sections; Context = context; Loaded = true;
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (current == generation) Error = ex.Message; }
        finally { if (current == generation) { Busy = false; Changed?.Invoke(); } }
    }

    public async Task RetryAsync(CancellationToken token = default)
    {
        if (Busy) return;
        Invalidate(); await LoadAsync(token);
    }

    public static IReadOnlyDictionary<string, string> Decode(JsonNode? response, string project, string session)
    {
        static string Text(JsonNode? value) => value is JsonValue scalar && scalar.TryGetValue<string>(out var text)
            ? text : throw new InvalidDataException("Invalid context detail text");
        if (response is not JsonObject root || Text(root["project_id"]) != project || Text(root["session_id"]) != session
            || root["details"] is not JsonObject details) throw new InvalidDataException("Context detail scope mismatch");
        var sections = new Dictionary<string, string>();
        foreach (var key in new[] { "system_prompt", "rules", "skills" }) sections[key] = details[key] == null ? "" : Text(details[key]);
        foreach (var key in new[] { "tool_definitions", "mcp_dynamic_tools", "subagent_definitions" })
        {
            if (details[key] == null) { sections[key] = ""; continue; }
            if (details[key] is not JsonArray rows) throw new InvalidDataException("Invalid context tool details");
            sections[key] = string.Join("\n\n", rows.Select(row => row is JsonObject tool
                ? Text(tool["name"]) + "\n" + Text(tool["description"]) : throw new InvalidDataException("Invalid context tool detail")));
        }
        return sections;
    }
}
