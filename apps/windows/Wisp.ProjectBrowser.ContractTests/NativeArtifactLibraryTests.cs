using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeArtifactLibraryTests
{
    public static async Task RunAsync()
    {
        Check(NativeArtifactNavigation.Direction(37, false, false, false) == -1, "left navigates");
        Check(NativeArtifactNavigation.Direction(39, false, false, false) == 1, "right navigates");
        foreach (var key in new[] { 37, 39 })
        {
            Check(NativeArtifactNavigation.Direction(key, true, false, false) == 0, "top layer blocks");
            Check(NativeArtifactNavigation.Direction(key, false, true, false) == 0, "code editing retains arrows");
            Check(NativeArtifactNavigation.Direction(key, false, false, true) == 0, "modified arrows retain meaning");
        }
        var host = new Host(); var client = new NativePanelClient(host);
        var artifacts = new[] { new NativePanelArtifact("a", "a.png", "image/png", "plots/a.png", null, 0, null),
            new NativePanelArtifact("b", "b.csv", "text/csv", "data/b.csv", null, 0, null) };
        var model = new WorkspaceArtifactModel(client, "p", "s", artifacts);
        await model.OpenAsync("a"); Check(model.LibraryKnown && model.LibraryItemId == null, "initial status read");
        await model.ToggleLibraryAsync(); Check(model.LibraryItemId == "saved-a" && host.Writes == 1, "star snapshot confirmed");
        await model.ToggleLibraryAsync(); Check(model.LibraryItemId == null && host.Writes == 2, "unstar confirmed");
        host.Fail = true; await model.ToggleLibraryAsync();
        Check(model.Content != null && model.LibraryError != null && !model.LibraryKnown, "failed write leaves preview and unknown status");
        host.Fail = false; var before = host.Writes; await model.ToggleLibraryAsync();
        Check(model.LibraryKnown && host.Writes == before, "retry only reads uncertain result");
        host.Hold = new(); var pending = model.ToggleLibraryAsync(); await model.ToggleLibraryAsync();
        Check(host.Writes == before + 1, "duplicate mutation is suppressed");
        await model.OpenAsync("b"); host.Hold.SetResult(); await pending;
        Check(model.Selected?.Id == "b" && model.LibraryItemId == null && !model.LibraryBusy, "late image star cannot mark CSV starred");
        host.Hold = null; await model.ToggleLibraryAsync(); Check(host.Writes == before + 1, "non-image has no star operation");
        var draft = new WorkspacePanelModel(client, "p", null, new NativePanelTabs("[\"runs\"]", "runs", ["runs"]));
        var projectImage = draft.ContextArtifactPreview(new("run", "a", "QC plot", "a.png", "image/png", "s"));
        var libraryCalls = host.LibraryCalls;
        foreach (var reader in new[] { projectImage, projectImage.Fork() })
        {
            await reader.OpenAsync("a"); await reader.ToggleLibraryAsync();
            Check(reader.Content != null && !reader.CanUseLibrary && host.LibraryCalls == libraryCalls,
                "opening a project run image may read the owner's snapshot but must not access its session library");
        }
        host.Foreign = true;
        try { await client.ArtifactLibraryAsync("p", "s", "a"); throw new Exception("scope mismatch accepted"); }
        catch (InvalidDataException) { }
        var panel = new WorkspacePanelModel(client, "p", "s", new NativePanelTabs("[\"artifacts\"]", "artifacts", ["artifacts", "files"]));
        await panel.RevealArtifactInputAsync("data/unregistered.csv");
        Check(panel.Tabs.Selected == "files" && panel.Path == "data" && panel.Preview == null, "unregistered produced input reveals local parent instead of previewing");
        Check(host.LastDirectory == "data", "directory request uses input parent");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Host : INativeSettingsClient
    {
        public int Writes, LibraryCalls; public bool Fail, Foreign; public string? Saved, LastDirectory; public TaskCompletionSource? Hold;
        public async Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Check(projectId == "p" && args["session_id"]!.GetValue<string>() == "s", "explicit scope");
            if (command.EndsWith("_files")) { LastDirectory = args["path"]!.GetValue<string>(); return new JsonArray(); }
            var id = args["artifact_id"]!.GetValue<string>();
            if (command.EndsWith("artifact_library"))
            {
                LibraryCalls++;
                if (args["enabled"] is { } value) { Writes++; if (Hold is { } hold) await hold.Task; if (Fail) throw new IOException("unconfirmed"); Saved = value.GetValue<bool>() ? "saved-a" : null; }
                return JsonSerializer.SerializeToNode(new NativeArtifactLibraryReply("wisp.native-conversations.v1", Foreign ? "other" : "p", "s", id, Saved), ConversationSnapshot.JsonOptions);
            }
            if (command.EndsWith("readartifact")) return JsonSerializer.SerializeToNode(new NativePanelFileContent("plots/a.png", "text/plain", "preview", null, false, 7), ConversationSnapshot.JsonOptions);
            return JsonSerializer.SerializeToNode(new NativeArtifactProvenanceReply("wisp.native-conversations.v1", "p", "s", id, "plots/a.png", null), ConversationSnapshot.JsonOptions);
        }
    }
}
