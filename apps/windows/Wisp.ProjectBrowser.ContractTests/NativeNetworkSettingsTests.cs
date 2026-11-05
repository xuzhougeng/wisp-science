using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeNetworkSettingsTests
{
    public static async Task RunAsync()
    {
        var client = new Fake();
        using var model = new NativeNetworkSettingsModel(client, "project-a");
        var snapshot = Snapshot(); model.AcceptSnapshot(snapshot);
        model.SetMode("model", "custom");
        Check(model.HasChanges && model.Mode("model") == "custom" && model.Mode("model", saved: true) == "system",
            "an empty custom draft does not change the saved status badge");
        Check(!await model.SaveAsync("model") && client.Calls.Count == 0 && model.Error == "network.address_required",
            "an empty custom address cannot accidentally save system proxy mode");
        model.OpenMirrors();
        Check(model.MirrorsOpen && model.Error == null && model.Mode("model") == "custom", "entering mirrors clears the previous proxy error without dropping the proxy draft");
        model.CloseMirrors();
        Check(!model.MirrorsOpen && model.Mode("model") == "custom" && client.Calls.Count == 0, "immediate Back or Escape closes only mirrors and makes no request");
        model.SetAddress("model", "http://localhost:7890");
        model.SetMode("command", "direct"); model.SetMirror("pip_index_url", "https://mirror.example/simple");
        model.AcceptSnapshot(Snapshot());
        Check(model.HasChanges && model.Mode("command") == "direct", "refresh does not replace network drafts");
        Check(await model.SaveAsync("model"), "a selected scope can save while other scopes remain dirty");
        var sent = client.Calls[^1].Args["settings"]!;
        Check(sent["model_proxy_url"]!.GetValue<string>() == "http://localhost:7890"
            && sent["command_proxy_url"]!.GetValue<string>() == "" && sent["pip_index_url"]!.GetValue<string>() == ""
            && sent["future_flag"]!.GetValue<bool>(), "each request starts from saved values and preserves unknown host fields");
        Check(model.Mode("command") == "direct" && model.Mode("command", saved: true) == "system"
            && model.Draft["pip_index_url"]!.GetValue<string>() == "https://mirror.example/simple" && model.HasChanges,
            "acknowledgement only commits the selected row and retains the other drafts");
        Check(model.SaveConfirmed, "the acknowledgement is visible until another edit or entry into mirrors");
        model.OpenMirrors();
        Check(!model.SaveConfirmed && model.Draft["pip_index_url"]!.GetValue<string>() == "https://mirror.example/simple", "opening the mirror form clears stale confirmation, not field values");
        model.CloseMirrors(); model.OpenMirrors();
        Check(model.Draft["pip_index_url"]!.GetValue<string>() == "https://mirror.example/simple", "returning via breadcrumb or Escape preserves mirror edits");
        model.CloseMirrors(cancel: true);
        Check(model.Draft["pip_index_url"]!.GetValue<string>() == "" && model.Mode("command") == "direct",
            "cancel mirrors cannot discard proxy edits");
        Check(await model.SaveAsync("command") && !model.HasChanges
            && client.Calls[^1].Args["settings"]!["command_proxy_url"]!.GetValue<string>() == "none",
            "direct mode uses the host sentinel and acknowledges the final draft");
        model.SetMode("model", "system"); client.Malformed = true;
        Check(!await model.SaveAsync("model") && model.HasChanges && model.Mode("model", saved: true) == "custom",
            "unconfirmed saves preserve both the edit and the last confirmed badge");
        var attempts = client.Calls.Count; client.Malformed = false; client.Fail = true;
        Check(!await model.SaveAsync("model") && client.Calls.Count == attempts + 1 && model.HasChanges,
            "failed saves do not replay automatically");
        client.Fail = false;
        Check(await model.SaveAsync("model") && client.Calls[^1].Args["settings"]!["model_proxy_url"]!.GetValue<string>() == ""
            && !model.HasChanges, "explicit retry clears the proxy with an empty string");
        model.SetMirror("conda_mirror_url", "https://mirror.example/conda");
        model.SetMirror("ca_bundle_path", "C:\\certs\\root.pem"); model.SetMode("mcp", "custom");
        Check(await model.SaveAsync("mirrors") && model.HasChanges && model.Mode("mcp") == "custom",
            "saving mirrors preserves an empty custom proxy editor");
        Check(client.Calls[^1].Args["settings"]!["ca_bundle_path"]!.GetValue<string>() == "C:\\certs\\root.pem",
            "mirror save includes the CA path in the existing host contract");
        model.Discard(); Check(!model.HasChanges && model.Mode("mcp") == "system", "discard restores only acknowledged values");
        Check(client.Calls.All(c => c.Command == "set_network_settings" && c.Project == "project-a"),
            "network edits remain in their own command and project scope");
        Check(snapshot["model_proxy_url"]!.GetValue<string>() == "", "network editing never mutates a shared snapshot");
        model.SetMode("subscription", "direct"); var pending = new TaskCompletionSource<JsonNode?>(); client.Pending = pending.Task;
        var save = model.SaveAsync("subscription"); model.Dispose(); pending.SetResult(Snapshot());
        Check(!await save && model.Saved.Count == 0 && model.Draft.Count == 0, "closed pages reject late save acknowledgements");
    }
    private static JsonObject Snapshot() => new() {
        ["model_proxy_url"] = "", ["subscription_proxy_url"] = "", ["mcp_proxy_url"] = "", ["command_proxy_url"] = "",
        ["conda_mirror_url"] = "", ["pip_index_url"] = "", ["ca_bundle_path"] = "", ["future_flag"] = true };
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public bool Malformed, Fail;
        public Task<JsonNode?>? Pending;
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            if (Fail) return Task.FromException<JsonNode?>(new IOException("lost acknowledgement"));
            var result = arguments["settings"]?.DeepClone();
            if (result is JsonObject value) value["new_host_field"] = "retained";
            return Pending ?? Task.FromResult(Malformed ? null : result);
        }
    }
}
