using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public sealed partial class NativeSettingsEditorModel
{
    public JsonObject[] Environments => NativeEnvironments.Rows(Values["list_execution_contexts"]);
    public JsonObject[] EnvironmentHosts => NativeEnvironments.Rows(Values["list_ssh_hosts"]);
    public JsonObject[] EnvironmentTrust => NativeEnvironments.Rows(Values["list_ssh_trust_edges"]);
    public string? DefaultEnvironment => Values["get_default_execution_context"]?.GetValue<string>();
    public string? ProbingEnvironment { get; private set; }
    public string? EnvironmentNoticeKey { get; private set; }
    public string? EnvironmentCleanupError { get; private set; }
    public string? EnvironmentEditor { get; private set; }
    public JsonObject? EnvironmentDraft { get; private set; }
    public JsonObject? EnvironmentTarget { get; private set; }
    public bool EditingEnvironmentHost { get; private set; }
    public bool EnvironmentTestPassed { get; private set; }
    public bool EnvironmentTesting { get; private set; }
    public bool CanSaveEnvironment => !Busy && !Closed && EnvironmentDraft != null && (EnvironmentEditor == "runtime" || EnvironmentEditor == "host" && NativeEnvironments.CanSaveHost(EnvironmentDraft));
    public bool CanTestEnvironmentHost => !Busy && !Closed && EnvironmentEditor == "host" && EnvironmentDraft != null
        && !string.IsNullOrWhiteSpace(NativeEnvironments.Text(EnvironmentDraft, "alias")) && NativeEnvironments.ValidPort(EnvironmentDraft);
    private bool HasEnvironment(string id) => Environments.Any(row => NativeEnvironments.Text(row, "id") == id);
    private static JsonArray EnvironmentArray(JsonNode? value) => value as JsonArray ?? throw new InvalidDataException("Missing environment list acknowledgement");
    private void AcceptEnvironment(JsonNode? value)
    {
        if (value is not JsonObject updated || string.IsNullOrEmpty(NativeEnvironments.Text(updated, "id"))) throw new InvalidDataException("Missing environment acknowledgement");
        var rows = (JsonArray)Values["list_execution_contexts"]!;
        var existing = Environments.FirstOrDefault(row => NativeEnvironments.Text(row, "id") == NativeEnvironments.Text(updated, "id"));
        if (existing == null) rows.Add(updated.DeepClone()); else rows[rows.IndexOf(existing)] = updated.DeepClone();
    }
    public Task<bool> SetDefaultEnvironmentAsync(string? id)
    {
        id = string.IsNullOrWhiteSpace(id) ? null : id;
        if (Busy || Closed || !Values.ContainsKey("get_default_execution_context") || id != null && !NativeEnvironments.RemoteOptions(Environments).Any(row => row.Id == id)) return Task.FromResult(false);
        if (id == DefaultEnvironment) return Task.FromResult(true);
        return RunAsync(() => client.InvokeAsync("set_default_execution_context", new() { ["contextId"] = id }, projectId), value => Values["get_default_execution_context"] = value?.DeepClone());
    }
    public async Task<bool> ProbeEnvironmentAsync(string id)
    {
        if (Busy || Closed || !HasEnvironment(id)) return false;
        ProbingEnvironment = id; EnvironmentNoticeKey = null;
        try
        {
            var success = await RunAsync(() => client.InvokeAsync("probe_execution_context", new() { ["contextId"] = id }, projectId), value =>
            {
                AcceptEnvironment(value);
                if (NativeEnvironments.Text(value, "last_probe_status") != "ok")
                    throw new IOException(NativeEnvironments.Text(value, "last_probe_error") is { Length: > 0 } detail ? detail : "Probe failed");
                JsonObject? capabilities = null;
                try { capabilities = JsonNode.Parse(NativeEnvironments.Text(value, "capabilities_json")) as JsonObject; } catch (System.Text.Json.JsonException) { }
                EnvironmentNoticeKey = capabilities != null && new[] { "os", "arch", "hostname" }.Any(key => string.IsNullOrEmpty(NativeEnvironments.Text(capabilities, key)))
                    ? "contexts.probe_success_partial" : "contexts.probe_success";
            });
            if (!success && !Closed && id.StartsWith("ssh:", StringComparison.Ordinal))
            {
                var context = Environments.First(row => NativeEnvironments.Text(row, "id") == id);
                EnvironmentTarget = (JsonObject)context.DeepClone();
                EnvironmentTarget["failure"] = Error;
                EnvironmentEditor = "diagnosis";
            }
            return success;
        }
        finally { ProbingEnvironment = null; if (!Closed) Notify(); }
    }
    public Task<bool> ImportEnvironmentsAsync(bool wsl)
    {
        if (Busy || Closed) return Task.FromResult(false);
        return RunAsync(() => client.InvokeAsync(wsl ? "import_wsl_contexts" : "import_ssh_config_hosts", new(), projectId), value =>
            Values[wsl ? "list_execution_contexts" : "list_ssh_hosts"] = EnvironmentArray(value).DeepClone());
    }
    public Task<bool> RefreshEnvironmentContextsAsync() => RunAsync(() => client.InvokeAsync("list_execution_contexts", new(), projectId), value => Values["list_execution_contexts"] = EnvironmentArray(value).DeepClone());
    public bool OpenEnvironmentHost(string? alias = null)
    {
        if (Busy || Closed || EnvironmentEditor != null) return false;
        var host = alias == null ? null : EnvironmentHosts.FirstOrDefault(row => NativeEnvironments.Text(row, "alias") == alias);
        if (alias != null && host == null) return false;
        EditingEnvironmentHost = alias != null; EnvironmentTarget = host == null ? null : (JsonObject)host.DeepClone();
        EnvironmentDraft = host == null ? new JsonObject { ["alias"] = "", ["port"] = "", ["auth_method"] = "key" } : (JsonObject)host.DeepClone();
        EnvironmentDraft["port"] = NativeEnvironments.Text(host, "port"); EnvironmentDraft["password"] = "";
        EnvironmentDraft["auth_method"] = NativeEnvironments.Text(host, "auth_method") == "password" ? "password" : "key";
        EnvironmentEditor = "host"; EnvironmentTestPassed = false; EnvironmentNoticeKey = null; ClearFailure(); Notify(); return true;
    }
    public bool OpenEnvironmentRuntime(string id)
    {
        if (Busy || Closed || EnvironmentEditor != null) return false;
        var context = Environments.FirstOrDefault(row => NativeEnvironments.Text(row, "id") == id); if (context == null) return false;
        var interpreters = NativeRuntimeInterpreterValues.From(NativeEnvironments.Text(context, "config_json"), NativeEnvironments.Text(context, "capabilities_json"));
        EnvironmentTarget = (JsonObject)context.DeepClone();
        EnvironmentDraft = new() { ["python_executable"] = interpreters.Python, ["rscript_executable"] = interpreters.Rscript };
        EnvironmentEditor = "runtime"; EnvironmentNoticeKey = null; ClearFailure(); Notify(); return true;
    }
    private void ClearEnvironmentEditor()
    {
        EnvironmentEditor = null; EnvironmentDraft?.Clear(); EnvironmentDraft = null; EnvironmentTarget = null; EnvironmentTestPassed = false; EditingEnvironmentHost = false;
    }
    public bool CancelEnvironmentEditor()
    {
        if (Busy || Closed || EnvironmentEditor == null) return false;
        ClearEnvironmentEditor(); ClearFailure(); Notify(); return true;
    }
    public void SetEnvironmentField(string key, string value)
    {
        if (Busy || Closed || EnvironmentDraft == null || EditingEnvironmentHost && key == "alias") return;
        EnvironmentDraft[key] = value; EnvironmentTestPassed = false; Notify();
    }
    public async Task<bool> TestEnvironmentHostAsync()
    {
        if (!CanTestEnvironmentHost) return false;
        EnvironmentTestPassed = false; EnvironmentTesting = true;
        try { return await RunAsync(() => client.InvokeAsync("test_ssh_connection", new() { ["host"] = NativeEnvironments.HostPayload(EnvironmentDraft!) }, projectId), _ => EnvironmentTestPassed = true); }
        finally { EnvironmentTesting = false; if (!Closed) Notify(); }
    }
    public Task<bool> SaveEnvironmentAsync()
    {
        if (!CanSaveEnvironment) return Task.FromResult(false);
        var host = EnvironmentEditor == "host";
        var args = host ? new JsonObject { ["host"] = NativeEnvironments.HostPayload(EnvironmentDraft!) } : new JsonObject
        {
            ["contextId"] = NativeEnvironments.Text(EnvironmentTarget, "id"),
            ["pythonExecutable"] = NativeEnvironments.Text(EnvironmentDraft, "python_executable"), ["rscriptExecutable"] = NativeEnvironments.Text(EnvironmentDraft, "rscript_executable")
        };
        return RunAsync(() => client.InvokeAsync(host ? "add_ssh_host" : "update_execution_context_interpreters", args, projectId), value =>
        {
            if (host) Values["list_ssh_hosts"] = EnvironmentArray(value).DeepClone(); else { AcceptEnvironment(value); EnvironmentNoticeKey = "runtime_config.saved"; }
            ClearEnvironmentEditor();
        });
    }
    public async Task<bool> RequestRemoveEnvironmentHostAsync(string alias)
    {
        if (Busy || Closed || !EnvironmentHosts.Any(row => NativeEnvironments.Text(row, "alias") == alias)) return false;
        JsonObject? report = null;
        if (!await RunAsync(() => client.InvokeAsync("context_disposal_report", new() { ["contextId"] = "ssh:" + alias }, projectId), value => report = value as JsonObject
            ?? throw new InvalidDataException("Missing environment disposal report")) || Closed) return false;
        EnvironmentTarget = new() { ["alias"] = alias, ["report"] = report!.DeepClone() };
        if (NativeEnvironments.NeedsDisposalConfirmation(report)) { EnvironmentEditor = "remove"; Notify(); return true; }
        return await RemoveEnvironmentHostAsync();
    }
    public Task<bool> RemoveEnvironmentHostAsync()
    {
        var alias = NativeEnvironments.Text(EnvironmentTarget, "alias");
        if (Busy || Closed || EnvironmentTarget?["report"] == null || !EnvironmentHosts.Any(row => NativeEnvironments.Text(row, "alias") == alias)) return Task.FromResult(false);
        return RunAsync(() => client.InvokeAsync("remove_ssh_host", new() { ["alias"] = alias }, projectId), value =>
        {
            Values["list_ssh_hosts"] = EnvironmentArray(value).DeepClone();
            foreach (var context in Environments.Where(row => NativeEnvironments.Text(row, "id") == "ssh:" + alias)) ((JsonArray)Values["list_execution_contexts"]!).Remove(context);
            if (DefaultEnvironment == "ssh:" + alias) Values["get_default_execution_context"] = null;
            ClearEnvironmentEditor();
        });
    }
    public Task<bool> RevokeEnvironmentTrustAsync(string source, string destination)
    {
        if (Busy || Closed || !EnvironmentTrust.Any(row => NativeEnvironments.Text(row, "source_context_id") == source && NativeEnvironments.Text(row, "destination_context_id") == destination)) return Task.FromResult(false);
        EnvironmentCleanupError = null;
        return RunAsync(() => client.InvokeAsync("revoke_ssh_trust_edge", new() { ["sourceContextId"] = source, ["destinationContextId"] = destination }, projectId), value =>
        {
            Values["list_ssh_trust_edges"] = EnvironmentArray(value?["edges"]).DeepClone(); EnvironmentCleanupError = value?["cleanup_error"]?.GetValue<string>();
        });
    }
}
