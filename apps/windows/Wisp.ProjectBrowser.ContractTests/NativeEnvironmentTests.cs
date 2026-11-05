using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeEnvironmentTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        await StandaloneHostAsync();
        Check(NativeEnvironments.RemoteOptions([Context("local", "local"), Context("ssh:lab", "ssh"), Context("wsl:Ubuntu", "wsl"), Context("unknown", "future")])
            .Select(row => row.Id).SequenceEqual(["ssh:lab", "wsl:Ubuntu"]), "default picker only includes supported remote contexts, preserving order");
        Check(NativeEnvironments.Config(Context("local", "local")).Count == 0, "missing interpreter config uses automatic discovery");
        var caps = Context("local", "local"); caps["capabilities_json"] = "{\"os\":\"linux\",\"arch\":\"x86_64\",\"python\":\"3.12\",\"probe_skill\":true}";
        Check(NativeEnvironments.Capabilities(caps) == "linux/x86_64 · 3.12 · No GPU", "capability summary follows reference ordering");
        Check(NativeEnvironments.FailureKind("SSH authentication succeeded, but probe command returned no output") == "probe_output"
            && NativeEnvironments.FailureKind("SSH password authentication failed: Permission denied") == "password"
            && NativeEnvironments.FailureKind("Identity file is not accessible") == "identity"
            && NativeEnvironments.FailureCauses("auth").Length == 4, "specific diagnosis wins over generic authentication errors");
        var host = new JsonObject { ["alias"] = " lab ", ["port"] = "", ["auth_method"] = "password", ["has_password"] = true, ["password"] = "", ["identity_file"] = "ignored" };
        Check(NativeEnvironments.CanSaveHost(host) && NativeEnvironments.HostPayload(host)["password"] == null && NativeEnvironments.HostPayload(host)["identity_file"] == null
            && NativeEnvironments.HostPayload(host)["alias"]!.ToString() == "lab" && NativeEnvironments.HostPayload(host)["port"] == null, "stored passwords remain untouched and inactive identity paths are not sent");
        host["has_password"] = false; Check(!NativeEnvironments.CanSaveHost(host), "new password authentication requires a password");
        host["password"] = "synthetic"; host["port"] = "65536"; Check(!NativeEnvironments.CanSaveHost(host), "out of range ports cannot save");
        host["port"] = "22"; Check(NativeEnvironments.CanSaveHost(host), "valid password and port enable save");
        var client = new Fake(); using var model = new NativeSettingsEditorModel(client, "project");
        Check(!await model.SetDefaultEnvironmentAsync("ssh:lab") && !model.OpenEnvironmentRuntime("local"), "unknown and unloaded contexts do not write");
        await model.LoadAsync("list_execution_contexts", "list_ssh_hosts", "get_default_execution_context", "list_ssh_trust_edges");
        var count = client.Calls.Count; Check(!await model.SetDefaultEnvironmentAsync("local") && client.Calls.Count == count, "local sentinel is represented by null, not a persisted local ID");
        client.Fail = "set_default_execution_context"; Check(!await model.SetDefaultEnvironmentAsync("ssh:lab") && model.DefaultEnvironment == null, "failed default change preserves acknowledged selection");
        client.Fail = null; Check(await model.SetDefaultEnvironmentAsync("ssh:lab") && model.DefaultEnvironment == "ssh:lab", "default changes only after acknowledgement");
        Check(model.OpenEnvironmentHost("lab"), "known host opens"); model.SetEnvironmentField("alias", "renamed");
        Check(model.EnvironmentDraft!["alias"]!.ToString() == "lab", "editing locks alias identity");
        model.SetEnvironmentField("notes", "new note"); client.Fail = "add_ssh_host";
        Check(!await model.SaveEnvironmentAsync() && model.EnvironmentDraft!["notes"]!.ToString() == "new note" && model.EnvironmentEditor == "host", "failed save keeps draft and modal");
        client.Fail = null; Check(await model.SaveEnvironmentAsync() && model.EnvironmentEditor == null && model.EnvironmentHosts[0]["notes"]!.ToString() == "new note", "host save applies returned list and clears draft");
        model.OpenEnvironmentHost(); model.SetEnvironmentField("alias", "sample"); model.SetEnvironmentField("auth_method", "password"); model.SetEnvironmentField("password", "synthetic");
        client.Pending = new(); var test = model.TestEnvironmentHostAsync(); count = client.Calls.Count;
        Check(!model.CancelEnvironmentEditor() && !await model.TestEnvironmentHostAsync() && client.Calls.Count == count, "busy test cannot close or duplicate operation");
        client.Pending.SetResult(null); await test; client.Pending = null;
        Check(model.EnvironmentTestPassed, "test acknowledgement is visible"); model.SetEnvironmentField("user", "different"); Check(!model.EnvironmentTestPassed, "changing connection invalidates test result"); model.CancelEnvironmentEditor();
        model.Environments.Single(row => row["id"]!.ToString() == "local")["capabilities_json"] = "{\"python_executable\":\"probed-python\"}";
        model.OpenEnvironmentRuntime("local");
        Check(model.EnvironmentDraft!["python_executable"]!.ToString() == "probed-python", "settings and runtime popover share probe fallback");
        model.SetEnvironmentField("python_executable", "C:\\sample\\python.exe"); client.Fail = "update_execution_context_interpreters";
        Check(!await model.SaveEnvironmentAsync() && model.EnvironmentDraft!["python_executable"]!.ToString().Contains("sample"), "interpreter write failure retains input");
        client.Fail = null; Check(await model.SaveEnvironmentAsync() && model.EnvironmentNoticeKey == "runtime_config.saved" && client.Calls[^1].Args.ContainsKey("pythonExecutable"), "runtime write uses camel case contract");
        client.Fail = "probe_execution_context"; Check(!await model.ProbeEnvironmentAsync("ssh:lab") && model.EnvironmentEditor == "diagnosis" && model.ProbingEnvironment == null, "transport probe failure stops spinner and opens diagnosis");
        model.CancelEnvironmentEditor(); client.Fail = null; client.ProbeError = true;
        Check(!await model.ProbeEnvironmentAsync("ssh:lab") && model.Environments.Single(row => row["id"]!.ToString() == "ssh:lab")["last_probe_status"]!.ToString() == "error", "failed probe acknowledgement updates status and opens diagnosis");
        model.CancelEnvironmentEditor(); client.ProbeError = false;
        Check(await model.ProbeEnvironmentAsync("ssh:lab") && model.EnvironmentNoticeKey == "contexts.probe_success", "successful probe displays host capabilities");
        client.Fail = "context_disposal_report"; count = client.Calls.Count;
        Check(!await model.RequestRemoveEnvironmentHostAsync("lab") && client.Calls.Count == count + 1 && model.EnvironmentHosts.Length == 1, "unconfirmed disposal report cannot remove host");
        client.Fail = null; Check(await model.RequestRemoveEnvironmentHostAsync("lab") && model.EnvironmentEditor == "remove" && model.EnvironmentHosts.Length == 1, "external references require confirmation");
        client.Fail = "remove_ssh_host"; Check(!await model.RemoveEnvironmentHostAsync() && model.EnvironmentEditor == "remove", "failed removal keeps confirmation for explicit retry");
        client.Fail = null; Check(await model.RemoveEnvironmentHostAsync() && model.EnvironmentHosts.Length == 0 && model.DefaultEnvironment == null && !model.Environments.Any(row => row["id"]!.ToString() == "ssh:lab"), "acknowledged removal clears only matching context and default");
        Check(await model.RevokeEnvironmentTrustAsync("ssh:lab", "ssh:other") && model.EnvironmentTrust.Length == 0 && model.EnvironmentCleanupError == "Synthetic cleanup failed", "trust edge acknowledgement applies even when remote key cleanup fails");
        Check(client.Calls.All(call => call.Project == "project"), "all operations preserve mounted project context");
        var late = new Fake(); var closed = new NativeSettingsEditorModel(late, null); await closed.LoadAsync("list_execution_contexts", "list_ssh_hosts", "get_default_execution_context", "list_ssh_trust_edges");
        closed.OpenEnvironmentHost(); closed.SetEnvironmentField("alias", "sample"); closed.SetEnvironmentField("password", "synthetic"); late.Pending = new();
        var save = closed.SaveEnvironmentAsync(); closed.Dispose(); late.Pending.SetResult(new JsonArray()); await save;
        Check(closed.EnvironmentDraft == null && closed.EnvironmentEditor == null && closed.Values.Count == 0, "closing erases secrets and ignores late acknowledgements");
    }
    private static async Task StandaloneHostAsync()
    {
        var client = new Fake();
        using var model = new NativeSettingsEditorModel(client, "conversation-project");
        Check(model.OpenEnvironmentHost() && model.EnvironmentEditor == "host" && client.Calls.Count == 0,
            "composer shortcut opens a fresh host draft without settings loads or mutations");
        model.SetEnvironmentField("alias", "unsaved");
        Check(model.CancelEnvironmentEditor() && model.EnvironmentDraft == null && client.Calls.Count == 0,
            "immediate Escape/cancel clears the standalone draft without creating a host or session");
        model.OpenEnvironmentHost();
        Check(model.EnvironmentDraft!["alias"]!.ToString() == "", "reopening does not restore a discarded host");
        model.SetEnvironmentField("alias", "review-host"); client.Fail = "add_ssh_host";
        Check(!await model.SaveEnvironmentAsync() && model.EnvironmentEditor == "host" && model.EnvironmentDraft!["alias"]!.ToString() == "review-host",
            "standalone failure keeps the editor and inputs for an explicit decision");
        Check(client.Calls.Count == 1 && client.Calls[0].Command == "add_ssh_host" && client.Calls[0].Project == "conversation-project",
            "one explicit save uses captured project scope without creating a conversation or connecting");
        client.Fail = null;
        Check(await model.SaveEnvironmentAsync() && model.EnvironmentEditor == null && model.EnvironmentDraft == null && model.EnvironmentHosts.Length == 1,
            "only the confirmed save clears the standalone host form");
        Check(client.Calls.Count == 2 && client.Calls.All(call => call.Command == "add_ssh_host"), "no automatic retry, probe or unrelated settings read");
    }
    private static JsonObject Context(string id, string kind) => new() { ["id"] = id, ["kind"] = kind, ["label"] = id };
    private sealed class Fake : INativeSettingsClient
    {
        public string? Fail; public bool ProbeError; public TaskCompletionSource<JsonNode?>? Pending;
        public readonly List<(string Command, JsonObject Args, string? Project)> Calls = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)args.DeepClone(), projectId));
            if (Fail == command) return Task.FromException<JsonNode?>(new IOException("Synthetic connection timed out"));
            if (Pending != null) return Pending.Task;
            JsonNode? reply = command switch
            {
                "list_execution_contexts" => new JsonArray(Context("local", "local"), Context("ssh:lab", "ssh")),
                "list_ssh_hosts" => new JsonArray(new JsonObject { ["alias"] = "lab", ["port"] = 22, ["auth_method"] = "key" }),
                "get_default_execution_context" => null,
                "list_ssh_trust_edges" => new JsonArray(new JsonObject { ["source_context_id"] = "ssh:lab", ["destination_context_id"] = "ssh:other" }),
                "set_default_execution_context" => args["contextId"]?.DeepClone(),
                "add_ssh_host" => new JsonArray(args["host"]!.DeepClone()),
                "update_execution_context_interpreters" => new JsonObject { ["id"] = args["contextId"]!.ToString(), ["kind"] = "local", ["label"] = "Local", ["config_json"] = new JsonObject { ["python_executable"] = args["pythonExecutable"]!.ToString() }.ToJsonString() },
                "probe_execution_context" => new JsonObject { ["id"] = args["contextId"]!.ToString(), ["kind"] = "ssh", ["label"] = "Lab", ["last_probe_status"] = ProbeError ? "error" : "ok", ["last_probe_error"] = ProbeError ? "SSH key authentication failed" : null, ["capabilities_json"] = "{\"os\":\"linux\",\"arch\":\"x86_64\",\"hostname\":\"lab\"}" },
                "context_disposal_report" => new JsonObject { ["external_references"] = 2, ["staged_files"] = 0, ["active_runs"] = 0 },
                "remove_ssh_host" => new JsonArray(),
                "revoke_ssh_trust_edge" => new JsonObject { ["edges"] = new JsonArray(), ["cleanup_error"] = "Synthetic cleanup failed" },
                _ => null
            };
            return Task.FromResult(reply);
        }
    }
}
