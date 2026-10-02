using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;
namespace Wisp.ProjectBrowser.Contracts;
public sealed record NativeRuntimeKey(
    [property: JsonPropertyName("projectId")] string ProjectId,
    [property: JsonPropertyName("contextId")] string ContextId,
    string Language,
    [property: JsonPropertyName("scopeKey")] string ScopeKey,
    [property: JsonPropertyName("sessionId")] string SessionId);
public sealed record NativeRuntimeInfo(
    [property: JsonPropertyName("runtimeId")] string RuntimeId,
    ulong Generation, NativeRuntimeKey Key, string Status, string? Interpreter, string? Version,
    [property: JsonPropertyName("processId")] uint? ProcessId,
    [property: JsonPropertyName("startedAtMs")] ulong StartedAtMs,
    [property: JsonPropertyName("lastActivityAtMs")] ulong LastActivityAtMs,
    [property: JsonPropertyName("residentMemoryBytes")] ulong? ResidentMemoryBytes,
    [property: JsonPropertyName("lastError")] string? LastError);
public sealed record NativeRuntimeObject(string Name, [property: JsonPropertyName("typeName")] string TypeName, string Summary,
    [property: JsonPropertyName("sizeBytes")] ulong? SizeBytes);
public sealed record NativeRuntimeObjects(NativeRuntimeObject[] Objects, [property: JsonPropertyName("totalCount")] int TotalCount);
public sealed record NativeRun(string Id, string? FrameId, string ContextId, string Title, string Kind, string Status,
    long CreatedAt, long? StartedAt, long? EndedAt, long? ExitCode, string? RemoteWorkdir, long? TimeoutSecs,
    long? LastPolledAt, string? LastPollError, string ProgressJson, long? HarvestedAt, long? CleanedAt,
    string? CleanupError, string? OutputFingerprint, string? Command, string? StdoutTail, string? StderrTail, string? EnvSnapshotJson);
public sealed record NativeContextActivity(NativeRuntimeInfo[] Runtimes, NativeRun[] Runs, bool ReadOnly, bool? RunReviewSupported = null);
public sealed record NativeRuntimeExecution(string Text, string[] Plots);
public interface INativeContextActivityClient
{
    Task<NativeRuntimeInfo> StartRuntimeAsync(string project, string session, string context, string language, CancellationToken token = default);
    Task<NativeRuntimeInfo?> StopRuntimeAsync(string project, string session, string runtimeId, ulong generation, CancellationToken token = default);
    Task<NativeRuntimeInfo> RestartRuntimeAsync(string project, string session, string runtimeId, ulong generation, CancellationToken token = default);
    Task DismissRuntimeAsync(string project, string session, string runtimeId, ulong generation, CancellationToken token = default);
    Task<NativeRuntimeExecution> ExecuteAsync(string project, string session, string context, string language, string code, CancellationToken token = default);
    Task<NativeContextActivity> ReadAsync(string project, string session, CancellationToken token = default);
    Task<NativeRun> ReadRunAsync(string project, string session, string runId, CancellationToken token = default);
    Task<NativeRuntimeObjects> InspectRuntimeAsync(string project, string session, string runtimeId, CancellationToken token = default);
    Task<NativeRun> CancelRunAsync(string project, string session, string runId, CancellationToken token = default);
    Task<NativeRun> HarvestRunAsync(string project, string session, string runId, CancellationToken token = default);
}
/// Uses explicit frame scope. Mutations are never replayed after uncertain replies.
public sealed class NativeContextActivityClient(INativeSettingsClient transport) : INativeContextActivityClient
{
    private async Task<T> Call<T>(string action, string project, string session, JsonObject args, CancellationToken token) where T : class
    {
        args["session_id"] = session;
        return (await transport.InvokeAsync("native_conversation_panel_" + action, args, project, token).ConfigureAwait(false))?.Deserialize<T>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing activity response");
    }
    public Task<NativeRuntimeInfo> StartRuntimeAsync(string project, string session, string context, string language, CancellationToken token = default) => Call<NativeRuntimeInfo>("runtime_start", project, session, new() { ["context_id"] = context, ["language"] = language }, token);
    public async Task<NativeRuntimeInfo?> StopRuntimeAsync(string project, string session, string runtimeId, ulong generation, CancellationToken token = default) =>
        (await transport.InvokeAsync("native_conversation_panel_runtime_stop", new() { ["session_id"] = session, ["runtime_id"] = runtimeId, ["runtime_generation"] = generation }, project, token).ConfigureAwait(false))?.Deserialize<NativeRuntimeInfo>(ConversationSnapshot.JsonOptions);
    public Task<NativeRuntimeInfo> RestartRuntimeAsync(string project, string session, string runtimeId, ulong generation, CancellationToken token = default) => Call<NativeRuntimeInfo>("runtime_restart", project, session, new() { ["runtime_id"] = runtimeId, ["runtime_generation"] = generation }, token);
    public async Task DismissRuntimeAsync(string project, string session, string runtimeId, ulong generation, CancellationToken token = default) =>
        _ = await transport.InvokeAsync("native_conversation_panel_runtime_dismiss", new() { ["session_id"] = session, ["runtime_id"] = runtimeId, ["runtime_generation"] = generation }, project, token).ConfigureAwait(false);
    public Task<NativeRuntimeExecution> ExecuteAsync(string project, string session, string context, string language, string code, CancellationToken token = default) => Call<NativeRuntimeExecution>("runtime_execute", project, session, new() { ["context_id"] = context, ["language"] = language, ["code"] = code }, token);
    public Task<NativeContextActivity> ReadAsync(string project, string session, CancellationToken token = default) => Call<NativeContextActivity>("activity", project, session, new(), token);
    public Task<NativeRun> ReadRunAsync(string project, string session, string runId, CancellationToken token = default) => Call<NativeRun>("run_detail", project, session, new() { ["run_id"] = runId }, token);
    public Task<NativeRuntimeObjects> InspectRuntimeAsync(string project, string session, string runtimeId, CancellationToken token = default) => Call<NativeRuntimeObjects>("runtime_inspect", project, session, new() { ["runtime_id"] = runtimeId }, token);
    public Task<NativeRun> CancelRunAsync(string project, string session, string runId, CancellationToken token = default) => Call<NativeRun>("run_cancel", project, session, new() { ["run_id"] = runId }, token);
    public Task<NativeRun> HarvestRunAsync(string project, string session, string runId, CancellationToken token = default) => Call<NativeRun>("run_harvest", project, session, new() { ["run_id"] = runId }, token);
}
