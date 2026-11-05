using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeHandoffLaunch(string ProjectId, string SessionId, string Epoch, string Note);

/// A read-only summary until the user explicitly starts its new conversation.
public sealed class NativeHandoffModel(INativeSettingsClient? transport)
{
    private string? project, session, epoch;
    private int generation;
    public event Action? Changed;
    public bool Available { get; private set; }
    public bool Visible { get; private set; }
    public bool Generating { get; private set; }
    public bool Starting { get; private set; }
    public bool Uncertain { get; private set; }
    public string Instruction { get; private set; } = "";
    public string Draft { get; private set; } = "";
    public string? Error { get; private set; }
    public bool CanStart => Available && Visible && !Generating && !Starting && !Uncertain && Draft.Trim().Length > 0;

    public void Bind(ConversationSnapshot? snapshot, bool available)
    {
        if (project != snapshot?.ProjectId || session != snapshot?.SessionId || epoch != snapshot?.Epoch) Reset();
        project = snapshot?.ProjectId; session = snapshot?.SessionId; epoch = snapshot?.Epoch;
        var next = transport != null && snapshot != null && available && !snapshot.ReadOnly && !snapshot.Running
            && snapshot.AcpAgentId == null && !snapshot.ModelId.StartsWith("acp:", StringComparison.Ordinal);
        if (Available != next) { Available = next; Changed?.Invoke(); }
    }
    private void Reset()
    {
        generation++; Visible = Generating = Starting = Uncertain = false;
        Instruction = Draft = ""; Error = null; Changed?.Invoke();
    }
    public void Close() { if (!Starting) Reset(); }
    public void SetInstruction(string text) { if (!Generating && !Starting) Instruction = text; }
    public void SetDraft(string text) { if (!Generating && !Starting) { Draft = text; Changed?.Invoke(); } }
    public async Task OpenAsync(CancellationToken token = default)
    {
        if (!Available || Visible) return;
        Reset(); Visible = true; await GenerateAsync(token);
    }
    public async Task GenerateAsync(CancellationToken token = default)
    {
        if (!Available || !Visible || Generating || Starting || Uncertain || token.IsCancellationRequested) return;
        var current = ++generation; var p = project!; var s = session!;
        Generating = true; Error = null; Changed?.Invoke();
        try
        {
            var instruction = Instruction.Trim();
            var response = await transport!.InvokeAsync("native_conversation_handoff",
                new() { ["session_id"] = s, ["instruction"] = instruction.Length == 0 ? null : instruction }, p, token);
            if (current != generation || token.IsCancellationRequested) return;
            Draft = Decode(response, p, s);
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (current == generation) Error = ex.Message; }
        finally { if (current == generation) { Generating = false; Changed?.Invoke(); } }
    }
    public async Task StartAsync(Func<NativeHandoffLaunch, Task> launch)
    {
        if (!CanStart) return;
        var current = generation; var request = new NativeHandoffLaunch(project!, session!, epoch!, Draft.Trim());
        Starting = true; Error = null; Changed?.Invoke();
        try { await launch(request); if (current == generation) { Starting = false; Close(); } }
        catch (Exception ex)
        {
            if (current == generation) { Starting = false; Uncertain = true; Error = ex.Message; Changed?.Invoke(); }
        }
    }
    public static string Decode(JsonNode? response, string project, string session)
    {
        if (response is not JsonObject root || root["project_id"]?.GetValue<string>() != project
            || root["session_id"]?.GetValue<string>() != session || root["note"] is not JsonValue note
            || !note.TryGetValue<string>(out var text) || string.IsNullOrWhiteSpace(text))
            throw new InvalidDataException("Invalid handoff summary or conversation scope");
        return text.Trim();
    }
}
