using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeFileProvenanceTests
{
    public static async Task RunAsync(string fixture)
    {
        static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
        var sample = JsonNode.Parse(File.ReadAllText(fixture))!;
        var host = new Host(sample); var client = new NativePanelClient(host);
        const string path = "figures/generated.png";
        var value = await client.FileProvenanceAsync("project-a", "session-a", path);
        Check(value?.Code == "print('synthetic example')\n" && value.Output == "synthetic example\n"
            && value.Inputs.Single().ProducedHere && value.Env?.Packages.Single().Version == "1.0", "shared path-provenance fixture carries all four detail tabs");
        foreach (var field in new[] { "schema", "project_id", "session_id", "path" })
        {
            host.Corrupt = field;
            try { await client.FileProvenanceAsync("project-a", "session-a", path); throw new Exception("Mismatched path provenance accepted"); }
            catch (InvalidDataException) { }
        }
        host.Corrupt = null;
        var output = new NativeGeneratedOutput("generated-id", "project-a", "session-a", "reply", 0, "generated.png", "image", path, null, null);
        var second = output with { Id = "second-id", Name = "second.png", Path = "figures/second.png" };
        var reader = WorkspaceArtifactModel.ForGenerated(client, "project-a", "session-a", [output, second]);
        await reader.OpenAsync(output.Id);
        Check(reader.Content != null && reader.Provenance != null && !reader.CanUseLibrary, "generated modal loads provenance without inventing a library artifact ID");
        var fork = reader.Fork(); await fork.OpenAsync(output.Id);
        Check(fork.Provenance?.Code == value?.Code, "center-tab reader forks retain path provenance");
        host.Fail = true; await reader.OpenAsync(output.Id);
        Check(reader.Content != null && reader.Provenance == null && reader.ProvenanceError == "Provenance unavailable", "failed provenance leaves the independent file preview visible");
        host.Fail = false; await reader.OpenAsync(output.Id);
        Check(reader.Provenance != null && reader.ProvenanceError == null, "explicit retry restores provenance");
        host.Empty = true; await reader.OpenAsync(output.Id);
        Check(reader.Provenance == null && reader.ProvenanceError == null && !reader.LoadingProvenance, "missing provenance is a loaded empty state, not an error");
        host.Empty = false;
        host.Hold = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var pending = reader.OpenAsync(output.Id);
        Check(reader.Content != null && reader.LoadingProvenance, "file preview does not wait for slow provenance");
        reader.Close(); host.Hold.SetResult(); await pending;
        Check(reader.Selected == null && reader.Provenance == null, "late provenance cannot reopen a closed viewer");
        host.Hold = new(TaskCreationOptions.RunContinuationsAsynchronously);
        pending = reader.OpenAsync(output.Id); await reader.OpenAsync(second.Id); host.Hold.SetResult(); await pending;
        Check(reader.Selected?.Id == second.Id && reader.Provenance?.Code == "second image", "late provenance cannot replace the next image's details");
        host.Hold = new(TaskCreationOptions.RunContinuationsAsynchronously);
        using var cancel = new CancellationTokenSource();
        pending = reader.OpenAsync(output.Id, cancel.Token); cancel.Cancel(); host.Hold.SetResult(); await pending;
        Check(reader.Provenance == null && reader.ProvenanceError == null, "cancelled provenance results are discarded");
        Check(host.Calls.All(command => command is "native_conversation_panel_readfile" or "native_conversation_panel_file_provenance"), "generated viewer never calls registered artifact or mutation commands");
        reader.Close(); fork.Close();
    }
    private sealed class Host(JsonNode sample) : INativeSettingsClient
    {
        public string? Corrupt; public bool Fail, Empty; public TaskCompletionSource? Hold;
        public List<string> Calls = [];
        public async Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (projectId != "project-a" || args["session_id"]?.GetValue<string>() != "session-a" || args.ContainsKey("artifact_id"))
                throw new InvalidOperationException("Wrong generated provenance request scope");
            Calls.Add(command); var path = args["path"]!.GetValue<string>();
            if (command == "native_conversation_panel_readfile")
                return JsonSerializer.SerializeToNode(new NativePanelFileContent(path, "text/plain", "preview", null, false, 7), ConversationSnapshot.JsonOptions);
            if (command != "native_conversation_panel_file_provenance") throw new InvalidOperationException("Unexpected command");
            if (Hold != null && path == "figures/generated.png") await Hold.Task;
            if (Fail) throw new IOException("Provenance unavailable");
            var result = sample.DeepClone(); result["path"] = path;
            if (Empty) result["provenance"] = null;
            else if (path == "figures/second.png") result["provenance"]!["code"] = "second image";
            if (Corrupt != null) result[Corrupt] = "wrong";
            return result;
        }
    }
}
