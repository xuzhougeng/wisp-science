using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeTranscriptSelectionTests
{
    private static void Check(bool value, string message) { if (!value) throw new Exception(message); }
    public static async Task RunAsync()
    {
        var host = new Host(); var model = new WorkspaceConversationModel(new NativeConversationClient(host));
        await model.OpenAsync("p", "s"); model.Draft = "Keep my draft";
        var source = model.Snapshot!;
        model.ApplyInputPreferences(new(SelectionPopupEnabled: false));
        Check(model.CanQuote(source, 1, "selected evidence") && model.AddQuote(source, 1, "selected evidence"),
            "disabling the automatic popup retains explicit transcript quoting");
        Check(model.Draft == "Keep my draft" && model.Quotes.Single().Text == "selected evidence" && host.Writes == 0,
            "quoting preserves the draft and never sends or writes to the host");
        Check(!model.CanQuote(source, 1, " ") && !model.CanQuote(source, -1, "evidence")
            && !model.CanQuote(source, 2, "evidence") && !model.CanQuote(source, 1, new string('x', 32769)),
            "menu eligibility rejects blank, unavailable and oversized selections");
        Check(!model.CanQuote(source with { ProjectId = "other" }, 1, "evidence")
            && !model.AddQuote(source with { ProjectId = "other" }, 1, "evidence"), "same session id in another project is not the current source");
        await model.OpenAsync("p", "next");
        Check(!model.CanQuote(source, 1, "stale") && !model.AddQuote(source, 1, "stale") && model.Quotes.Length == 0,
            "a menu opened before navigation cannot attach to the new session");
        host.ReadOnly = true; await model.RefreshAsync();
        Check(!model.CanQuote(model.Snapshot!, 1, "evidence"), "read-only destination disables Add to chat");
        host.ReadOnly = false; await model.RefreshAsync();
        for (var n = 0; n < 16; n++) Check(model.AddQuote(model.Snapshot!, 1, "selection " + n), "bounded quote is accepted");
        Check(!model.CanQuote(model.Snapshot!, 1, "one too many") && !model.AddQuote(model.Snapshot!, 1, "one too many"),
            "menu and command agree at the attachment count limit");
        model.Pause();
    }
    private sealed class Host : INativeSettingsClient
    {
        public bool ReadOnly; public int Writes; private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "native_conversation_seen") return Task.FromResult<JsonNode?>(null);
            if (command != "native_conversation_snapshot") { Writes++; throw new InvalidOperationException(command); }
            return Task.FromResult(JsonSerializer.SerializeToNode(new ConversationSnapshot(ConversationSnapshot.SchemaId,
                "memory", ++sequence, projectId!, args["session_id"]!.GetValue<string>(),
                [new("user", "Question", null, null, null, null), new("assistant", "Selected evidence", null, null, null, null)],
                null, false, false, ReadOnly, "model", null, null, []), ConversationSnapshot.JsonOptions));
        }
    }
}
