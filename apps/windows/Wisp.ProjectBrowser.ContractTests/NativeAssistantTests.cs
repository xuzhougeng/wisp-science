using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeAssistantTests
{
    private static void Check(bool value, string reason) { if (!value) throw new InvalidOperationException(reason); }
    public static async Task RunAsync()
    {
        var layout = new NativeAssistantLayout(); layout.Resize(1103);
        Check(layout.Left && layout.Right && !layout.Narrow, "desktop starts with both assistant rails");
        layout.Toggle(true); layout.Resize(600);
        Check(!layout.Left && !layout.Right && layout.Narrow, "compact mode starts with closed drawers");
        layout.Toggle(true); layout.Toggle(false);
        Check(layout.CloseDrawer() && layout.Left && !layout.Right, "Escape closes only the visually topmost right drawer");
        Check(layout.CloseDrawer() && !layout.Left && !layout.CloseDrawer(), "second Escape closes the remaining drawer, then yields to the workspace");
        layout.Resize(1103); Check(!layout.Left && layout.Right, "returning to desktop restores its rail choices");

        var saved = (Projects: false, Calendar: true); var saves = 0;
        var persisted = new NativeAssistantLayout(saved.Projects, saved.Calendar, (projects, calendar) => { saved = (projects, calendar); saves++; });
        persisted.Resize(1103); persisted.Toggle(false);
        Check(saved == (false, false) && saves == 1, "desktop changes persist both rail choices");
        persisted.Resize(600); persisted.Toggle(true); persisted.Toggle(false); persisted.CloseDrawer();
        Check(saved == (false, false) && saves == 1, "temporary drawers and Escape never overwrite desktop preferences");
        var restarted = new NativeAssistantLayout(saved.Projects, saved.Calendar); restarted.Resize(600); restarted.Resize(1103);
        Check(!restarted.Left && !restarted.Right, "a new assistant window restores the saved desktop choices");
        var oldSettings = JsonSerializer.Deserialize<Wisp.Science.Preview.PreviewSettings>("{}");
        Check(oldSettings is { AssistantProjectsVisible: true, AssistantCalendarVisible: true }, "older settings default to visible assistant rails");
        var settingsJson = JsonSerializer.Serialize(new Wisp.Science.Preview.PreviewSettings { AssistantProjectsVisible = false, AssistantCalendarVisible = false });
        Check(JsonSerializer.Deserialize<Wisp.Science.Preview.PreviewSettings>(settingsJson) is { AssistantProjectsVisible: false, AssistantCalendarVisible: false }, "closed rails survive settings serialization");

        var transport = new Fake(); var model = new NativeAssistantModel(transport);
        await model.OpenAsync();
        Check(model.Workspace?.Projects.Single().Id == "p" && model.Conversation.Snapshot?.SessionId == NativeAssistantClient.SessionId,
            "assistant binds its fixed frame, not a regular project conversation");
        Check(transport.Calls.All(call => call.Command is not "native_conversation_create"), "opening never creates a normal conversation");
        Check(transport.Calls.Where(call => call.Command.StartsWith("native_assistant_")).All(call => call.Project == null), "assistant control-plane calls are global");
        model.SelectProject("p"); model.DraftDay(false);
        Check(model.Conversation.Draft.Contains(model.Day.ToString("yyyy-MM-dd")) && transport.Sent == null, "Plan this day only fills a draft");
        var added = await model.Conversation.AttachFilesAsync(["notes.csv", "notes.csv", "report.pdf"]);
        Check(added == 2 && model.Conversation.Attachments.Length == 2 && transport.Attachments.Count == 2, "one picker selection stages multiple distinct files");
        Check(transport.Attachments.All(a => a.Project == NativeAssistantClient.ProjectId && a.Session == NativeAssistantClient.SessionId),
            "selected project context never changes the assistant attachment destination");
        await model.Conversation.SendAsync();
        Check(transport.Sent?["references"]?[0]?["id"]?.GetValue<string>() == "p" && model.SelectedProjectId == "p", "selected context is sent and remains selected for the next message");
        Check(transport.Sent?["attachments"]?.AsArray().Count == 2 && model.Conversation.Attachments.Length == 0, "accepted send consumes the staged batch");
        transport.FailAttachment = "bad.csv";
        added = await model.Conversation.AttachFilesAsync(["keep.csv", "bad.csv", "never.csv"]);
        Check(added == 1 && model.Conversation.Attachments.Single().Name == "keep.csv" && model.Conversation.OperationError != null
            && transport.Attachments.All(a => a.Path != "never.csv"), "batch failure preserves accepted files and stops without replay or continuing");
        model.Conversation.RemoveAttachment("uploads/keep.csv");
        transport.FailAttachment = null; transport.PendingAttachment = new();
        var attaching = model.Conversation.AttachFilesAsync(["late.csv", "not-after-close.csv"]);
        model.Close(); transport.PendingAttachment.SetResult(new JsonObject { ["path"] = "uploads/late.csv", ["name"] = "late.csv" });
        Check(await attaching == 0 && transport.Attachments.All(a => a.Path != "not-after-close.csv"), "closing while a copy is pending cannot continue the batch");
        transport.PendingAttachment = null; await model.OpenAsync();
        Check(model.Conversation.Attachments.Length == 0, "late attachment completion cannot repopulate a reopened assistant");
        model.Conversation.ReportAttachmentSelectionError("Picker unavailable"); await model.Conversation.RefreshAsync();
        Check(model.Conversation.OperationError == "Picker unavailable", "polling does not erase a picker failure");
        model.SelectProject("unknown"); Check(model.SelectedProjectId == null, "unverified project ids cannot be selected");
        model.SelectProject("p"); transport.FailRead = true; await model.ReloadAsync();
        model.Conversation.Draft = "Do not send with stale context";
        Check(model.Workspace == null && model.SelectedProjectId == null && !model.Conversation.CanSend && !model.Conversation.CanCompose,
            "failed authoritative visibility read clears context and blocks sending");
        transport.FailRead = false; await model.ReloadAsync();
        Check(model.Conversation.CanSend, "explicit successful reload restores the draft without sending it");
        var pending = new TaskCompletionSource<JsonNode?>(); transport.Pending = pending;
        var old = model.SelectDayAsync(model.Day.AddDays(-1));
        var oldDay = model.Day; transport.Pending = null; await model.SelectDayAsync(oldDay.AddDays(-1));
        pending.SetResult(Fake.Workspace(oldDay)); await old;
        Check(model.Workspace?.Day == model.Day.ToString("yyyy-MM-dd"), "late calendar navigation cannot restore an older workspace");
        transport.Pending = new(); var closing = model.ReloadAsync(); model.Close(); transport.Pending.SetResult(Fake.Workspace(model.Day)); await closing;
        Check(!model.Active && model.Workspace == null && !model.Conversation.ContextReady, "closing invalidates outstanding reads");
        transport.Pending = null; transport.InvalidProject = true; await model.OpenAsync();
        Check(model.Workspace == null && model.Error != null, "unexpected hidden assistant identity is rejected in project list");
        transport.InvalidProject = false; await model.OpenAsync();
        Check(model.Conversation.Draft == "Do not send with stale context", "reopening restores the unsent draft without an extra normal session");
        Console.WriteLine("Assistant context, visibility and responsive navigation checks passed.");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public readonly List<(string Command, string? Project)> Calls = [];
        public bool FailRead, InvalidProject;
        public TaskCompletionSource<JsonNode?>? Pending;
        public TaskCompletionSource<JsonNode?>? PendingAttachment;
        public string? FailAttachment;
        public readonly List<(string? Project, string? Session, string Path)> Attachments = [];
        public JsonObject? Sent;
        private ulong sequence;
        public static JsonObject Workspace(DateTime day) => new()
        {
            ["day"] = day.ToString("yyyy-MM-dd"),
            ["projects"] = new JsonArray(new JsonObject { ["id"] = "p", ["name"] = "Research", ["description"] = "" }), ["plan"] = new JsonArray()
        };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, projectId)); JsonNode? result = null;
            switch (command)
            {
                case "native_assistant_open": result = new JsonObject { ["project_id"] = NativeAssistantClient.ProjectId, ["session_id"] = NativeAssistantClient.SessionId }; break;
                case "native_assistant_workspace":
                    if (FailRead) throw new IOException("Visibility unavailable");
                    if (Pending != null) return Pending.Task;
                    result = Workspace(DateTime.Parse(args["day"]!.GetValue<string>()));
                    if (InvalidProject) result["projects"]![0]!["id"] = NativeAssistantClient.ProjectId;
                    break;
                case "native_conversation_snapshot": result = JsonSerializer.SerializeToNode(new ConversationSnapshot(ConversationSnapshot.SchemaId,
                    "host", ++sequence, projectId!, NativeAssistantClient.SessionId, [], null, false, false, false, "", null, null, [], ComposerReferences: true), ConversationSnapshot.JsonOptions); break;
                case "native_conversation_send": Sent = (JsonObject)args.DeepClone(); break;
                case "native_conversation_attach":
                    var path = args["path"]!.GetValue<string>();
                    Attachments.Add((projectId, args["session_id"]?.GetValue<string>(), path));
                    if (path == FailAttachment) throw new IOException("Copy result unknown");
                    if (PendingAttachment != null) return PendingAttachment.Task;
                    result = new JsonObject { ["path"] = "uploads/" + path, ["name"] = path }; break;
                case "native_research_calendar": case "list_models": case "list_acp_agents": result = new JsonArray(); break;
            }
            return Task.FromResult(result);
        }
    }
}
