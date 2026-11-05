using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativeComposerReference(string Kind, string? Id = null, string? Name = null,
    string? ContextId = null, string? Language = null)
{
    public bool Valid => Kind switch
    {
        "file-path" => NativeFilePathRules.Relative(Id) || NativeFilePathRules.WindowsClipboardPath(Id),
        "artifact" or "session" or "project" or "workflow" or "context" => !string.IsNullOrWhiteSpace(Id),
        "skill" => !string.IsNullOrWhiteSpace(Name),
        "runtime" => !string.IsNullOrWhiteSpace(ContextId) && Language is "python" or "r",
        _ => false
    };
    public string Key => Kind + ":" + (Kind == "skill" ? Name : Kind == "runtime" ? ContextId + ":" + Language : Id);
    public JsonObject ToJson()
    {
        if (!Valid) throw new InvalidDataException("Invalid composer reference");
        if (Kind == "file-path") throw new InvalidOperationException("File paths are represented in message text, not structured host references");
        var value = new JsonObject { ["kind"] = Kind };
        if (Kind == "skill") value["name"] = Name;
        else if (Kind == "runtime") { value["context_id"] = ContextId; value["language"] = Language; }
        else value["id"] = Id;
        return value;
    }
}

public sealed record NativeReferenceOption(NativeComposerReference Reference, string Label, string Detail)
{
    public override string ToString() => Label + (string.IsNullOrWhiteSpace(Detail) ? "" : " · " + Detail);
}
public sealed record NativeReferenceCatalog(string SessionId, NativeReferenceOption[] Options);

/// <summary>Local drag identity only. Resolve labels and membership from the current sidebar.</summary>
public sealed record NativeSessionDrag(string ProjectId, string Kind, string Id)
{
    public const string Format = "application/x-wisp-native-session-reference";
    public bool Valid => !string.IsNullOrWhiteSpace(ProjectId) && !string.IsNullOrWhiteSpace(Id)
        && Kind is "session" or "folder";
}

public sealed record NativeSessionReferenceScope(string ProjectId, string ProjectName, IReadOnlyList<BrowserSession> Sessions);
