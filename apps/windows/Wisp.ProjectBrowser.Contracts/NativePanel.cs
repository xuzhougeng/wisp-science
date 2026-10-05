using System.Text.Json;
using System.Text.Json.Nodes;
namespace Wisp.ProjectBrowser.Contracts;
public sealed record NativePanelArtifact(string Id, string Name, string Kind, string Path, string? Location, long Ts, string? LogicalPath);
public sealed record NativePanelFile(string Name, bool IsDir, ulong Size, ulong? ModifiedUnixMillis);
public sealed record NativePanelFileContent(string Path, string Mime, string? Text, string? Base64, bool Truncated, ulong? TotalBytes);
public enum NativePanelFileAction { CreateFile, CreateDirectory, Rename, Delete }
public static class NativePanelPaths
{
    public static string Destination(string directory, string name)
    {
        var trimmed = name.Trim();
        if (trimmed.Length == 0 || trimmed is "." or ".." || name.Contains('/') || name.Contains('\\')
            || name.Any(char.IsControl))
            throw new InvalidOperationException("请输入有效名称，不含路径分隔符或控制字符。");
        return directory is "." or "" ? trimmed : directory.TrimEnd('/') + "/" + trimmed;
    }
}
public sealed record NativePanelContext(string Id, string Kind, string Label, string ConfigJson, string CapabilitiesJson, string? LastProbeStatus, string? LastProbeError);
public sealed record NativePanelDefaultContext(string? ContextId);
public sealed record NativePanelContexts(NativePanelContext[] Contexts, string[] EnabledIds, bool ReadOnly, NativePanelDefaultContext? DefaultContext = null)
{
    public NativePanelContext[] Attached => Contexts.Where(c => c.Kind == "local" || EnabledIds.Contains(c.Id)).ToArray();
    public NativePanelContext[] Available => Contexts.Where(c => c.Kind != "local" && !EnabledIds.Contains(c.Id)).ToArray();
}
public interface INativePanelClient
{
    Task<NativePanelContexts> ContextsAsync(string project, string session, CancellationToken token = default);
    Task<string[]> SetContextEnabledAsync(string project, string session, string contextId, bool enabled, CancellationToken token = default);
    Task SetDefaultContextAsync(string project, string session, string contextId, CancellationToken token = default)
        => throw new NotSupportedException("Default execution context is not supported by this client");
    Task ProbeContextAsync(string project, string contextId, CancellationToken token = default);
    Task<NativePanelArtifact[]> ArtifactsAsync(string project, string session, CancellationToken token = default);
    Task<NativePanelFile[]> FilesAsync(string project, string session, string path = ".", CancellationToken token = default);
    Task SaveFileAsync(string project, string session, string path, string originalText, string text, CancellationToken token = default);
    Task FileActionAsync(string project, string session, NativePanelFileAction action, string path, string? newPath = null, CancellationToken token = default);
    Task<NativePanelFileContent> ReadFileAsync(string project, string session, string path, CancellationToken token = default);
    Task<NativePanelFileContent> ReadArtifactAsync(string project, string session, string artifactId, CancellationToken token = default);
}
public sealed class NativePanelClient(INativeSettingsClient transport) : INativePanelClient
{
    public async Task SetDefaultContextAsync(string project, string session, string contextId, CancellationToken token = default)
    {
        if (string.IsNullOrWhiteSpace(contextId)) throw new ArgumentException("Choose an execution context", nameof(contextId));
        var result = await Call<NativePanelDefaultContext>("context_default", project, session, new() { ["context_id"] = contextId }, token);
        if (result.ContextId != contextId) throw new InvalidDataException("Default execution context was not confirmed; refresh before trying again");
    }
    private async Task<T> Call<T>(string action, string project, string session, JsonObject args, CancellationToken token) where T : class
    {
        args["session_id"] = session;
        return (await transport.InvokeAsync("native_conversation_panel_" + action, args, project, token).ConfigureAwait(false))?.Deserialize<T>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing panel response");
    }
    public Task<NativePanelContexts> ContextsAsync(string project, string session, CancellationToken token = default) => Call<NativePanelContexts>("contexts", project, session, new(), token);
    public Task<string[]> SetContextEnabledAsync(string project, string session, string contextId, bool enabled, CancellationToken token = default) => Call<string[]>("context_enabled", project, session, new() { ["context_id"] = contextId, ["enabled"] = enabled }, token);
    public async Task ProbeContextAsync(string project, string contextId, CancellationToken token = default) => _ = await transport.InvokeAsync("probe_execution_context", new() { ["contextId"] = contextId }, project, token).ConfigureAwait(false);
    public Task<NativePanelArtifact[]> ArtifactsAsync(string project, string session, CancellationToken token = default) => Call<NativePanelArtifact[]>("artifacts", project, session, new(), token);
    public Task<NativePanelFile[]> FilesAsync(string project, string session, string path = ".", CancellationToken token = default) => Call<NativePanelFile[]>("files", project, session, new() { ["path"] = path }, token);
    public async Task SaveFileAsync(string project, string session, string path, string originalText, string text, CancellationToken token = default)
    {
        var result = await transport.InvokeAsync("native_conversation_panel_savefile", new() { ["session_id"] = session, ["path"] = path, ["original_text"] = originalText, ["text"] = text }, project, token).ConfigureAwait(false);
        if (result?.GetValue<bool>() != true) throw new InvalidDataException("File save was not confirmed");
    }
    public async Task FileActionAsync(string project, string session, NativePanelFileAction action, string path, string? newPath = null, CancellationToken token = default)
    {
        var name = action switch { NativePanelFileAction.CreateFile => "create_file", NativePanelFileAction.CreateDirectory => "create_directory", NativePanelFileAction.Rename => "rename", NativePanelFileAction.Delete => "delete", _ => throw new ArgumentOutOfRangeException(nameof(action)) };
        if (action == NativePanelFileAction.Rename && string.IsNullOrWhiteSpace(newPath)) throw new ArgumentException("Rename requires a destination", nameof(newPath));
        var result = await transport.InvokeAsync("native_conversation_panel_file_action", new() { ["session_id"] = session, ["file_action"] = name, ["path"] = path, ["new_path"] = newPath }, project, token).ConfigureAwait(false);
        if (result?.GetValue<bool>() != true) throw new InvalidDataException("File action was not confirmed");
    }
    public Task<NativePanelFileContent> ReadFileAsync(string project, string session, string path, CancellationToken token = default) => Call<NativePanelFileContent>("readfile", project, session, new() { ["path"] = path, ["render_pdf"] = true, ["render_office"] = true }, token);
    public Task<NativePanelFileContent> ReadArtifactAsync(string project, string session, string artifactId, CancellationToken token = default) => Call<NativePanelFileContent>("readartifact", project, session, new() { ["artifact_id"] = artifactId, ["render_pdf"] = true, ["render_office"] = true }, token);
}
