using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeArtifactModalTests
{
    public static async Task RunAsync(string fixture)
    {
        Check(NativeArtifactGroups.EnglishLabel("image/png") == "Images · PNG"
            && NativeArtifactGroups.EnglishLabel("text/x-python") == "Python code"
            && NativeArtifactGroups.EnglishLabel(null) == "Other", "artifact headings follow the selected UI language");
        var sample = JsonNode.Parse(File.ReadAllText(fixture))!;
        var host = new Host(sample); var client = new NativePanelClient(host);
        var provenance = await client.ArtifactProvenanceAsync("project-a", "session-a", "figure-a");
        Check(provenance?.Inputs.Single().ProducedHere == true && provenance.Env?.Packages.Single().Version == "1.0", "shared provenance fixture");
        foreach (var field in new[] { "schema", "project_id", "session_id", "artifact_id" })
        {
            host.Reply = sample.DeepClone(); host.Reply[field] = "wrong";
            try { await client.ArtifactProvenanceAsync("project-a", "session-a", "figure-a"); throw new Exception("mismatched envelope accepted"); }
            catch (InvalidDataException) { }
        }
        host.Reply = sample.DeepClone();
        var artifacts = new[] { Item("figure-a", "a.png", "image/png"), Item("table", "data/source.csv", "text/csv"), Item("figure-b", "b.png", "image/png") };
        var model = new WorkspaceArtifactModel(client, "project-a", "session-a", artifacts);
        await model.OpenAsync("figure-a");
        Check(model.Content != null && model.Provenance != null && model.Adjacent(-1) == null && model.Adjacent(1)?.Id == "figure-b", "gallery navigation skips non-images");
        Check(model.InputArtifact("data/source.csv")?.Id == "table", "produced input resolves to listed artifact");
        host.FailProvenance = true; await model.NavigateAsync(1);
        Check(model.Selected?.Id == "figure-b" && model.Content != null && model.ProvenanceError != null && model.Adjacent(1) == null, "provenance failure does not hide preview");
        host.FailProvenance = false;
        host.Hold = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var pending = model.OpenAsync("figure-a"); model.Close(); host.Hold.SetResult(); await pending;
        Check(model.Selected == null && model.Content == null && model.Provenance == null, "closing prevents late reopen");
        host.Hold = new(TaskCreationOptions.RunContinuationsAsynchronously);
        pending = model.OpenAsync("figure-a"); await model.OpenAsync("table"); host.Hold.SetResult(); await pending;
        Check(model.Selected?.Id == "table" && model.Content?.Path == "table" && model.Adjacent(1) == null, "old image reply cannot replace selected text artifact");
        try { await model.OpenAsync("not-listed"); throw new Exception("unlisted artifact accepted"); } catch (InvalidOperationException) { }
        Check(host.Calls.All(c => c.StartsWith("native_conversation_panel_")), "modal reads use scoped panel transport");
    }
    private static NativePanelArtifact Item(string id, string path, string kind) => new(id, Path.GetFileName(path), kind, path, null, 0, path);
    private static void Check(bool ok, string message) { if (!ok) throw new InvalidOperationException(message); }
    private sealed class Host(JsonNode reply) : INativeSettingsClient
    {
        public JsonNode Reply = reply; public bool FailProvenance;
        public TaskCompletionSource? Hold;
        public List<string> Calls = [];
        public async Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add(command); var id = args["artifact_id"]!.GetValue<string>();
            if (id == "figure-a" && Hold is { } hold) await hold.Task;
            if (command.EndsWith("readartifact")) return JsonSerializer.SerializeToNode(new NativePanelFileContent(id, "text/plain", "preview", null, false, 7), ConversationSnapshot.JsonOptions);
            if (FailProvenance) throw new IOException("provenance unavailable");
            var result = Reply.DeepClone(); if (id != "figure-a") result["artifact_id"] = id; return result;
        }
    }
}
