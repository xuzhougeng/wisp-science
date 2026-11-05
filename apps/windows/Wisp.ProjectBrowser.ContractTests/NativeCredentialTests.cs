using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeCredentialTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var client = new Fake(); using var model = new NativeSettingsEditorModel(client, "project");
        Check(!await model.SaveCredentialsAsync(["ncbi_api_key"]) && !await model.ClearCredentialAsync("ncbi_api_key") && !model.CanAddCredential, "unloaded credential writes are blocked");
        await model.LoadAsync("credential_status", "list_custom_credentials");
        Check(!model.CredentialPresent("ncbi_api_key") && model.CredentialPresent("custom:EXAMPLE"), "status includes built-in pairs and custom row presence");
        model.SetCredentialInput("ncbi_api_key", "  "); var count = client.Calls.Count;
        Check(!await model.SaveCredentialsAsync(["ncbi_api_key"]) && client.Calls.Count == count, "blank save does not clear a credential");
        model.SetCredentialInput("ncbi_api_key", "synthetic-key"); model.SetCredentialInput("ncbi_email", "sample@example.invalid");
        model.SetCredentialInput("custom:EXAMPLE", "synthetic-custom"); client.FailId = "ncbi_email";
        Check(!await model.SaveCredentialsAsync(["ncbi_api_key", "ncbi_email"]) && model.CredentialPresent("ncbi_api_key")
            && model.CredentialInput("ncbi_api_key") == "" && model.CredentialInput("ncbi_email") == "sample@example.invalid"
            && model.CredentialInput("custom:EXAMPLE") == "synthetic-custom", "partial write clears acknowledged secret only, retaining failed and other-page drafts");
        count = client.Calls.Count; client.FailId = null;
        Check(await model.SaveCredentialsAsync(["ncbi_api_key", "ncbi_email"]) && client.Calls.Count == count + 1
            && client.Calls[^1].Args["id"]!.ToString() == "ncbi_email" && model.CredentialFeedbackKey == "cred.saved", "explicit retry does not replay an acknowledged write");
        model.SetCredentialInput("ncbi_email", "new@example.invalid"); model.DiscardCredentialInputs(["ncbi_email"]);
        Check(model.CredentialInput("ncbi_email") == "" && model.CredentialInput("custom:EXAMPLE") != "", "service cancellation preserves unrelated custom draft");
        client.Pending = new(); var pending = model.ClearCredentialAsync("ncbi_api_key"); count = client.Calls.Count;
        Check(model.CredentialPresent("ncbi_api_key") && !await model.ClearCredentialAsync("ncbi_api_key") && client.Calls.Count == count, "pending clear retains status and blocks duplicate writes");
        client.Pending.SetException(new IOException("Synthetic failure")); await pending; client.Pending = null;
        Check(model.CredentialPresent("ncbi_api_key"), "failed clear preserves confirmed status");
        Check(await model.ClearCredentialAsync("ncbi_api_key") && !model.CredentialPresent("ncbi_api_key") && client.Calls[^1].Args["value"]!.ToString() == "", "explicit clear uses the existing empty-value contract");
        model.CustomCredentialName = "Example renamed"; model.CustomCredentialEnvironment = "EXAMPLE"; model.CustomCredentialValue = "synthetic-new";
        client.FailId = "add"; Check(!await model.AddCustomCredentialAsync() && model.CustomCredentialValue == "synthetic-new", "failed custom add keeps all input for retry");
        client.FailId = null; Check(await model.AddCustomCredentialAsync() && model.CustomCredentials.Length == 1 && model.CustomCredentials[0]["name"]!.ToString() == "Example renamed"
            && model.CustomCredentialValue == "" && client.Calls[^1].Args.ContainsKey("envVar") && !client.Calls[^1].Args.ContainsKey("env_var"), "custom upsert replaces status row and uses camel-case payload");
        client.FailId = "remove"; Check(!await model.RemoveCustomCredentialAsync("custom:EXAMPLE") && model.CustomCredentials.Length == 1, "failed remove keeps row");
        client.FailId = null; Check(await model.RemoveCustomCredentialAsync("custom:EXAMPLE") && model.CustomCredentials.Length == 0 && !model.CredentialPresent("custom:EXAMPLE"), "acknowledged remove clears row, status and input");
        count = client.Calls.Count; Check(!await model.RemoveCustomCredentialAsync("custom:EXAMPLE") && !await model.ClearCredentialAsync("unknown") && client.Calls.Count == count, "stale and unknown rows cannot write");
        Check(client.Calls.All(call => call.Project == "project"), "requests retain mounted project context");
        var late = new Fake(); var closed = new NativeSettingsEditorModel(late, null); await closed.LoadAsync("credential_status", "list_custom_credentials");
        closed.SetCredentialInput("ncbi_api_key", "synthetic"); closed.SetCredentialInput("ncbi_email", "not-sent@example.invalid"); late.Pending = new();
        var save = closed.SaveCredentialsAsync(["ncbi_api_key", "ncbi_email"]); count = late.Calls.Count; closed.Dispose(); late.Pending.SetResult(null); await save;
        Check(closed.CredentialInput("ncbi_api_key") == "" && !closed.HasCredentialChanges && late.Calls.Count == count && !closed.CredentialPresent("ncbi_api_key"), "closing erases drafts, ignores late acknowledgement and stops remaining writes");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public string? FailId; public TaskCompletionSource<JsonNode?>? Pending;
        public readonly List<(string Command, JsonObject Args, string? Project)> Calls = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)args.DeepClone(), projectId));
            if (command == "credential_status") return Task.FromResult<JsonNode?>(new JsonArray(new JsonArray("ncbi_api_key", false), new JsonArray("ncbi_email", false)));
            if (command == "list_custom_credentials") return Task.FromResult<JsonNode?>(new JsonArray(Custom("Example")));
            if (FailId != null && (args["id"]?.ToString() == FailId || command == "add_custom_credential" && FailId == "add" || command == "remove_custom_credential" && FailId == "remove"))
                return Task.FromException<JsonNode?>(new IOException("Synthetic failure"));
            if (Pending != null) return Pending.Task;
            return Task.FromResult<JsonNode?>(command == "add_custom_credential" ? Custom(args["name"]!.ToString()) : null);
        }
        private static JsonObject Custom(string name) => new() { ["id"] = "custom:EXAMPLE", ["name"] = name, ["env_var"] = "EXAMPLE", ["present"] = true };
    }
}
