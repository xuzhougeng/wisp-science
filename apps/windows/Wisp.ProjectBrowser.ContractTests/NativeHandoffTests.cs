using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeHandoffTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private static ConversationSnapshot Snapshot(string session = "s", string epoch = "e") => new(
        ConversationSnapshot.SchemaId, epoch, 1, "p", session, [], null, false, false, false, "m", null, null, []);
    public static async Task RunAsync()
    {
        var host = new Fake(); var model = new NativeHandoffModel(host); model.Bind(Snapshot(), true);
        Check(host.Calls == 0, "Binding never calls a model.");
        await model.OpenAsync(); Check(model.Visible && model.Draft == "Summary" && model.CanStart && host.Calls == 1, "Open generates once, trims the returned note.");
        model.SetInstruction("  retain methods  "); model.SetDraft("Edited note"); host.Fail = true;
        await model.GenerateAsync(); Check(model.Error != null && model.Draft == "Edited note" && host.Instruction == "retain methods", "Generation failure preserves edits and explicit instructions.");
        model.SetDraft("  "); Check(!model.CanStart, "Blank summary cannot create a session.");
        model.SetDraft(" Edited note "); var launches = 0;
        await model.StartAsync(launch => { launches++; Check(launch == new NativeHandoffLaunch("p", "s", "e", "Edited note"), "Start captures the scoped edited note."); throw new IOException("Unknown create outcome"); });
        await model.StartAsync(_ => { launches++; return Task.CompletedTask; });
        Check(launches == 1 && model.Uncertain && !model.CanStart, "Unknown mutation outcome is not replayed.");
        model.Close(); Check(!model.Visible && model.Draft == "", "Close clears the dialog.");
        host.Fail = false; host.Pending = new(); var open = model.OpenAsync();
        Check(model.Generating, "Generation is visible."); model.Close(); host.Pending.SetResult(Fake.Response()); await open;
        Check(!model.Visible && model.Draft == "" && !model.Generating, "Closing during generation rejects its late reply.");
        host.Pending = new(); open = model.OpenAsync(); model.Bind(Snapshot("other"), true); host.Pending.SetResult(Fake.Response()); await open;
        Check(!model.Visible && model.Draft == "", "Navigating rejects the previous source response.");
        host.Pending = null; await model.OpenAsync(); Check(model.Error != null && !model.CanStart, "Response scope is independently validated."); model.Close();
        foreach (var snapshot in new[] { Snapshot() with { Running = true }, Snapshot() with { ReadOnly = true }, Snapshot() with { AcpAgentId = "a" }, Snapshot() with { ModelId = "acp:a" } })
        { var calls = host.Calls; model.Bind(snapshot, true); await model.OpenAsync(); Check(host.Calls == calls && !model.Available, "Ineligible conversations cannot generate."); }
        model.Bind(Snapshot(), false); await model.OpenAsync(); Check(!model.Available, "History/unavailable scope cannot generate.");
        model.Bind(Snapshot(), true); await model.OpenAsync(); model.SetDraft("Summary");
        await model.StartAsync(_ => { launches++; return Task.CompletedTask; }); Check(!model.Visible && !model.Starting, "Successful handoff dismisses exactly once.");
        var conversationHost = new ConversationFake(); var workspace = new WorkspaceConversationModel(new NativeConversationClient(conversationHost), conversationHost);
        await workspace.OpenAsync("p", "source"); workspace.Draft = "Original unsent draft";
        var destination = await workspace.PrepareHandoffSessionAsync(new("p", "source", "e", " Edited handoff "));
        Check(workspace.Draft == "Original unsent draft" && conversationHost.Creates == 1 && conversationHost.Sends == 0, "Preparing the destination neither edits the source nor sends.");
        await workspace.OpenAsync("p", destination);
        Check(workspace.Draft == "Edited handoff" && workspace.Attachments.Length == 0 && workspace.References.Length == 0 && workspace.Quotes.Length == 0, "Destination receives only its handoff.");
        await workspace.SendAsync(); Check(conversationHost.Sends == 1 && conversationHost.Message == "Edited handoff", "Start uses the existing scoped send pipeline once.");
        await workspace.OpenAsync("p", "source"); Check(workspace.Draft == "Original unsent draft", "Returning to the source restores its draft.");
        var rejected = false; try { await workspace.PrepareHandoffSessionAsync(new("p", "wrong", "e", "note")); } catch (InvalidOperationException) { rejected = true; }
        Check(rejected && conversationHost.Creates == 1, "Stale source cannot create a destination.");
        Check(workspace.CanStartPromptSession("p") && !workspace.CanStartPromptSession("wrong"), "Environment setup belongs to its project.");
        var setup = await workspace.CreatePromptSessionAsync("p", "Set up my environment");
        Check(workspace.Draft == "Original unsent draft" && conversationHost.Sends == 1, "Setting up a new draft neither sends nor edits the source.");
        await workspace.OpenAsync("p", setup);
        Check(workspace.Draft == "Set up my environment" && workspace.Attachments.Length == 0, "Only the setup prompt enters the destination draft.");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public int Calls; public bool Fail; public string? Instruction; public TaskCompletionSource<JsonNode?>? Pending;
        public static JsonNode Response() => new JsonObject { ["project_id"] = "p", ["session_id"] = "s", ["note"] = " Summary " };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Check(command == "native_conversation_handoff" && projectId == "p" && args.Count == 2, "Only the canonical scoped generation route is used.");
            Calls++; Instruction = args["instruction"]?.GetValue<string>(); if (Fail) throw new IOException("Synthetic failure");
            return Pending?.Task ?? Task.FromResult<JsonNode?>(Response());
        }
    }
    private sealed class ConversationFake : INativeSettingsClient
    {
        public int Creates, Sends; public string? Message; private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            JsonNode? response = null;
            if (command == "native_conversation_snapshot") response = System.Text.Json.JsonSerializer.SerializeToNode(Snapshot(args["session_id"]!.GetValue<string>()) with { Sequence = ++sequence }, ConversationSnapshot.JsonOptions);
            else if (command == "native_conversation_create") { Creates++; response = JsonValue.Create("destination"); }
            else if (command == "native_conversation_send") { Sends++; Message = args["message"]!.GetValue<string>(); Check(args["session_id"]!.GetValue<string>() == "destination" && args["attachments"] is JsonArray { Count: 0 } && args["references"] == null, "No source attachment/reference leaks into handoff."); }
            else if (command is "list_models" or "list_acp_agents") response = new JsonArray();
            else if (command == "get_settings") response = new JsonObject();
            return Task.FromResult(response);
        }
    }
}
