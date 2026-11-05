using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSubagentModelTests
{
    public static async Task RunAsync()
    {
        var host = new Fake();
        using var model = new NativeSettingsEditorModel(host, "project-a");
        Check(!await model.SetSubagentModelAsync("chat") && host.Writes == 0, "no mutation before the default and catalog are loaded");
        await model.LoadAsync("get_subagent_model", "list_models");
        Check(model.Values["get_subagent_model"]!.GetValue<string>() == "missing", "restored profile IDs are preserved");
        var options = NativeSpecialists.SubagentOptions("missing", host.Models.OfType<JsonObject>());
        Check(options.Select(o => o.Id).SequenceEqual(["", "chat", "missing"]) && !options[^1].Available,
            "only chat profiles are offered; unavailable saved bindings remain visible");
        Check(!await model.SetSubagentModelAsync("image") && host.Writes == 0, "non-chat models never reach the host");
        host.Fail = true;
        Check(!await model.SetSubagentModelAsync("chat") && model.Values["get_subagent_model"]!.GetValue<string>() == "missing" && host.Writes == 1,
            "failed save retains the acknowledged default without retrying");
        host.Fail = false; host.Pending = new();
        var saving = model.SetSubagentModelAsync("chat");
        Check(model.Busy && !await model.SetSubagentModelAsync("") && host.Writes == 2, "pending save blocks duplicate writes");
        host.Pending.SetResult(JsonValue.Create("chat")); await saving; host.Pending = null;
        Check(model.Values["get_subagent_model"]!.GetValue<string>() == "chat" && host.Project == "project-a", "successful response updates the displayed default under the originating scope");
        Check(await model.SetSubagentModelAsync("") && model.Values["get_subagent_model"]!.GetValue<string>() == "", "follow-conversation sends and acknowledges the empty ID");
        var writes = host.Writes;
        Check(await model.SetSubagentModelAsync("") && host.Writes == writes, "selecting the current default is read-only");
        host.Pending = new(); saving = model.SetSubagentModelAsync("chat");
        host.Pending.SetResult(null); Check(!await saving && model.Values["get_subagent_model"]!.GetValue<string>() == "", "a malformed acknowledgement cannot become a saved default");
        host.Pending = new(); saving = model.SetSubagentModelAsync("chat"); model.Dispose();
        host.Pending.SetResult(JsonValue.Create("chat")); await saving;
        Check(model.Values.Count == 0, "a late save cannot repopulate a closed editor");
        Console.WriteLine("Native default subagent model selection and acknowledged persistence passed.");
    }
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public readonly JsonArray Models = [new JsonObject { ["id"] = "chat", ["model"] = "chat", ["label"] = "Chat" }, new JsonObject { ["id"] = "image", ["model"] = "gpt-image-2" }];
        public int Writes;
        public bool Fail;
        public string? Project;
        public TaskCompletionSource<JsonNode?>? Pending;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "list_models") return Task.FromResult<JsonNode?>(Models.DeepClone());
            if (command == "get_subagent_model") return Task.FromResult<JsonNode?>(JsonValue.Create("missing"));
            if (command != "set_subagent_model") throw new InvalidOperationException(command);
            Writes++; Project = projectId;
            return Fail ? Task.FromException<JsonNode?>(new IOException("Save failed")) : Pending?.Task ?? Task.FromResult(arguments["id"]?.DeepClone());
        }
    }
}
