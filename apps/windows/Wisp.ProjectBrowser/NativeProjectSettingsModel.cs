using System.Globalization;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// The dialog owns one explicit project; opening or saving never activates it.
public sealed class NativeProjectSettingsModel(INativeSettingsClient client, string projectId, bool inWorkspace,
    Func<CancellationToken, Task>? reconnect = null, NativeLocalization? localization = null) : WorkspaceActionModel
{
    public static readonly string[] RetentionKeys = ["run_retention_days", "failed_run_retention_days", "orphan_file_retention_days"];
    private static readonly string[] RetentionArguments = ["runRetentionDays", "failedRunRetentionDays", "orphanFileRetentionDays"];
    public bool InWorkspace { get; } = inWorkspace;
    public bool Loaded { get; private set; }
    public string Name { get; set; } = "";
    public string Description { get; set; } = "";
    public string AgentContext { get; set; } = "";
    private string originalContext = "";
    public bool FolderSync { get; private set; }
    public bool EnablingFolderSync { get; private set; }
    public string? FolderSyncError { get; private set; }
    public bool ConfirmationOpen { get; private set; }
    public bool RetentionReady { get; private set; }
    public bool RetentionInvalid { get; private set; }
    public bool RetentionSaving { get; private set; }
    public string[] Retention { get; private set; } = ["", "", ""];
    public bool CanSave => Loaded && !Busy && !EnablingFolderSync && !Closed && !string.IsNullOrWhiteSpace(Name);
    public bool ContextChanged => AgentContext.Trim().ReplaceLineEndings("\n") != originalContext.Trim().ReplaceLineEndings("\n");

    private readonly CancellationTokenSource lifetime = new();

    private async Task<JsonNode?> ReadAsync(string command, JsonObject args, bool reconnectFirst)
    {
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);
        deadline.CancelAfter(TimeSpan.FromSeconds(30));
        var token = deadline.Token;
        if (reconnectFirst && reconnect != null) await reconnect(token);
        token.ThrowIfCancellationRequested();
        try { return await client.InvokeAsync(command, args, projectId, token); }
        catch (Exception ex) when (!reconnectFirst && reconnect != null && !token.IsCancellationRequested
            && (ex is HttpRequestException { StatusCode: null } or FileNotFoundException or DirectoryNotFoundException))
        {
            // Only these two lookups are replayable. Never resend an edit or cleanup policy.
            await reconnect(token);
            token.ThrowIfCancellationRequested();
            return await client.InvokeAsync(command, args, projectId, token);
        }
    }

    public async Task<bool> LoadAsync(bool reconnectFirst = false)
    {
        var loaded = await RunAsync(async () =>
        {
            var value = await ReadAsync("get_project_settings", new() { ["id"] = projectId }, reconnectFirst);
            if (value is not JsonObject data || data["id"]?.GetValue<string>() != projectId)
                throw new InvalidDataException("The returned project does not match this dialog.");
            return data;
        }, data =>
        {
            Name = data["name"]?.GetValue<string>() ?? "";
            Description = data["description"]?.GetValue<string>() ?? "";
            AgentContext = originalContext = data["agent_context"]?.GetValue<string>() ?? "";
            FolderSync = data["folder_sync"]?.GetValue<bool>() == true; Loaded = true;
        }, ex => ex.Message);
        if (loaded && InWorkspace) await LoadRetentionAsync();
        return loaded;
    }
    public Task<bool> LoadRetentionAsync(bool reconnectFirst = false) => !InWorkspace || !Loaded || RetentionSaving || Closed ? Task.FromResult(false) : RunAsync(async () =>
    {
        var value = await ReadAsync("get_project_run_retention", new(), reconnectFirst);
        if (value is not JsonObject data || RetentionKeys.Any(key => !data.ContainsKey(key)))
            throw new InvalidDataException("Missing project retention settings.");
        return RetentionKeys.Select(key => data[key]?.ToJsonString() ?? "").ToArray();
    }, values => { Retention = values; RetentionReady = true; RetentionInvalid = false; }, ex => ex.Message);

    public async Task<bool> SaveAsync(bool confirmContext = false)
    {
        if (!CanSave) return false;
        if (ContextChanged && !(confirmContext && ConfirmationOpen))
        { ConfirmationOpen = true; Notify(); return false; }
        // Editing and undoing in a WinUI TextBox can normalize CRLF to LF.
        // Preserve the stored line endings if the text itself did not change.
        var context = AgentContext.ReplaceLineEndings("\n") == originalContext.ReplaceLineEndings("\n") ? originalContext : AgentContext;
        var args = new JsonObject { ["id"] = projectId, ["name"] = Name, ["description"] = Description, ["agentContext"] = context };
        ConfirmationOpen = false;
        return await RunAsync(() => client.InvokeAsync("update_project", args, projectId), _ => originalContext = context);
    }
    public void CancelConfirmation() { if (!Busy) { ConfirmationOpen = false; Notify(); } }
    protected override string UnconfirmedFailure(Exception exception)
    {
        if (exception is NativeSettingsCommandException) return exception.Message;
        const string zh = "操作结果未能确认；不会自动重试。";
        const string en = "The operation could not be confirmed; it will not be retried automatically.";
        return (localization?.Choose(zh, en) ?? zh) + "\n" + exception.Message;
    }
    public async Task<bool> EnableFolderSyncAsync()
    {
        if (InWorkspace || !Loaded || FolderSync || EnablingFolderSync || Busy || Closed) return false;
        // This command only uses the project ID. The metadata draft can still be edited
        // or dismissed while we await the host; only another submission must be blocked.
        EnablingFolderSync = true; FolderSyncError = null; Notify();
        try
        {
            await client.InvokeAsync("enable_project_folder_sync", new() { ["id"] = projectId }, projectId);
            if (Closed) return false;
            FolderSync = true; return true;
        }
        catch (Exception ex)
        {
            if (!Closed) FolderSyncError = UnconfirmedFailure(ex);
            return false;
        }
        finally { EnablingFolderSync = false; if (!Closed) Notify(); }
    }

    public static bool TryRetention(IReadOnlyList<string> values, out long?[] parsed)
    {
        parsed = new long?[3]; if (values.Count != 3) return false;
        for (var i = 0; i < 3; i++)
        {
            if (string.IsNullOrWhiteSpace(values[i])) continue;
            if (!long.TryParse(values[i].Trim(), NumberStyles.None, CultureInfo.InvariantCulture, out var days) || days is < 1 or > 3650) return false;
            parsed[i] = days;
        }
        return true;
    }
    public async Task<bool> SaveRetentionAsync(IReadOnlyList<string> values)
    {
        if (!InWorkspace || !RetentionReady || RetentionSaving || Busy || Closed) return false;
        RetentionInvalid = !TryRetention(values, out var parsed);
        if (RetentionInvalid) { Notify(); return false; }
        var normalized = parsed.Select(value => value?.ToString(CultureInfo.InvariantCulture) ?? "").ToArray();
        if (normalized.SequenceEqual(Retention)) { Notify(); return true; }
        var args = new JsonObject(); for (var i = 0; i < 3; i++) args[RetentionArguments[i]] = parsed[i];
        // Retention saves independently from the project form, as in WebView.
        // Disabling the whole form here interrupts Tab's destination during LostFocus.
        RetentionSaving = true; ClearFailure(); Notify();
        try
        {
            await client.InvokeAsync("set_project_run_retention", args, projectId);
            if (Closed) return false;
            Retention = normalized; return true;
        }
        catch (Exception ex)
        {
            if (!Closed) Fail(UnconfirmedFailure(ex));
            return false;
        }
        finally { RetentionSaving = false; if (!Closed) Notify(); }
    }
    public override void Dispose()
    {
        if (Closed) return;
        ConfirmationOpen = false; base.Dispose(); lifetime.Cancel(); lifetime.Dispose();
    }
}
