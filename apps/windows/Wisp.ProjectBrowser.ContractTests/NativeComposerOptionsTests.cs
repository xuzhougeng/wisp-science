using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeComposerOptionsTests
{
    public static async Task RunAsync(string fixturePath)
    {
        var fake = new Fake { Scoped = JsonNode.Parse(File.ReadAllText(fixturePath))!.AsObject() };
        var model = new NativeComposerOptionsModel(fake);
        model.Bind("project-a", "session-a"); model.Writable = true;
        await model.LoadAsync();
        Check(model.CanEdit && model.Session?.SessionId == "session-a" && model.Failure?.FailureRateThreshold == 30 && model.MemoryEnabled, "loads real scopes and default state");
        Check(model.Reviewers.Select(c => c.Id).SequenceEqual(["http:", "follow_session", "http:chat", "acp:agent"]), "reviewer uses host-filtered HTTP and ACP options");
        Check(model.Specialists.Select(c => c.Id).SequenceEqual(["", "scientist"]), "internal specialists are excluded");
        Check(!await model.SetToggleAsync("full_permission", true) && fake.Writes == 0, "full permission cannot enable without confirmation");
        Check(!await model.SetCompletionAsync("background", true) && fake.Writes == 0, "completion is disabled without delegation");
        Check(await model.SetToggleAsync("full_permission", true, true) && model.Session!.FullPermission, "confirmed permission is scoped and reread");
        Check(fake.LastArgs?["session_id"]?.GetValue<string>() == "session-a" && fake.LastProject == "project-a", "mutation carries exact project and session");
        Check(await model.SetToggleAsync("delegation", true) && await model.SetCompletionAsync("background", true)
            && model.Session!.Completion.AutoResume, "background completion enables auto resume");
        Check(await model.SetCompletionAsync("inline", true) && !model.Session!.Completion.AutoResume, "inline clears auto resume");
        Check(await model.SetToggleAsync("auto_review", true) && model.Session!.AutoReview, "review is saved on conversation");
        var writes = fake.Writes;
        Check(!await model.SetFailureAsync(true, 0, 2) && !await model.SetFailureAsync(true, 30, 101) && fake.Writes == writes, "invalid thresholds cannot write");
        Check(await model.SetFailureAsync(true, 45, 3) && model.Failure == new ComposerFailureAnalysis(true, 45, 3), "global failure analysis survives reload");
        Check(await model.SetMemoryAsync(false) && !model.MemoryEnabled, "memory reflects confirmed global value");
        fake.Specs[0]!["instructions"] = "latest unrelated instructions";
        Check(await model.SetReviewerAsync("acp:agent") && model.Reviewer == "acp:agent" && fake.Specs[0]!["instructions"]!.GetValue<string>() == "latest unrelated instructions", "review backend preserves latest specialist configuration");
        Check(await model.SetReviewerAsync("follow_session") && await model.SetReviewerAsync("http:chat") && model.Reviewer == "http:chat", "review supports follow session and explicit HTTP model");
        Check(await model.SetSpecialistAsync("scientist") && NativeComposerOptionsModel.S(model.Session!.Specialist, "id") == "scientist", "specialist persists before first message");
        fake.Scoped["specialist_locked"] = true; await model.LoadAsync(); writes = fake.Writes;
        Check(!await model.SetSpecialistAsync("") && fake.Writes == writes, "started conversations lock specialist");
        Check(await model.SetContextAsync("ssh:test", true) && model.Contexts!.EnabledIds.Contains("ssh:test"), "remote membership is saved");
        Check(await model.SetDefaultContextAsync("ssh:test") && model.Contexts!.DefaultContext?.ContextId == "ssh:test", "session compute default persists");
        fake.Fail = true; writes = fake.Writes;
        Check(!await model.SetToggleAsync("auto_review", false) && model.Session!.AutoReview && model.Error != null && fake.Writes == writes + 1, "failed write preserves confirmed state without replay");
        Check(!await model.SetToggleAsync("auto_review", false) && fake.Writes == writes + 1, "uncertain write must be reread before another write");
        fake.Fail = false; await model.LoadAsync(); Check(model.CanEdit && fake.Writes == writes + 1, "recovery rereads only");
        model.Writable = false; Check(!await model.SetMemoryAsync(true) && fake.Writes == writes + 1, "read-only conversations cannot mutate");
        model.Writable = true;
        var hold = fake.PendingRead = new(); var loading = model.LoadAsync();
        model.Bind("project-b", "session-b"); hold.SetResult(fake.Scoped.DeepClone()); await loading;
        Check(model.Session == null && !model.Busy, "late load cannot populate another conversation");
        fake.PendingRead = null; model.Bind("project-a", "session-a"); model.Writable = true; await model.LoadAsync();
        var pendingWrite = fake.PendingWrite = new(); var saving = model.SetToggleAsync("auto_review", false);
        var reads = fake.Reads; model.Bind("project-b", "session-b"); pendingWrite.SetResult(fake.Scoped.DeepClone()); await saving;
        Check(model.Session == null && !model.Busy && fake.Reads == reads, "late write cannot reread or overwrite the next conversation");
        fake.PendingWrite = null; fake.Scoped["session_id"] = "wrong";
        model.Bind("project-a", "session-a"); await model.LoadAsync();
        Check(model.Session == null && model.Error != null, "wrong-session response is rejected");
        Console.WriteLine("Native composer options persistence, scopes, confirmation, validation, recovery and stale reply tests passed.");
    }
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public JsonObject Scoped = new();
        public JsonArray Specs = JsonNode.Parse("""[{"id":"reviewer","model_id":"","instructions":"original"},{"id":"scientist","name":"科学家"},{"id":"reader","name":"Reader"}]""")!.AsArray();
        private JsonNode failure = JsonNode.Parse("""{"enabled":true,"failure_rate_threshold":30,"minimum_failures":2}""")!;
        private bool memory = true;
        private JsonObject contexts = JsonNode.Parse("""{"contexts":[{"id":"ssh:test","kind":"ssh","label":"Test","config_json":"{}","capabilities_json":"{}"}],"enabled_ids":[],"read_only":false,"default_context":{"context_id":null}}""")!.AsObject();
        public TaskCompletionSource<JsonNode?>? PendingRead, PendingWrite;
        public bool Fail;
        public int Writes, Reads;
        public JsonObject? LastArgs;
        public string? LastProject;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken token = default)
        {
            if (command is "native_conversation_options_set" or "native_conversation_panel_context_default" or "native_conversation_panel_context_enabled" || command.StartsWith("set_") || command.StartsWith("save_"))
            {
                Writes++; LastArgs = (JsonObject)args.DeepClone(); LastProject = projectId;
                if (PendingWrite != null) return PendingWrite.Task;
                if (Fail) throw new IOException("lost reply");
            }
            else Reads++;
            JsonNode? value;
            switch (command)
            {
                case "native_conversation_options": return PendingRead?.Task ?? Task.FromResult<JsonNode?>(Scoped.DeepClone());
                case "native_conversation_options_set":
                    var change = args["change"]!; var kind = change["kind"]!.GetValue<string>();
                    if (kind is "auto_review" or "full_permission" or "delegation") Scoped[kind] = change["enabled"]!.DeepClone();
                    else if (kind == "completion") Scoped["completion"] = new JsonObject { ["policy"] = change["policy"]!.DeepClone(), ["auto_resume"] = change["auto_resume"]!.DeepClone() };
                    else if (kind == "specialist") Scoped["specialist"] = Specs.First(v => v?["id"]!.GetValue<string>() == change["id"]!.GetValue<string>())!.DeepClone();
                    value = Scoped; break;
                case "get_auto_failure_analysis_settings": value = failure; break;
                case "set_auto_failure_analysis_settings": failure = args["settings"]!.DeepClone(); value = failure; break;
                case "set_memory_enabled": memory = args["enabled"]!.GetValue<bool>(); goto case "get_memory_view";
                case "get_memory_view": value = new JsonObject { ["enabled"] = memory }; break;
                case "list_specialists": value = Specs; break;
                case "save_specialist_cmd": Specs[0] = args["spec"]!.DeepClone(); value = Specs; break;
                case "native_conversation_panel_side_chat_options":
                    Check(projectId == "project-a" && args["session_id"]?.GetValue<string>() == "session-a", "reviewer options use the selected project and session");
                    value = JsonNode.Parse("""[{"id":"chat","label":"Chat","kind":"http","active":true},{"id":"agent","label":"Agent","kind":"acp","active":false}]"""); break;
                case "native_conversation_panel_contexts": value = contexts; break;
                case "get_default_execution_context": value = null; break;
                case "native_conversation_panel_context_enabled":
                    contexts["enabled_ids"] = args["enabled"]!.GetValue<bool>() ? new JsonArray(args["context_id"]!.DeepClone()) : new JsonArray(); value = contexts["enabled_ids"]; break;
                case "native_conversation_panel_context_default":
                    contexts["default_context"] = new JsonObject { ["context_id"] = args["context_id"]!.DeepClone() }; value = contexts["default_context"]; break;
                default: throw new InvalidOperationException(command);
            }
            return Task.FromResult(value?.DeepClone());
        }
    }
}
