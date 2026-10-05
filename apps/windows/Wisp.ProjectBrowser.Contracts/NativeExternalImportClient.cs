using System.Text.Json;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record ExternalImportSource(string Id, string Label, string Kind);
public sealed record ExternalImportSources(string Schema, string ProjectId, ExternalImportSource[] Sources);
public sealed record ExternalImportItem(string Path, string SessionId, string Title, string Cwd, int MessageCount, long LastActiveAt, string State);
public sealed record ExternalImportList(string Schema, string ProjectId, string Provider, string ContextId, ExternalImportItem[] Items);
public sealed record ExternalImportPreview(string Schema, string ProjectId, string Provider, string ContextId, string Path,
    string SourceSessionId, string Sha256, int MessageCount, ArchivePreviewLine[] Messages, string? ExistingSessionId);
public sealed record ExternalImportResult(string Schema, string ProjectId, string Provider, string ContextId, string Path,
    string SourceSessionId, string FrameId, string Status, int MessageCount);

public interface INativeExternalImportClient
{
    Task<ExternalImportSources> SourcesAsync(string project, CancellationToken cancellationToken = default);
    Task<ExternalImportList> ListAsync(string project, string provider, string context, bool refresh, CancellationToken cancellationToken = default);
    Task<ExternalImportPreview> PreviewAsync(string project, string provider, string context, ExternalImportItem item, CancellationToken cancellationToken = default);
    Task<ExternalImportResult> ImportAsync(ExternalImportPreview reviewed);
}

public sealed class NativeExternalImportClient(INativeSettingsClient transport) : INativeExternalImportClient
{
    public const string Schema = NativeSessionImportClient.Schema;
    private static bool Missing(string? value) => string.IsNullOrWhiteSpace(value);
    private static bool SourceValid(string? id) => id == "local" || (id?.StartsWith("ssh:") == true || id?.StartsWith("wsl:") == true) && id.Length > 4 && !id.Any(char.IsControl);
    private static void Scope(string schema, string project, string provider, string context, string expectedProject, string expectedProvider, string expectedContext)
    {
        if (schema != Schema || project != expectedProject || provider != expectedProvider || context != expectedContext
            || Missing(project) || provider is not ("codex" or "claude") || !SourceValid(context))
            throw new InvalidDataException("External conversation response belongs to another destination or source.");
    }
    public static ExternalImportSources DecodeSources(JsonNode? node, string project)
    {
        var value = node?.Deserialize<ExternalImportSources>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing import sources");
        if (value.Schema != Schema || value.ProjectId != project || value.Sources == null
            || value.Sources.Any(s => s == null || !SourceValid(s.Id) || Missing(s.Label) || s.Kind is not ("local" or "ssh" or "wsl")
                || (s.Kind == "local" ? s.Id != "local" : !s.Id.StartsWith(s.Kind + ":", StringComparison.Ordinal)))
            || value.Sources.Select(s => s.Id).Distinct().Count() != value.Sources.Length || !value.Sources.Any(s => s.Id == "local"))
            throw new InvalidDataException("Invalid external conversation sources");
        return value;
    }
    public static ExternalImportList DecodeList(JsonNode? node, string project, string provider, string context)
    {
        var value = node?.Deserialize<ExternalImportList>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing external conversations");
        Scope(value.Schema, value.ProjectId, value.Provider, value.ContextId, project, provider, context);
        if (value.Items == null || value.Items.Length > 500 || value.Items.Any(i => i == null || Missing(i.Path) || Missing(i.SessionId)
            || i.Title == null || i.Cwd == null || i.MessageCount < 0 || i.State is not ("new" or "imported" or "updatable"))
            || value.Items.Select(i => i.Path).Distinct().Count() != value.Items.Length)
            throw new InvalidDataException("Invalid external conversation list");
        return value;
    }
    public static ExternalImportPreview DecodePreview(JsonNode? node, string project, string provider, string context, ExternalImportItem item)
    {
        var value = node?.Deserialize<ExternalImportPreview>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing external conversation preview");
        Scope(value.Schema, value.ProjectId, value.Provider, value.ContextId, project, provider, context);
        if (value.Path != item.Path || value.SourceSessionId != item.SessionId || value.Sha256?.Length != 64 || !value.Sha256.All(char.IsAsciiHexDigit)
            || value.MessageCount <= 0 || value.Messages == null || value.Messages.Length > 4
            || value.Messages.Any(m => m == null || m.Role is not ("user" or "assistant") || m.Text == null)
            || (value.ExistingSessionId != null && Missing(value.ExistingSessionId)))
            throw new InvalidDataException("The conversation source changed; refresh its list before previewing.");
        return value;
    }
    public static ExternalImportResult DecodeResult(JsonNode? node, ExternalImportPreview reviewed)
    {
        var value = node?.Deserialize<ExternalImportResult>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing external import result");
        Scope(value.Schema, value.ProjectId, value.Provider, value.ContextId, reviewed.ProjectId, reviewed.Provider, reviewed.ContextId);
        if (value.Path != reviewed.Path || value.SourceSessionId != reviewed.SourceSessionId || Missing(value.FrameId)
            || value.Status is not ("imported" or "updated" or "skipped") || value.MessageCount != reviewed.MessageCount
            || (reviewed.ExistingSessionId != null && reviewed.ExistingSessionId != value.FrameId))
            throw new InvalidDataException("External import result does not match the reviewed conversation.");
        return value;
    }
    public async Task<ExternalImportSources> SourcesAsync(string project, CancellationToken cancellationToken = default) =>
        DecodeSources(await transport.InvokeAsync("native_external_session_sources", new(), project, cancellationToken), project);
    public async Task<ExternalImportList> ListAsync(string project, string provider, string context, bool refresh, CancellationToken cancellationToken = default) =>
        DecodeList(await transport.InvokeAsync("native_external_session_list", new() { ["provider"] = provider, ["context_id"] = context, ["refresh"] = refresh }, project, cancellationToken), project, provider, context);
    public async Task<ExternalImportPreview> PreviewAsync(string project, string provider, string context, ExternalImportItem item, CancellationToken cancellationToken = default) =>
        DecodePreview(await transport.InvokeAsync("native_external_session_preview", new() { ["provider"] = provider, ["context_id"] = context, ["path"] = item.Path }, project, cancellationToken), project, provider, context, item);
    public async Task<ExternalImportResult> ImportAsync(ExternalImportPreview reviewed) =>
        DecodeResult(await transport.InvokeAsync("native_external_session_import", new() { ["provider"] = reviewed.Provider, ["context_id"] = reviewed.ContextId,
            ["path"] = reviewed.Path, ["source_session_id"] = reviewed.SourceSessionId, ["sha256"] = reviewed.Sha256 }, reviewed.ProjectId), reviewed);
}
