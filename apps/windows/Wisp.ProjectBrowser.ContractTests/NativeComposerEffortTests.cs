using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeComposerEffortTests
{
    public static async Task RunAsync()
    {
        var fake = new Fake(); var model = new NativeComposerEffortModel(fake);
        await model.BindAsync("p", "s", fake.Profile);
        Check(model.Options.SequenceEqual(["low", "high"]) && model.Value == "low", "only exact catalog efforts are offered");
        Check(fake.Lookup?["model"]?.GetValue<string>() == "exact-model" && fake.Lookup?["apiUrl"]?.GetValue<string>() == "https://example.invalid", "catalog uses full provider endpoint and exact model");
        Check(!await model.SaveAsync("ultra") && fake.Writes == 0, "unsupported effort cannot write");
        fake.Profile["future_field"] = "new value";
        Check(await model.SaveAsync("high") && model.Value == "high", "confirmed model default is reflected");
        Check(fake.Saved?["key"] == null && fake.Saved?["profile"]?["key"] == null && fake.Saved?["profile"]?["future_field"]?.GetValue<string>() == "new value"
            && fake.Saved?["profile"]?["service_tier"]?.GetValue<string>() == "priority", "latest unrelated model fields survive and no key is written");
        fake.Fail = true; var writes = fake.Writes;
        Check(!await model.SaveAsync("low") && model.Value == "high" && fake.Writes == writes + 1 && model.Error != null, "uncertain response preserves confirmed display without replay");
        fake.Fail = false; await model.ReloadAsync(); Check(fake.Writes == writes + 1 && model.Error == null, "recovery only rereads state");
        Check(await model.SaveAsync("") && model.Value == "", "empty choice restores provider default");
        fake.Profile["model"] = "changed-model"; writes = fake.Writes;
        Check(!await model.SaveAsync("low") && fake.Writes == writes, "changed model identity requires refreshed capabilities");
        var pending = fake.Pending = new();
        var bind = model.BindAsync("p", "other", fake.Profile);
        model.Reset(); pending.SetResult(new JsonObject { ["efforts"] = new JsonArray("ultra") }); await bind;
        Check(model.Options.Length == 0, "late lookup cannot populate closed or different binding");
        fake.Pending = null; fake.Catalog = null;
        await model.BindAsync("p", "s", fake.Profile);
        Check(model.Options.Length == 0 && !await model.SaveAsync("low"), "unknown catalog does not infer capabilities");
        fake.Profile["id"] = "removed-model"; await model.ReloadAsync();
        Check(model.Options.Length == 0 && model.Value == "", "removed profile clears the picker during recovery");
        Console.WriteLine("Native composer model effort catalog, preservation, no-replay and late response tests passed.");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public JsonObject Profile = new() { ["id"] = "m", ["provider"] = "openai", ["api_url"] = "https://example.invalid", ["model"] = "exact-model", ["reasoning_effort"] = "low", ["service_tier"] = "priority", ["key"] = "must not be sent" };
        public JsonObject? Catalog = new() { ["efforts"] = new JsonArray("low", "high") };
        public JsonObject? Lookup, Saved;
        public TaskCompletionSource<JsonNode?>? Pending;
        public int Writes;
        public bool Fail;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken token = default)
        {
            if (command == "model_catalog_lookup") { Lookup = args; return Pending?.Task ?? Task.FromResult<JsonNode?>(Catalog?.DeepClone()); }
            if (command == "list_models") return Task.FromResult<JsonNode?>(new JsonArray(Profile.DeepClone()));
            if (command == "save_model")
            {
                Writes++; Saved = (JsonObject)args.DeepClone();
                if (Fail) return Task.FromException<JsonNode?>(new IOException("response lost"));
                Profile = (JsonObject)args["profile"]!.DeepClone();
                return Task.FromResult<JsonNode?>(new JsonArray(Profile.DeepClone()));
            }
            throw new InvalidOperationException(command);
        }
    }
}
