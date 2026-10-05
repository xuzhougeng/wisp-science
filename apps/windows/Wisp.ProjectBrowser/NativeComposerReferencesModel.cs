using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeComposerToken(int Start, int End, string Kind, string Query)
{
    public static NativeComposerToken? Find(string draft, int caret)
    {
        if (caret <= 0 || caret > draft.Length || (caret < draft.Length && char.IsHighSurrogate(draft[caret - 1]) && char.IsLowSurrogate(draft[caret]))) return null;
        var at = draft.LastIndexOfAny(['@', '#', '/'], Math.Max(0, caret - 1), caret);
        if (at < 0) return null;
        var trigger = draft[at];
        if (at > 0)
        {
            var previous = draft[at - 1];
            if (previous is >= 'a' and <= 'z' or >= 'A' and <= 'Z' or >= '0' and <= '9' or '_'
                || trigger == '/' && previous is ':' or '/' or '\\' or '.') return null;
        }
        var query = draft[(at + 1)..caret];
        if (query.Any(char.IsWhiteSpace)) return null;
        return new(at, caret, trigger == '@' ? "artifact" : trigger == '#' ? "session" : "skill", query);
    }
    public string RemoveFrom(string draft) => draft.Remove(Start, End - Start);
}

/// <summary>Read-only, session-scoped candidate search. Dismissal invalidates pending results.</summary>
public sealed class NativeComposerReferencesModel(INativeSettingsClient? client)
{
    public event Action? Changed;
    public NativeComposerToken? Token { get; private set; }
    public NativeReferenceOption[] Options { get; private set; } = [];
    public string? Error { get; private set; }
    public bool Loading { get; private set; }
    private string? project, session;
    private int generation;
    public void Bind(string? projectId, string? sessionId) { project = projectId; session = sessionId; Dismiss(); }
    public void Dismiss() { generation++; Token = null; Options = []; Error = null; Loading = false; Changed?.Invoke(); }
    public async Task SearchAsync(NativeComposerToken token, CancellationToken cancellationToken = default)
    {
        if (client == null || project is not { } p || session is not { } s) return;
        var request = ++generation; Token = token; Options = []; Error = null; Loading = true; Changed?.Invoke();
        try
        {
            await Task.Delay(120, cancellationToken);
            if (request != generation) return;
            var node = await client.InvokeAsync("native_conversation_references", new() {
                ["session_id"] = s, ["kind"] = token.Kind, ["query"] = token.Query }, p, cancellationToken);
            if (request != generation) return;
            var result = node?.Deserialize<NativeReferenceCatalog>(ConversationSnapshot.JsonOptions);
            if (result?.SessionId != s || result.Options is null || result.Options.Length > 120
                || result.Options.Any(option => option.Reference is null || !option.Reference.Valid || option.Label is null))
                throw new InvalidDataException("引用候选响应不匹配。");
            Options = result.Options.DistinctBy(option => option.Reference.Key).ToArray();
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (request == generation) Error = "未能读取引用候选：" + ex.Message; }
        finally { if (request == generation) { Loading = false; Changed?.Invoke(); } }
    }

    public static string Message(string draft, NativeReferenceOption[] references)
    {
        var message = draft.Trim();
        foreach (var group in references.GroupBy(option => option.Reference.Kind))
        {
            var label = group.Key switch { "artifact" => "Attached artifacts", "session" => "Attached sessions",
                "project" => "Project context", "skill" => "Selected skills", "workflow" => "Selected workflows",
                "context" => "Target environments", "runtime" => "Target runtimes", _ => throw new InvalidDataException("Invalid reference kind") };
            message += (message.Length > 0 ? "\n\n" : "") + label + ": " + string.Join(", ", group.Select(option => option.Label.Replace('\r', ' ').Replace('\n', ' ')));
        }
        return message;
    }
}
