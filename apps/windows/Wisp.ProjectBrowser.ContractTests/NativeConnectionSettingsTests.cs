using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeConnectionSettingsTests
{
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var draft = NativeConnections.Create(); var other = NativeConnections.Create();
        Check(draft["id"]!.GetValue<string>() != other["id"]!.GetValue<string>() && draft["enabled"]!.GetValue<bool>() && draft["transport"]!["kind"]!.GetValue<string>() == "stdio", "new connection has its own identity and shared stdio defaults");
        draft["name"] = " Example "; draft["future"] = 9; var transport = draft["transport"]!.AsObject(); transport["command"] = " command "; transport["cwd"] = "C:/kept"; transport["args"] = new JsonArray("path with spaces", "--flag");
        transport["env"] = new JsonArray(new JsonObject { ["name"] = " TOKEN ", ["value"] = "", ["has_value"] = true, ["secret_ref"] = "existing" }, new JsonObject { ["name"] = " NEW ", ["value"] = " replacement " }, new JsonObject { ["name"] = " ", ["value"] = "ignored" });
        var save = NativeConnections.SaveDraft(draft);
        Check(save["name"]!.GetValue<string>() == "Example" && save["future"]!.GetValue<int>() == 9 && save["transport"]!["cwd"]!.GetValue<string>() == "C:/kept" && save["transport"]!["args"]![0]!.GetValue<string>() == "path with spaces", "normalization preserves unknown fields, existing cwd and untouched spaced arguments");
        var secrets = save["transport"]!["env"]!.AsArray();
        Check(secrets.Count == 2 && !secrets[0]!.AsObject().ContainsKey("value") && secrets[0]!["secret_ref"]!.GetValue<string>() == "existing" && secrets[1]!["value"]!.GetValue<string>() == "replacement", "blank names are omitted and blank replacements retain host keyring references");
        Check(draft["transport"]!["env"]!.AsArray().Count == 3 && draft["name"]!.GetValue<string>() == " Example ", "payload construction never mutates the draft");
        var hintsLocale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>
        {
            ["en"] = new() { ["conn.secret_keep"] = "Leave blank to keep the stored value", ["conn.secret_value"] = "Value" },
            ["zh"] = new() { ["conn.secret_keep"] = "留空则保留已保存的值", ["conn.secret_value"] = "值" }
        });
        var emptySecret = new JsonObject();
        Check(NativeConnections.SecretPlaceholders(NativeConnections.Transport("stdio"), emptySecret, hintsLocale) == ("NAME", "Value"), "environment fields use localized value hints");
        var httpHints = NativeConnections.Transport("http");
        Check(NativeConnections.SecretPlaceholders(httpHints, emptySecret, hintsLocale) == ("Authorization", "Bearer token"), "manual HTTP headers identify the authorization token");
        httpHints["auth"] = "oauth";
        Check(NativeConnections.SecretPlaceholders(httpHints, emptySecret, hintsLocale) == ("X-Custom-Header", "value"), "OAuth headers do not prompt for a bearer token");
        hintsLocale.Preview("zh");
        Check(NativeConnections.SecretPlaceholders(NativeConnections.Transport("stdio"), emptySecret, hintsLocale) == ("NAME", "值"), "environment hints follow the current locale");
        foreach (var stored in new[] { new JsonObject { ["has_value"] = true }, new JsonObject { ["secret_ref"] = "opaque-reference" } })
            Check(NativeConnections.SecretPlaceholders(httpHints, stored, hintsLocale) == ("X-Custom-Header", "留空则保留已保存的值"), "stored-value hints take priority without retrieving or replacing a secret");
        Check(emptySecret.Count == 0, "placeholder calculation does not populate the draft or expose stored values");
        var schema = JsonNode.Parse("{\"type\":\"object\",\"required\":[\"query\"],\"properties\":{\"query\":{\"type\":[\"string\",\"null\"],\"description\":\"Question\",\"default\":null},\"limit\":{\"type\":\"integer\",\"default\":0},\"anything\":true}}")!;
        var parameters = NativeConnections.Parameters(schema);
        Check(parameters[0].Required && parameters[0].Type == "string | null" && parameters[0].Default == "null" && !parameters[1].Required && parameters[1].Default == "0" && parameters[2].Type == "", "documentation preserves union types, required flags, null/zero defaults and boolean schemas");
        Check(NativeConnections.Parameters(JsonValue.Create(false)).Length == 0, "boolean input schemas do not break the detail page");
        var descriptiveSchema = JsonNode.Parse("""{"required":["dataset_attribute_names"],"properties":{"dataset_attribute_names":{"type":["array","null"],"description":"First line\nSecond line","default":[]},"empty_value":{"type":"string","default":""},"no_default":{"type":"string"}}}""")!;
        var descriptiveParameters = NativeConnections.Parameters(descriptiveSchema);
        Check(descriptiveParameters[0].Name == "dataset_attribute_names" && descriptiveParameters[0].Required
            && descriptiveParameters[0].Description == "First line\nSecond line" && descriptiveParameters[0].Default == "[]"
            && descriptiveParameters[1].Default == "\"\"" && descriptiveParameters[2].Default == null,
            "separate documentation cells retain multiline descriptions and distinguish empty defaults from absent defaults");
        Check(NativeConnections.Bundled(JsonNode.Parse("{\"connectors\":[{\"kind\":\"bundled\"},{\"kind\":\"custom\"}]}" )).Length == 1, "custom connections are not duplicated in the bundled list");
        var bilingual = new JsonObject { ["description"] = "Query genomic annotations", ["description_zh"] = "查询基因组注释" };
        Check(NativeConnections.Description(bilingual, "en") == "Query genomic annotations"
            && NativeConnections.Description(bilingual, "zh-CN") == "查询基因组注释",
            "connector introductions follow the selected locale instead of always preferring Chinese");
        bilingual["description_zh"] = "";
        Check(NativeConnections.Description(bilingual, "zh") == "Query genomic annotations"
            && NativeConnections.Description(new JsonObject(), "en") == "",
            "missing connector translations fall back to the catalog description without fabricating copy");
        var fake = new Fake(); using var model = new NativeSettingsEditorModel(fake, "project");
        await model.LoadAsync("list_mcp_connections"); var source = model.Values["list_mcp_connections"]!["connections"]![0]!.AsObject(); JsonArray? tools = null;
        Check(!await model.DiscoverConnectionToolsAsync((JsonObject)source.DeepClone(), value => tools = value), "detached custom rows cannot start discovery");
        Check(await model.DiscoverConnectionToolsAsync(source, value => tools = value) && tools?.Count == 1 && fake.LastProject == "project", "owned custom discovery returns descriptions and schemas under explicit project context");
        draft["transport"] = NativeConnections.Transport("http"); draft["transport"]!["auth"] = "oauth"; model.Edit(draft, "add_mcp_connection", "conn");
        fake.Pending = new(); var pending = model.TestConnectionAsync(value => tools = value); var calls = fake.Commands.Count;
        Check(model.Busy && model.OAuthPending && !await model.SaveAsync() && fake.Commands.Count == calls && fake.Commands[^1] == "test_oauth_mcp_connection", "OAuth tests block duplicate writes and use the test-only host command");
        await model.CancelOAuthAsync(); Check(fake.Commands[^1] == "cancel_oauth_authorization", "pending test can cancel OAuth without resubmitting");
        fake.Pending.SetException(new IOException("cancelled")); Check(!await pending && model.Draft != null && !model.OAuthPending && !model.SaveConfirmed, "failed test preserves the draft without claiming it was saved");
        fake.Pending = null; model.Draft!["transport"]!["auth"] = "none";
        Check(await model.TestConnectionAsync(value => tools = value) && fake.Commands[^1] == "test_mcp_connection" && model.Draft != null && !model.SaveConfirmed, "plain connection test leaves editor and save state intact");
        fake.Pending = new(); var callback = false; pending = model.TestConnectionAsync(_ => callback = true); model.Dispose(); fake.Pending.SetResult(new JsonArray()); await pending;
        Check(!callback, "closed editor ignores late discovery results");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public List<string> Commands { get; } = [];
        public TaskCompletionSource<JsonNode?>? Pending;
        public string? LastProject;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Commands.Add(command); LastProject = projectId;
            if (command == "list_mcp_connections") return Task.FromResult<JsonNode?>(new JsonObject { ["connections"] = new JsonArray(NativeConnections.Create()) });
            if (command == "cancel_oauth_authorization") return Task.FromResult<JsonNode?>(null);
            if (command is "test_mcp_connection" or "test_oauth_mcp_connection") return Pending?.Task ?? Task.FromResult<JsonNode?>(new JsonArray(new JsonObject { ["name"] = "lookup", ["description"] = "Search", ["inputSchema"] = new JsonObject { ["type"] = "object" } }));
            throw new InvalidOperationException(command);
        }
    }
}
