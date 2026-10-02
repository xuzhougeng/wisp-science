using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeModelSettingsTests
{
    private static void Check(bool value, string description)
    { if (!value) throw new InvalidOperationException(description); Console.WriteLine("PASS model settings: " + description); }
    public static async Task RunAsync()
    {
        var fake = new Fake(); using var model = new NativeSettingsEditorModel(fake, "project-models");
        var profile = NativeModelDrafts.Create(); profile["id"] = "profile-a"; profile["model"] = "exact/custom-id";
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
