using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeComposerOptionsTests
{
    public static async Task RunAsync(string fixturePath)
    {
        await DraftOptionsAsync();
        var fake = new Fake { Scoped = JsonNode.Parse(File.ReadAllText(fixturePath))!.AsObject() };
        var model = new NativeComposerOptionsModel(fake);
        model.Bind("project-a", "session-a"); model.Writable = true;
        await model.LoadAsync();
        Check(model.CanEdit && model.Session?.SessionId == "session-a", "loads conversation options without app-wide memory or failure-analysis endpoints");
        Check(model.Reviewers.Select(c => c.Id).SequenceEqual(["http:", "follow_session", "http:chat", "acp:agent"]), "reviewer uses host-filtered HTTP and ACP options");
        Check(model.Specialists.Select(c => c.Id).SequenceEqual(["", "scientist"]), "internal specialists are excluded");
        fake.Scoped["completion"] = new JsonObject { ["policy"] = "background", ["auto_resume"] = true }; await model.LoadAsync();
        Check(model.CanEdit && model.Session?.SessionId == "session-a", "older hosts that still send the removed completion policy decode");
        Check(!await model.SetToggleAsync("full_permission", true) && fake.Writes == 0, "full permission cannot enable without confirmation");
        Check(await model.SetToggleAsync("full_permission", true, true) && model.Session!.FullPermission, "confirmed permission is scoped and reread");
        Check(fake.LastArgs?["session_id"]?.GetValue<string>() == "session-a" && fake.LastProject == "project-a", "mutation carries exact project and session");
        Check(await model.SetToggleAsync("auto_review", true) && model.Session!.AutoReview, "review is saved on conversation");
        var writes = fake.Writes;
        fake.Specs[0]!["instructions"] = "latest unrelated instructions";
        Check(await model.SetReviewerAsync("acp:agent") && model.Reviewer == "acp:agent" && fake.Specs[0]!["instructions"]!.GetValue<string>() == "latest unrelated instructions", "review backend preserves latest specialist configuration");
        Check(await model.SetReviewerAsync("follow_session") && await model.SetReviewerAsync("http:chat") && model.Reviewer == "http:chat", "review supports follow session and explicit HTTP model");
        Check(await model.SetSpecialistAsync("scientist") && NativeComposerOptionsModel.S(model.Session!.Specialist, "id") == "scientist", "specialist persists before first message");
        fake.Scoped["specialist_locked"] = true; await model.LoadAsync(); writes = fake.Writes;
        Check(!await model.SetSpecialistAsync("") && fake.Writes == writes, "started conversations lock specialist");
        Check(await model.SetContextAsync("ssh:test", true) && model.Contexts!.EnabledIds.Contains("ssh:test"), "remote membership is saved");
        Check(await model.SetDefaultContextAsync("ssh:test") && model.Contexts!.DefaultContext?.ContextId == "ssh:test", "session compute default persists");
        Check(await model.SetContextAsync("ssh:test", false), "default and explicit attachment are independent");
        writes = fake.Writes;
        Check(await model.SetDefaultContextAsync("ssh:test") && model.Contexts!.EnabledIds.Contains("ssh:test") && fake.Writes == writes + 2,
            "choosing a remote default confirms default then attaches like WebView");
        Check(await model.SetDefaultContextAsync("local") && model.Contexts!.DefaultContext?.ContextId == "local" && model.Contexts.EnabledIds.Contains("ssh:test"),
            "clearing a star pins local without detaching remote resources");
        writes = fake.Writes;
        Check(!await model.SetContextAsync("local", false) && !await model.SetDefaultContextAsync("missing") && fake.Writes == writes, "invalid resource actions never dispatch");
        fake.Contexts["read_only"] = true; await model.LoadAsync();
        Check(!await model.SetContextAsync("ssh:test", false) && !await model.SetDefaultContextAsync("local") && fake.Writes == writes, "read-only resource snapshots reject mutations");
        fake.Contexts["read_only"] = false; await model.LoadAsync();
        Check(await model.SetContextAsync("ssh:test", false), "prepare independent attachment failure");
        fake.FailAttach = true; writes = fake.Writes;
        Check(!await model.SetDefaultContextAsync("ssh:test") && fake.Writes == writes + 2 && model.Error != null, "partial default action reports unconfirmed attachment");
        fake.FailAttach = false; await model.LoadAsync();
        Check(model.Contexts!.DefaultContext?.ContextId == "ssh:test" && !model.Contexts.EnabledIds.Contains("ssh:test") && fake.Writes == writes + 2,
            "reread exposes saved default without replaying the failed attachment");
        fake.Fail = true; writes = fake.Writes;
        Check(!await model.SetToggleAsync("auto_review", false) && model.Session!.AutoReview && model.Error != null && fake.Writes == writes + 1, "failed write preserves confirmed state without replay");
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>());
        Check(model.ErrorText(locale)?.StartsWith("The change was not confirmed.") == true
            && model.ErrorText(locale)!.EndsWith("lost reply"), "English failure guidance preserves the original transport detail");
        locale.Preview("zh");
        Check(model.ErrorText(locale)?.StartsWith("设置未确认保存") == true, "failure guidance follows locale changes without reissuing a write");
        Check(!await model.SetToggleAsync("auto_review", false) && fake.Writes == writes + 1, "uncertain write must be reread before another write");
        fake.Fail = false; await model.LoadAsync(); Check(model.CanEdit && fake.Writes == writes + 1, "recovery rereads only");
        model.Writable = false; await model.LoadAsync();
        Check(model.Session != null && model.Contexts != null && !model.CanEditCompute && !await model.SetToggleAsync("auto_review", true) && fake.Writes == writes + 1,
            "read-only conversations may inspect options but cannot mutate");
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
        locale.Preview("en");
        Check(model.ErrorText(locale)?.StartsWith("Could not load conversation options:") == true, "read failure has distinct localized guidance");
        fake.Scoped["session_id"] = "session-a";
        model.Bind("project-a", "session-a"); model.Writable = true; await model.LoadAsync();
        var heldDefault = fake.PendingWrite = new(); writes = fake.Writes;
        var defaultSave = model.SetDefaultContextAsync("ssh:test");
        model.Bind("project-b", "session-b");
        heldDefault.SetResult(new JsonObject { ["context_id"] = "ssh:test" });
        Check(!await defaultSave && fake.Writes == writes + 1 && model.Contexts == null,
            "navigation after a default acknowledgement prevents the follow-up attachment and stale reread");
        Console.WriteLine("Native composer options persistence, scopes, confirmation, validation, recovery and stale reply tests passed.");
    }
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    private static async Task DraftOptionsAsync()
    {
        var fake = new Fake(); var options = new NativeComposerOptionsModel(fake);
        options.Bind("draft-project", null); options.Writable = true;
        await options.LoadAsync();
        Check(options.IsDraft && options.Session == null && options.CanEdit && options.CanEditCompute,
            "a draft can inspect options without inventing a session identity");
        Check(fake.Calls.All(call => !call.Command.StartsWith("native_conversation_") && call.Project == "draft-project" && call.Args.Count == 0)
            && fake.Writes == 0, "opening draft options only reads project/global catalogs");
        Check(options.Reviewers.Select(c => c.Id).SequenceEqual(["http:", "follow_session", "http:chat", "acp:agent"]),
            "draft reviewers share chat-model filtering and ACP ordering");
        Check(options.Contexts!.EnabledIds.Length == 0 && options.Contexts.DefaultContext?.ContextId == null,
            "draft resources have no fabricated session membership or pinned default");
        Check(!await options.SetToggleAsync("auto_review", true) && !await options.SetSpecialistAsync("scientist")
            && !await options.SetDefaultContextAsync("ssh:test") && !await options.SetContextAsync("ssh:test", true) && fake.Writes == 0,
            "session-only writes are rejected until an explicit action prepares a real session");
        Check(await options.SetReviewerAsync("acp:agent") && options.IsDraft && options.Session == null && fake.Writes == 1,
            "shared reviewer configuration can change without creating a conversation");
        fake.FailDraftRead = true; var writes = fake.Writes; await options.LoadAsync();
        Check(options.Error != null && !options.CanEdit && fake.Writes == writes, "draft load errors disable writes without creating a session");
        fake.FailDraftRead = false; await options.LoadAsync();
        Check(options.CanEdit && fake.Writes == writes, "draft reload recovers through reads only");
        var held = fake.PendingDraftRead = new(); var loading = options.LoadAsync();
        options.Bind("other-project", null); held.SetResult(new JsonArray()); await loading;
        Check(options.Session == null && options.Contexts == null && options.Reviewer == "http:" && !options.Busy,
            "late draft data and prior reviewer do not leak into another project");
        fake.PendingDraftRead = null; options.Bind("draft-project", null); options.Writable = true; await options.LoadAsync();
        var heldSave = fake.PendingWrite = new(); var saving = options.SetReviewerAsync("http:chat");
        var reads = fake.Reads; options.Bind("other-project", "existing-session"); heldSave.SetResult(fake.Specs.DeepClone()); await saving;
        Check(options.Session == null && options.Contexts == null && !options.Busy && fake.Reads == reads,
            "late global reviewer acknowledgement cannot reload the next conversation");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public JsonObject Scoped = new();
        public JsonArray Specs = JsonNode.Parse("""[{"id":"reviewer","model_id":"","instructions":"original"},{"id":"scientist","name":"科学家"},{"id":"reader","name":"Reader"}]""")!.AsArray();
        public JsonObject Contexts = JsonNode.Parse("""{"contexts":[{"id":"ssh:test","kind":"ssh","label":"Test","config_json":"{}","capabilities_json":"{}"}],"enabled_ids":[],"read_only":false,"default_context":{"context_id":null}}""")!.AsObject();
        public TaskCompletionSource<JsonNode?>? PendingRead, PendingWrite;
        public TaskCompletionSource<JsonNode?>? PendingDraftRead;
        public bool FailDraftRead;
        public readonly List<(string Command, JsonObject Args, string? Project)> Calls = [];
        public bool Fail, FailAttach;
        public int Writes, Reads;
        public JsonObject? LastArgs;
        public string? LastProject;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken token = default)
        {
            Calls.Add((command, (JsonObject)args.DeepClone(), projectId));
            if (command is "native_conversation_options_set" or "native_conversation_panel_context_default" or "native_conversation_panel_context_enabled" || command.StartsWith("set_") || command.StartsWith("save_"))
            {
                Writes++; LastArgs = (JsonObject)args.DeepClone(); LastProject = projectId;
                if (PendingWrite != null) return PendingWrite.Task;
                if (Fail) throw new IOException("lost reply");
                if (FailAttach && command == "native_conversation_panel_context_enabled") throw new IOException("attachment reply lost");
            }
            else Reads++;
            JsonNode? value;
            switch (command)
            {
                case "list_models": value = JsonNode.Parse("""[{"id":"chat","model":"chat","label":"Chat"},{"id":"image","model":"gateway/gpt-image-2"},{"id":"video","model":"grok-imagine-video"},{"id":"custom-image","model":"custom","image_generation_capable":true}]"""); break;
                case "list_acp_agents": value = JsonNode.Parse("""[{"id":"agent","label":"Agent"}]"""); break;
                case "list_execution_contexts":
                    if (FailDraftRead) throw new IOException("draft context read failed");
                    return PendingDraftRead?.Task ?? Task.FromResult(Contexts["contexts"]?.DeepClone());
                case "native_conversation_options": return PendingRead?.Task ?? Task.FromResult<JsonNode?>(Scoped.DeepClone());
                case "native_conversation_options_set":
                    var change = args["change"]!; var kind = change["kind"]!.GetValue<string>();
                    if (kind is "auto_review" or "full_permission" or "delegation") Scoped[kind] = change["enabled"]!.DeepClone();
                    else if (kind == "specialist") Scoped["specialist"] = Specs.First(v => v?["id"]!.GetValue<string>() == change["id"]!.GetValue<string>())!.DeepClone();
                    value = Scoped; break;
                case "list_specialists": value = Specs; break;
                case "save_specialist_cmd": Specs[0] = args["spec"]!.DeepClone(); value = Specs; break;
                case "native_conversation_panel_side_chat_options":
                    Check(projectId == "project-a" && args["session_id"]?.GetValue<string>() == "session-a", "reviewer options use the selected project and session");
                    value = JsonNode.Parse("""[{"id":"chat","label":"Chat","kind":"http","active":true},{"id":"agent","label":"Agent","kind":"acp","active":false}]"""); break;
                case "native_conversation_panel_contexts": value = Contexts; break;
                case "list_ssh_hosts": value = JsonNode.Parse("""[{"alias":"test"}]"""); break;
                case "get_default_execution_context": value = null; break;
                case "native_conversation_panel_context_enabled":
                    Contexts["enabled_ids"] = args["enabled"]!.GetValue<bool>() ? new JsonArray(args["context_id"]!.DeepClone()) : new JsonArray(); value = Contexts["enabled_ids"]; break;
                case "native_conversation_panel_context_default":
                    Contexts["default_context"] = new JsonObject { ["context_id"] = args["context_id"]!.DeepClone() }; value = Contexts["default_context"]; break;
                default: throw new InvalidOperationException(command);
            }
            return Task.FromResult(value?.DeepClone());
        }
    }
}
