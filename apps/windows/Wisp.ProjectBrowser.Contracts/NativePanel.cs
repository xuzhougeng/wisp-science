using System.Text.Json;
using System.Text.Json.Nodes;
namespace Wisp.ProjectBrowser.Contracts;
public sealed record NativePanelArtifact(string Id, string Name, string Kind, string Path, string? Location, long Ts, string? LogicalPath);
public sealed record NativeRunArtifact(string RunId, string ArtifactId, string Label, string Filename, string Kind, string ReaderSession);
public sealed record NativePanelFile(string Name, bool IsDir, ulong Size, ulong? ModifiedUnixMillis);
public sealed record NativePanelFileContent(string Path, string Mime, string? Text, string? Base64, bool Truncated, ulong? TotalBytes);
public sealed record NativePanelExport(string Path, string Name, ulong TotalBytes);
public sealed record NativeArtifactInput(string Path, bool ProducedHere);
public sealed record NativeArtifactPackage(string Name, string Version);
public sealed record NativeArtifactEnvironment(NativeArtifactPackage[] Packages);
public sealed record NativeArtifactProvenance(string Code, string Language, string Output, NativeArtifactInput[] Inputs, NativeArtifactEnvironment? Env);
public sealed record NativeArtifactProvenanceReply(string Schema, string ProjectId, string SessionId, string ArtifactId, string Path, NativeArtifactProvenance? Provenance);
public sealed record NativeFileProvenanceReply(string Schema, string ProjectId, string SessionId, string Path, NativeArtifactProvenance? Provenance);
public sealed record NativeArtifactLibraryReply(string Schema, string ProjectId, string SessionId, string ArtifactId, string? LibraryItemId);
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
    Task<NativeMethodDetail> MethodSearchAsync(string project, string? session, string context, string runId, string operation, CancellationToken token)
        => throw new NotSupportedException("Method-search review is unavailable from this host");
    Task<T> MethodOutputAsync<T>(string project, string? session, string context, string runId, string versionId, CancellationToken token) where T : class
        => throw new NotSupportedException("Method-search output is unavailable from this host");
    Task<NativePanelContexts> ContextsAsync(string project, string session, CancellationToken token = default);
    Task<string[]> SshAliasesAsync(string project, CancellationToken token = default) => Task.FromResult(Array.Empty<string>());
    Task<NativePanelContexts> ProjectContextsAsync(string project, CancellationToken token = default)
        => throw new NotSupportedException("Project environment listing is unavailable");
    Task<NativeRun[]> ContextRunsAsync(string project, string? session, string context, CancellationToken token = default)
        => throw new NotSupportedException("Environment runs are unavailable from this host");
    Task<NativeRunArtifact[]> ContextRunArtifactsAsync(string project, string? session, string context, CancellationToken token = default)
        => throw new NotSupportedException("Run artifacts are unavailable from this host");
    Task<NativeRun> ContextRunDetailAsync(string project, string? session, string context, string id, CancellationToken token = default)
        => throw new NotSupportedException("Environment run details are unavailable from this host");
    Task ApplyContextRunAsync(string project, string? session, string context, string id, JsonObject operation, CancellationToken token = default)
        => throw new NotSupportedException("Environment run controls are unavailable from this host");
    Task<NativeRunReviewReply> ReviewContextRunAsync(string project, string? session, string context, string id, JsonObject operation, CancellationToken token = default)
        => throw new NotSupportedException("Environment run review is unavailable from this host");
    Task<string[]> SetContextEnabledAsync(string project, string session, string contextId, bool enabled, CancellationToken token = default);
    Task SetDefaultContextAsync(string project, string session, string contextId, CancellationToken token = default)
        => throw new NotSupportedException("Default execution context is not supported by this client");
    Task ProbeContextAsync(string project, string contextId, CancellationToken token = default);
    Task<NativePanelArtifact[]> ArtifactsAsync(string project, string session, CancellationToken token = default);
    Task<NativePanelFile[]> FilesAsync(string project, string session, string path = ".", CancellationToken token = default);
    Task SaveFileAsync(string project, string? session, string path, string originalText, string text, CancellationToken token = default);
    Task FileActionAsync(string project, string? session, NativePanelFileAction action, string path, string? newPath = null, CancellationToken token = default);
    Task<NativePanelFileContent> ReadFileAsync(string project, string session, string path, CancellationToken token = default);
    Task<NativePanelFileContent> ReadArtifactAsync(string project, string session, string artifactId, CancellationToken token = default);
    Task<NativeArtifactProvenance?> ArtifactProvenanceAsync(string project, string session, string artifactId, CancellationToken token = default)
        => throw new NotSupportedException("Artifact provenance is not available from this host.");
    Task<NativeArtifactProvenance?> FileProvenanceAsync(string project, string session, string path, CancellationToken token = default)
        => throw new NotSupportedException("File provenance is not available from this host.");
    Task<NativePanelExport> ExportArtifactAsync(string project, string session, string artifactId, CancellationToken token = default)
        => throw new NotSupportedException("Artifact export is not available from this host.");
    Task<NativePanelExport> ExportFileAsync(string project, string session, string path, CancellationToken token = default)
        => throw new NotSupportedException("File export is not available from this host.");
    Task<string?> ArtifactLibraryAsync(string project, string session, string artifactId, bool? enabled = null, CancellationToken token = default)
        => throw new NotSupportedException("Artifact library is not available from this host.");
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
    public async Task<NativePanelContexts> ProjectContextsAsync(string project, CancellationToken token = default)
    {
        var snapshot = await new NativeRuntimesClient(transport).ReadAsync(project, null, token).ConfigureAwait(false);
        if (snapshot.ProjectId != project || snapshot.SessionId != null) throw new InvalidDataException("Runtime snapshot scope mismatch");
        return new(snapshot.Contexts, [], snapshot.ReadOnly);
    }
    public async Task<string[]> SshAliasesAsync(string project, CancellationToken token = default)
    {
        var hosts = await transport.InvokeAsync("list_ssh_hosts", new(), project, token).ConfigureAwait(false) as JsonArray
            ?? throw new InvalidDataException("Missing SSH host list");
        return hosts.Select(host => host?["alias"]?.GetValue<string>()).OfType<string>()
            .Where(alias => !string.IsNullOrWhiteSpace(alias)).ToArray();
    }
    private sealed record ContextRunsReply<T>(string ProjectId, string? SessionId, string ContextId, T Data);
    public async Task<NativeMethodDetail> MethodSearchAsync(string project, string? session, string context, string runId, string operation, CancellationToken token)
    {
        if (operation is not ("inspect" or "start" or "pause" or "resume" or "cancel")) throw new ArgumentException("Invalid method-search operation");
        var detail = await ContextRunRequest<NativeMethodDetail>(project, session, context, runId, token,
            new() { ["action"] = "method_search", ["operation"] = new JsonObject { ["kind"] = operation } }).ConfigureAwait(false);
        return detail.Verify(runId, context);
    }
    private sealed record MethodOutputReply<T>(string RunId, string VersionId, T Data);
    public async Task<T> MethodOutputAsync<T>(string project, string? session, string context, string runId, string versionId, CancellationToken token) where T : class
    {
        var kind = typeof(T) == typeof(NativePanelFileContent) ? "read_output" : typeof(T) == typeof(NativePanelExport) ? "export_output" : throw new ArgumentException("Invalid output operation");
        var reply = await ContextRunRequest<MethodOutputReply<T>>(project, session, context, runId, token,
            new() { ["action"] = "method_search", ["operation"] = new JsonObject { ["kind"] = kind, ["version_id"] = versionId } }).ConfigureAwait(false);
        if (reply.RunId != runId || reply.VersionId != versionId || reply.Data == null ||
            reply.Data is NativePanelExport source && (!Path.IsPathFullyQualified(source.Path) || string.IsNullOrWhiteSpace(source.Name) || source.Name != Path.GetFileName(source.Name)))
            throw new InvalidDataException("Unexpected method-search output version");
        return reply.Data;
    }
    private async Task<T> ContextRunRequest<T>(string project, string? session, string context, string? id, CancellationToken token, JsonObject? operation = null, string? readCommand = null) where T : class
    {
        var args = new JsonObject { ["session_id"] = session, ["context_id"] = context };
        if (id != null) args["run_id"] = id;
        if (operation != null) args["operation"] = operation.DeepClone();
        var response = await transport.InvokeAsync(readCommand ?? (operation != null ? "native_context_run_action" : id == null ? "native_context_runs_list" : "native_context_runs_detail"),
            args, project, token).ConfigureAwait(false);
        var reply = response?.Deserialize<ContextRunsReply<T>>(ConversationSnapshot.JsonOptions);
        if (reply?.Data == null || reply.ProjectId != project || reply.SessionId != session || reply.ContextId != context)
            throw new InvalidDataException("Environment run scope mismatch");
        return reply.Data;
    }
    public async Task<NativeRunArtifact[]> ContextRunArtifactsAsync(string project, string? session, string context, CancellationToken token = default)
    {
        var rows = await ContextRunRequest<NativeRunArtifact[]>(project, session, context, null, token,
            readCommand: "native_context_run_artifacts").ConfigureAwait(false);
        if (rows.Any(r => r == null || string.IsNullOrWhiteSpace(r.RunId) || string.IsNullOrWhiteSpace(r.ArtifactId)
            || string.IsNullOrWhiteSpace(r.ReaderSession) || (session != null && session != r.ReaderSession)
            || string.IsNullOrWhiteSpace(r.Filename) || r.Kind == null)
            || rows.DistinctBy(r => (r.RunId, r.ArtifactId)).Count() != rows.Length)
            throw new InvalidDataException("Invalid environment artifact links");
        return rows;
    }
    public async Task<NativeRun[]> ContextRunsAsync(string project, string? session, string context, CancellationToken token = default)
    {
        var rows = await ContextRunRequest<NativeRun[]>(project, session, context, null, token).ConfigureAwait(false);
        if (rows.Any(row => row == null || string.IsNullOrEmpty(row.Id) || row.ContextId != context) ||
            rows.DistinctBy(row => row.Id).Count() != rows.Length) throw new InvalidDataException("Invalid environment run list");
        return rows;
    }
    public async Task<NativeRun> ContextRunDetailAsync(string project, string? session, string context, string id, CancellationToken token = default)
    {
        var row = await ContextRunRequest<NativeRun>(project, session, context, id, token).ConfigureAwait(false);
        if (row.Id != id || row.ContextId != context) throw new InvalidDataException("Unexpected environment run");
        return row;
    }
    public async Task ApplyContextRunAsync(string project, string? session, string context, string id, JsonObject operation, CancellationToken token = default)
    {
        if (await ContextRunRequest<string>(project, session, context, id, token, operation).ConfigureAwait(false) != id)
            throw new InvalidDataException("Unexpected environment run acknowledgement");
    }
    public async Task<NativeRunReviewReply> ReviewContextRunAsync(string project, string? session, string context, string id, JsonObject operation, CancellationToken token = default)
    {
        var reply = await ContextRunRequest<NativeRunReviewReply>(project, session, context, id, token, new() { ["action"] = "review", ["operation"] = operation.DeepClone() }).ConfigureAwait(false);
        return NativeRunReviewClient.Validate(reply, id, operation);
    }
    public Task<string[]> SetContextEnabledAsync(string project, string session, string contextId, bool enabled, CancellationToken token = default) => Call<string[]>("context_enabled", project, session, new() { ["context_id"] = contextId, ["enabled"] = enabled }, token);
    public async Task ProbeContextAsync(string project, string contextId, CancellationToken token = default) => _ = await transport.InvokeAsync("probe_execution_context", new() { ["contextId"] = contextId }, project, token).ConfigureAwait(false);
    public Task<NativePanelArtifact[]> ArtifactsAsync(string project, string session, CancellationToken token = default) => Call<NativePanelArtifact[]>("artifacts", project, session, new(), token);
    public Task<NativePanelFile[]> FilesAsync(string project, string session, string path = ".", CancellationToken token = default) => Call<NativePanelFile[]>("files", project, session, new() { ["path"] = path }, token);
    public async Task SaveFileAsync(string project, string? session, string path, string originalText, string text, CancellationToken token = default)
    {
        var result = await transport.InvokeAsync(session == null ? "native_project_file_save" : "native_conversation_panel_savefile", new() { ["session_id"] = session, ["path"] = path, ["original_text"] = originalText, ["text"] = text }, project, token).ConfigureAwait(false);
        if (result?.GetValue<bool>() != true) throw new InvalidDataException("File save was not confirmed");
    }
    public async Task FileActionAsync(string project, string? session, NativePanelFileAction action, string path, string? newPath = null, CancellationToken token = default)
    {
        var name = action switch { NativePanelFileAction.CreateFile => "create_file", NativePanelFileAction.CreateDirectory => "create_directory", NativePanelFileAction.Rename => "rename", NativePanelFileAction.Delete => "delete", _ => throw new ArgumentOutOfRangeException(nameof(action)) };
        if (action == NativePanelFileAction.Rename && string.IsNullOrWhiteSpace(newPath)) throw new ArgumentException("Rename requires a destination", nameof(newPath));
        var result = await transport.InvokeAsync(session == null ? "native_project_file_action" : "native_conversation_panel_file_action", new() { ["session_id"] = session, ["file_action"] = name, ["path"] = path, ["new_path"] = newPath }, project, token).ConfigureAwait(false);
        if (result?.GetValue<bool>() != true) throw new InvalidDataException("File action was not confirmed");
    }
    public Task<NativePanelFileContent> ReadFileAsync(string project, string session, string path, CancellationToken token = default) => Call<NativePanelFileContent>("readfile", project, session, new() { ["path"] = path, ["render_pdf"] = true, ["render_office"] = true }, token);
    public Task<NativePanelFileContent> ReadArtifactAsync(string project, string session, string artifactId, CancellationToken token = default) => Call<NativePanelFileContent>("readartifact", project, session, new() { ["artifact_id"] = artifactId, ["render_pdf"] = true, ["render_office"] = true }, token);
    public async Task<NativeArtifactProvenance?> ArtifactProvenanceAsync(string project, string session, string artifactId, CancellationToken token = default)
    {
        var reply = await Call<NativeArtifactProvenanceReply>("artifact_provenance", project, session, new() { ["artifact_id"] = artifactId }, token);
        if (reply.Schema != "wisp.native-conversations.v1" || reply.ProjectId != project || reply.SessionId != session
            || reply.ArtifactId != artifactId || string.IsNullOrWhiteSpace(reply.Path))
            throw new InvalidDataException("Artifact provenance scope mismatch");
        return reply.Provenance;
    }
    public async Task<NativeArtifactProvenance?> FileProvenanceAsync(string project, string session, string path, CancellationToken token = default)
    {
        if (string.IsNullOrWhiteSpace(path)) throw new ArgumentException("Choose a file", nameof(path));
        var reply = await Call<NativeFileProvenanceReply>("file_provenance", project, session, new() { ["path"] = path }, token);
        if (reply.Schema != "wisp.native-conversations.v1" || reply.ProjectId != project || reply.SessionId != session || reply.Path != path)
            throw new InvalidDataException("File provenance scope mismatch");
        return reply.Provenance;
    }
    public async Task<NativePanelExport> ExportArtifactAsync(string project, string session, string artifactId, CancellationToken token = default)
    {
        var source = await Call<NativePanelExport>("export", project, session, new() { ["artifact_id"] = artifactId }, token);
        if (!Path.IsPathFullyQualified(source.Path) || string.IsNullOrWhiteSpace(source.Name))
            throw new InvalidDataException("Artifact export source is invalid");
        return source;
    }
    public async Task<NativePanelExport> ExportFileAsync(string project, string session, string path, CancellationToken token = default)
    {
        if (string.IsNullOrWhiteSpace(path)) throw new ArgumentException("Choose a file", nameof(path));
        var source = await Call<NativePanelExport>("export", project, session, new() { ["path"] = path }, token);
        if (!Path.IsPathFullyQualified(source.Path) || string.IsNullOrWhiteSpace(source.Name))
            throw new InvalidDataException("File export source is invalid");
        return source;
    }
    public async Task<string?> ArtifactLibraryAsync(string project, string session, string artifactId, bool? enabled = null, CancellationToken token = default)
    {
        var args = new JsonObject { ["artifact_id"] = artifactId };
        if (enabled.HasValue) args["enabled"] = enabled.Value;
        var reply = await Call<NativeArtifactLibraryReply>("artifact_library", project, session, args, token);
        if (reply.Schema != "wisp.native-conversations.v1" || reply.ProjectId != project || reply.SessionId != session || reply.ArtifactId != artifactId)
            throw new InvalidDataException("Artifact library scope mismatch");
        if (enabled.HasValue && enabled.Value != (reply.LibraryItemId != null))
            throw new InvalidDataException("Artifact library change was not confirmed; refresh before trying again.");
        return reply.LibraryItemId;
    }
}
