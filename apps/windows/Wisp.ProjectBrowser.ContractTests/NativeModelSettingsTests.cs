using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeModelSettingsTests
{
    private static void Check(bool value, string description)
    { if (!value) throw new InvalidOperationException(description); Console.WriteLine("PASS model settings: " + description); }
    public static async Task RunAsync()
    {
        await EditorParityAsync();
        var fake = new Fake(); using var model = new NativeSettingsEditorModel(fake, "project-models");
        var profile = NativeModelDrafts.Create(); profile["id"] = "profile-a"; profile["model"] = "exact/custom-id";
        Check(NativeModelPresentation.IsChat(profile) && !NativeModelPresentation.CanDelete(profile, [profile]),
            "the last chat model has no destructive list action");
        var imageProfile = NativeModelDrafts.Create(name: "vendor/gpt-image-2"); imageProfile["id"] = "image";
        Check(!NativeModelPresentation.IsChat(imageProfile) && !NativeModelPresentation.CanDelete(profile, [profile, imageProfile])
            && NativeModelPresentation.CanDelete(imageProfile, [profile, imageProfile]), "image models cannot stand in for the last chat model");
        imageProfile["model"] = "gpt-image-2-future";
        Check(NativeModelPresentation.IsChat(imageProfile), "media role lookup never absorbs a longer sibling ID");
        imageProfile["image_generation_capable"] = true;
        Check(!NativeModelPresentation.IsChat(imageProfile), "catalog capability marks media-only profiles");
        profile["label"] = "Readable name";
        Check(NativeModelPresentation.Label(profile) == "Readable name" && NativeModelPresentation.Subtitle(profile) == "exact/custom-id", "model rows preserve the label and distinct model ID");
        profile["use_for_expert"] = true;
        profile["key"] = "synthetic-key"; profile["use_for_vision"] = true; profile["use_for_image_generation"] = false; profile["use_for_video_generation"] = false;
        profile["future_option"] = new JsonObject { ["keep"] = true }; profile["send_session_id"] = null;
        model.Edit(profile, "save_model", "profile"); await model.SaveAsync();
        var saved = fake.Calls.Last();
        Check(saved.Command == "save_model" && saved.Args["key"]!.GetValue<string>() == "synthetic-key" && !saved.Args["profile"]!.AsObject().ContainsKey("key"),
            "model key is sent only as a host keyring argument, never inside the persisted profile");
        Check(saved.Args["useForVision"]!.GetValue<bool>() && !saved.Args["useForImageGeneration"]!.GetValue<bool>() && !saved.Args["useForVideoGeneration"]!.GetValue<bool>(),
            "all three model assignments use explicit camelCase top-level arguments");
        Check(saved.Args["profile"]!["future_option"]!["keep"]!.GetValue<bool>() && saved.Args["profile"]!["send_session_id"] == null && model.Draft == null,
            "model save preserves unknown options and nullable header policy and clears the local secret draft");
        Check(saved.Args["profile"]!["use_for_expert"]!.GetValue<bool>(), "the expert checkbox persists through the existing profile contract");
        model.Edit(profile, "save_model", "profile"); fake.Handler = (_, _) => throw new IOException("synthetic failure"); var count = fake.Calls.Count;
        Check(!await model.SaveAsync() && fake.Calls.Count == count + 1 && model.Draft!["key"]!.GetValue<string>() == "synthetic-key",
            "failed model save retains input without replaying credentials");
        model.Draft!["key"] = ""; fake.Handler = null; await model.SaveAsync();
        Check(fake.Calls.Last().Args["key"]!.GetValue<string>() == "", "blank model key retains the host's existing credential semantics");
        var settings = JsonNode.Parse("{\"provider\":\"openai\",\"api_url\":\"old\",\"model\":\"old-model\",\"max_tokens\":10,\"workspace_dir\":\"C:/synthetic\",\"locale\":\"zh\"}")!.AsObject();
        fake.Handler = (command, _) => Task.FromResult<JsonNode?>(command == "get_settings" ? settings : JsonValue.Create("ok"));
        model.Edit(profile, "save_model", "profile");
        Check(await model.TestModelAsync(_ => { }) && model.Draft != null, "API testing retains the unsaved model draft");
        var test = fake.Calls.Last();
        Check(test.Command == "validate_settings" && test.Project == "project-models" && test.Args["profileId"]!.GetValue<string>() == "profile-a"
            && test.Args["settings"]!["model"]!.GetValue<string>() == "exact/custom-id" && test.Args["settings"]!["workspace_dir"]!.GetValue<string>() == "C:/synthetic"
            && !test.Args["settings"]!.AsObject().ContainsKey("key") && settings["model"]!.GetValue<string>() == "old-model",
            "API test combines profile fields with current settings without mutating settings or persisting keys");
        var second = NativeModelDrafts.Create(); second["id"] = "profile-b"; var third = NativeModelDrafts.Create(); third["id"] = "profile-c";
        var ids = NativeModelDrafts.Reorder([profile, second, third], "profile-b", 1);
        Check(ids.Select(n => n!.GetValue<string>()).SequenceEqual(["profile-a", "profile-c", "profile-b"]), "reordering sends every profile id exactly once");
        var blocked = false; try { NativeModelDrafts.Reorder([profile, second], "profile-a", -1); } catch (ArgumentOutOfRangeException) { blocked = true; }
        Check(blocked, "reordering outside the full model list is rejected");
        var pending = new TaskCompletionSource<JsonNode?>(); fake.Handler = (_, _) => pending.Task;
        var testing = model.TestModelAsync(_ => throw new Exception("late reply applied")); count = fake.Calls.Count;
        model.Dispose(); pending.SetResult(settings); await testing;
        Check(fake.Calls.Count == count, "closing while settings load prevents a later API test request");

        var auth = new Fake { Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonObject { ["text"] = "\u001b[31mLogin\u001b[0m\n\u001b]0;title\u0007Prompt", ["running"] = true }) };
        using var terminal = new NativeAuthTerminalModel(auth, "auth-project", "auth-terminal");
        Check(await terminal.ReadAsync() && terminal.Output == "Login\nPrompt" && terminal.CanWrite, "auth snapshot strips terminal control sequences and enables input only after a running snapshot");
        auth.Handler = (_, _) => Task.FromResult<JsonNode?>(null);
        Check(await terminal.WriteAsync("synthetic-answer\r") && auth.Calls.Last().Command == "write_terminal"
            && auth.Calls.Last().Project == "auth-project" && auth.Calls.Last().Args["sessionId"]!.GetValue<string>() == "auth-terminal",
            "authentication input remains bound to the host-created terminal and project");
        auth.Handler = (_, _) => throw new IOException("lost acknowledgement"); count = auth.Calls.Count;
        await terminal.WriteAsync("synthetic-answer\r"); await terminal.WriteAsync("synthetic-answer\r");
        Check(terminal.InputUncertain && auth.Calls.Count == count + 1, "uncertain authentication input is not automatically or repeatedly sent");
        terminal.AcknowledgeInput(); auth.Handler = (_, _) => Task.FromResult<JsonNode?>(null);
        Check(await terminal.WriteAsync("\u0003"), "explicit acknowledgement enables a later terminal interrupt");
        Check(await terminal.CloseAsync() && terminal.Closed && !terminal.CanWrite && auth.Calls.Last().Command == "close_terminal", "confirmed close disposes authentication input and polling state");
        var delayed = new TaskCompletionSource<JsonNode?>(); auth.Handler = (_, _) => delayed.Task;
        using var closedTerminal = new NativeAuthTerminalModel(auth, "auth-project", "other-terminal");
        var read = closedTerminal.ReadAsync(); closedTerminal.Dispose(); delayed.SetResult(new JsonObject { ["text"] = "late", ["running"] = true }); await read;
        Check(closedTerminal.Output.Length == 0 && !closedTerminal.Running, "late authentication snapshots cannot repopulate a closed view");
    }
    private static async Task EditorParityAsync()
    {
        var oldProfile = new JsonObject { ["provider"] = "", ["max_tokens"] = 0, ["context_window"] = 1 };
        var editDraft = NativeModelDrafts.EditDraft(oldProfile);
        Check(editDraft["max_tokens"]!.GetValue<int>() == 8192 && editDraft["context_window"]!.GetValue<int>() == 128000
            && oldProfile["max_tokens"]!.GetValue<int>() == 0, "legacy zero limits get the same editor defaults without mutating the saved profile");
        Check(NativeModelDrafts.ProviderValue("") == "openai" && NativeModelDrafts.ProviderValue("openai-responses") == "openai_responses"
            && NativeModelDrafts.ProviderValue("anthropic") == "anthropic", "empty and legacy protocol names retain the WebView display selection");
        var profile = NativeModelDrafts.Create(name: "custom/model"); profile["id"] = "one";
        var second = NativeModelDrafts.Create(); second["id"] = "subscription";
        var third = NativeModelDrafts.Create(); third["id"] = "three";
        Check(NativeModelDrafts.ReorderTo([profile, second, third], "one", "three").Select(n => n!.GetValue<string>()).SequenceEqual(["subscription", "three", "one"])
            && NativeModelDrafts.ReorderTo([profile, second, third], "three", "one").Select(n => n!.GetValue<string>()).SequenceEqual(["three", "one", "subscription"]),
            "dragging in either direction retains every model including other categories");
        Check(NativeModelDrafts.ReorderTo([profile, second], "one", "one").Count == 2, "self drop preserves the complete list");
        foreach (var invalid in new[] { new[] { profile, second }, new[] { profile, profile } })
        {
            var blocked = false; try { NativeModelDrafts.ReorderTo(invalid, "one", "missing"); } catch (ArgumentException) { blocked = true; }
            Check(blocked, "stale drop IDs cannot submit an incomplete or duplicate order");
        }
        var duplicateBlocked = false; try { NativeModelDrafts.ReorderTo([profile, profile], "one", "one"); } catch (ArgumentException) { duplicateBlocked = true; }
        Check(duplicateBlocked, "duplicate model IDs cannot be dragged");
        NativeModelDrafts.SetRole(profile, "use_for_vision", true);
        Check(profile["supports_vision"]!.GetValue<bool>(), "assigning image analysis enables image input");
        NativeModelDrafts.SetRole(profile, "supports_vision", false);
        Check(!profile["use_for_vision"]!.GetValue<bool>(), "disabling image input clears image analysis");
        NativeModelDrafts.SetRole(profile, "use_for_image_generation", true);
        Check(NativeModelPresentation.IsImage(profile) && !profile["use_for_vision"]!.GetValue<bool>() && !profile["use_for_video_generation"]!.GetValue<bool>(),
            "image generation clears conflicting analysis and video assignments");
        NativeModelDrafts.SetRole(profile, "use_for_video_generation", true);
        Check(!profile["use_for_image_generation"]!.GetValue<bool>() && NativeModelPresentation.IsVideo(profile), "video generation clears the image assignment");
        NativeModelDrafts.SelectModel(profile, "vendor/grok-imagine-video-1.5-preview");
        Check(NativeModelPresentation.IsVideoId(profile) && !profile["supports_vision"]!.GetValue<bool>(), "exact video IDs coordinate capability fields");
        NativeModelDrafts.SelectModel(profile, "vendor/grok-imagine-video-1.5-preview-next");
        Check(!NativeModelPresentation.IsVideoId(profile), "longer video ID siblings are not inferred from prefixes");
        var args = NativeModelDrafts.AcpArguments("--config\r\n  C:/path with spaces  \r\n\r\n");
        Check(args.Select(n => n!.GetValue<string>()).SequenceEqual(["--config", "  C:/path with spaces  ", "", ""]),
            "ACP argument boundaries preserve whitespace and empty arguments across Windows newlines");
        var fake = new Fake(); using var editor = new NativeSettingsEditorModel(fake, "model-project");
        var agent = new JsonObject { ["id"] = "", ["label"] = "  Synthetic  ", ["command"] = "  fake-agent  ", ["args"] = args, ["future"] = true };
        editor.Edit(agent, "save_acp_agent", "profile");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonObject());
        Check(!await editor.SaveAsync() && editor.Draft != null && !editor.SaveConfirmed && editor.Values["list_acp_agents"] == null,
            "a malformed save reply leaves the agent draft open without a success notice");
        var requestedId = fake.Calls.Last().Args["profile"]!["id"]!.GetValue<string>();
        var sibling = new JsonObject { ["id"]="existing", ["label"]="Other", ["command"]="other" };
        fake.Handler = (_, data) => Task.FromResult<JsonNode?>(new JsonArray(sibling.DeepClone(), data["profile"]!.DeepClone()));
        Check(await editor.SaveAsync() && editor.Draft == null && editor.SaveConfirmed, "a confirmed reply closes the ACP editor");
        var saved = fake.Calls.Last().Args["profile"]!;
        Check(Guid.TryParse(requestedId, out _) && requestedId == saved["id"]!.GetValue<string>() &&
            editor.Values["list_acp_agents"] is JsonArray { Count: 2 } list && JsonNode.DeepEquals(list[0], sibling)
            && JsonNode.DeepEquals(list[1], saved), "retry reuses the new agent ID and mounts the host's returned list");
        var missing = new JsonArray(sibling.DeepClone());
        Check(NativeModelDrafts.ConfirmedAcpList(missing, saved.AsObject()) == null, "an unrelated saved profile cannot confirm this draft");
        Check(saved["label"]!.GetValue<string>() == "Synthetic" && saved["command"]!.GetValue<string>() == "fake-agent"
            && JsonNode.DeepEquals(saved["args"], args) && saved["future"]!.GetValue<bool>() && agent["label"]!.GetValue<string>() == "  Synthetic  ",
            "ACP save trims required fields but preserves arguments, unknown fields and the source");
        agent["label"] = " "; editor.Edit(agent, "save_acp_agent", "profile"); var count = fake.Calls.Count;
        Check(!await editor.SaveAsync() && fake.Calls.Count == count && editor.Draft != null, "blank ACP label is rejected without a host write");
        editor.Discard();
        Check(editor.Error == null && editor.Draft == null && !editor.SaveConfirmed, "cancel clears ACP errors without writing");
        profile = NativeModelDrafts.Create(name: "exact/custom-id"); profile["api_url"] = " https://opencode.ai/zen/ "; profile["endpoint_suffix"] = " /v1/ ";
        Check(NativeModelDrafts.SessionIdentityEnabled(profile), "default session header follows only the OpenCode Zen endpoint");
        profile["api_url"] = "https://opencode.ai.evil.invalid/zen"; Check(!NativeModelDrafts.SessionIdentityEnabled(profile), "lookalike endpoint hosts do not enable session identity");
        profile["api_url"] = " https://example.invalid/base/ ";
        var pending = new TaskCompletionSource<JsonNode?>(); fake.Handler = (_, _) => pending.Task;
        editor.Edit(profile, "save_model", "profile"); JsonObject? applied = null;
        var reading = editor.ReadModelCatalogAsync(value => applied = value);
        Check(fake.Calls.Last().Command == "model_catalog_lookup" && fake.Calls.Last().Args["apiUrl"]!.GetValue<string>() == "https://example.invalid/base/v1"
            && fake.Calls.Last().Args["model"]!.GetValue<string>() == "exact/custom-id" && !editor.Busy, "catalog lookup joins the suffix and sends only the exact model ID without locking the form");
        editor.Draft!["model"] = "exact/custom-id-new";
        pending.SetResult(new JsonObject { ["max_tokens"] = 1 }); await reading;
        Check(applied == null, "a catalog reply for the previous model cannot update the current form");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonObject { ["max_tokens"] = 42, ["efforts"] = new JsonArray("low") });
        await editor.ReadModelCatalogAsync(value => applied = value);
        Check(applied?["max_tokens"]?.GetValue<int>() == 42 && fake.Calls.All(call => call.Command is "save_acp_agent" or "model_catalog_lookup"),
            "current catalog metadata is delivered without testing endpoints or saving model settings");
        pending = new(); fake.Handler = (_, _) => pending.Task; applied = null;
        reading = editor.ReadModelCatalogAsync(value => applied = value); editor.Discard(); editor.Edit(profile, "save_model", "profile");
        pending.SetResult(new JsonObject { ["max_tokens"] = 99 }); await reading;
        Check(applied == null, "an identical reopened draft cannot receive a prior editor's catalog result");
        pending = new(); fake.Handler = (_, _) => pending.Task;
        reading = editor.ReadModelCatalogAsync(value => applied = value); editor.Dispose(); pending.SetResult(new JsonObject { ["max_tokens"] = 100 }); await reading;
        Check(applied == null, "closed model editors ignore late catalog replies");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public Func<string, JsonObject, Task<JsonNode?>>? Handler;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            return Handler?.Invoke(command, arguments) ?? Task.FromResult<JsonNode?>(null);
        }
    }
}
