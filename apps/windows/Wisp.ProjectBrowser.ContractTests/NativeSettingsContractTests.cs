using System.Net;
using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSettingsContractTests
{
    public static async Task Run(string fixtures)
    {
        var request = JsonSerializer.Deserialize<NativeSettingsRequest>(File.ReadAllText(Path.Combine(fixtures, "request.json")))!;
        Require(request.Schema == NativeSettingsProtocol.Schema && request.ProjectId == "research-1"
            && request.Arguments["prefs"]?["ui_font_size"]?.GetValue<int>() == 15, "Settings request drift");
        Require(NativeSettingsProtocol.Sections.Count == 20, "Settings sections drift");
        Require(NativeSettingsProtocol.Sections[1] == "network", "Network section order drift");
        var response = NativeSettingsProtocol.DecodeResponse(File.ReadAllText(Path.Combine(fixtures, "response.json")), "settings-1");
        Require(response?["theme"]?.GetValue<string>() == "dark", "Settings response drift");
        Require(NativeSettingsProtocol.DecodeResponse(File.ReadAllText(Path.Combine(fixtures, "void.json")), "settings-2") is null, "Void save rejected");
        Throws<InvalidDataException>(() => NativeSettingsProtocol.DecodeResponse(File.ReadAllText(Path.Combine(fixtures, "response.json")), "stale"));
        Throws<InvalidOperationException>(() => NativeSettingsProtocol.DecodeResponse(File.ReadAllText(Path.Combine(fixtures, "error.json")), "settings-3"));
        var database = Path.GetFullPath(Path.Combine(Path.GetTempPath(), "native-settings-test.sqlite"));
        var host = new NativeSettingsHost(NativeSettingsProtocol.Schema, "http://127.0.0.1:12345/invoke", new string('a', 64), database, 1);
        NativeSettingsProtocol.ValidateHost(host, database);
        foreach (var endpoint in new[] { "https://example.com/invoke", "http://localhost:1234/invoke", "http://127.0.0.1:1234/other", "http://127.0.0.1:1234/invoke?x=1", "http://user@127.0.0.1:1234/invoke" })
            Throws<InvalidDataException>(() => NativeSettingsProtocol.ValidateHost(host with { Endpoint = endpoint }, database));
        Throws<InvalidDataException>(() => NativeSettingsProtocol.ValidateHost(host, database + ".other"));
        Throws<InvalidDataException>(() => NativeSettingsProtocol.ValidateHost(host with { Token = "short" }, database));
        var handler = new FakeSettingsHandler();
        using var client = new NativeSettingsClient(host, database, handler);
        var args = new JsonObject { ["settings"] = new JsonObject { ["locale"] = "en", ["future_option"] = 42 } };
        await client.InvokeAsync("set_settings", args, "project-a");
        Require(handler.Calls == 1 && handler.Request?["project_id"]?.GetValue<string>() == "project-a"
            && handler.Request?["args"]?["settings"]?["future_option"]?.GetValue<int>() == 42, "Transport lost scope or unknown fields");
        handler.Fail = true;
        try { await client.InvokeAsync("set_settings", args); throw new Exception("Expected rejection"); }
        catch (HttpRequestException) { Require(handler.Calls == 2, "Mutation was retried"); }
        var catalog = JsonNode.Parse(File.ReadAllText(Path.Combine(fixtures, "commands.json")))!["commands"]!.AsArray();
        Require(catalog.Any(c => c?["command"]?.GetValue<string>() == "import_wsl_contexts"), "Missing Windows environment seam");
        Require(!catalog.Any(c => c?["command"]?.GetValue<string>() == "send_message"), "Non-settings capability leaked");
        Console.WriteLine("Native settings shared fixtures, scoped transport, error handling and no-retry tests passed.");
    }
    static void Require(bool ok, string message) { if (!ok) throw new InvalidOperationException(message); }
    static void Throws<T>(Action action) where T : Exception {
        try { action(); } catch (T) { return; }
        throw new InvalidOperationException("Expected " + typeof(T).Name);
    }
    sealed class FakeSettingsHandler : HttpMessageHandler {
        public int Calls; public bool Fail; public JsonObject? Request;
        protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken) {
            Calls++;
            Require(request.Headers.Authorization?.Scheme == "Bearer" && request.RequestUri?.Host == "127.0.0.1", "Missing authenticated loopback transport");
            Request = JsonNode.Parse(await request.Content!.ReadAsStringAsync(cancellationToken))!.AsObject();
            return new HttpResponseMessage(Fail ? HttpStatusCode.ServiceUnavailable : HttpStatusCode.OK) {
                Content = new StringContent(new JsonObject { ["schema"] = NativeSettingsProtocol.Schema, ["id"] = Request["id"]!.DeepClone(), ["result"] = null, ["error"] = null }.ToJsonString())
            };
        }
    }
}
