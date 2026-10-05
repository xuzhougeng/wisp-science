using System.Net;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeHostReconnectTests
{
    public static async Task RunAsync()
    {
        var root = Path.Combine(Path.GetTempPath(), "wisp-native-reconnect-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root);
        try
        {
            var database = Path.Combine(root, "wisp.sqlite");
            var descriptor = Path.Combine(root, "native-settings.json");
            var first = new NativeSettingsHost(NativeSettingsProtocol.Schema, "http://127.0.0.1:41111/invoke", new string('a', 64), database, 1);
            var second = first with { Endpoint = "http://127.0.0.1:42222/invoke", Token = new string('b', 64), Pid = 2 };
            await File.WriteAllTextAsync(descriptor, JsonSerializer.Serialize(first));
            var handler = new Handler();
            using var client = new NativeSettingsClient(first, database, handler, descriptor);
            handler.Fail = true;
            try { await client.InvokeAsync("native_conversation_acp_setting", new(), "p"); throw new Exception("Expected lost reply"); }
            catch (HttpRequestException) { }
            Check(handler.Calls.Count == 1, "failed mutation has exactly one attempt");
            await File.WriteAllTextAsync(descriptor, JsonSerializer.Serialize(second));
            handler.Fail = false;
            await client.InvokeAsync("native_conversation_snapshot", new(), "p");
            Check(handler.Calls.Count == 2 && handler.Calls[1] == (second.Endpoint, second.Token, "native_conversation_snapshot"),
                "next read pairs the new port and token without replaying the mutation");
            await File.WriteAllTextAsync(descriptor, JsonSerializer.Serialize(second with { Database = Path.Combine(root, "foreign.sqlite") }));
            try { await client.InvokeAsync("native_conversation_snapshot", new(), "p"); throw new Exception("Expected owner rejection"); }
            catch (InvalidDataException) { }
            Check(handler.Calls.Count == 2, "changed database is rejected before sending a bearer token");
            await File.WriteAllTextAsync(descriptor, JsonSerializer.Serialize(second with { Endpoint = "http://example.com/invoke" }));
            try { await client.InvokeAsync("native_conversation_snapshot", new(), "p"); throw new Exception("Expected endpoint rejection"); }
            catch (InvalidDataException) { }
            Check(handler.Calls.Count == 2, "non-loopback replacement never receives a request");
        }
        finally { Directory.Delete(root, true); }
    }
    private static void Check(bool value, string message)
    { if (!value) throw new InvalidOperationException(message); Console.WriteLine("PASS native host reconnect: " + message); }
    private sealed class Handler : HttpMessageHandler
    {
        public bool Fail;
        public List<(string Endpoint, string Token, string Command)> Calls = [];
        protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken token)
        {
            var body = JsonNode.Parse(await request.Content!.ReadAsStringAsync(token))!;
            Calls.Add((request.RequestUri!.AbsoluteUri, request.Headers.Authorization!.Parameter!, body["command"]!.GetValue<string>()));
            if (Fail) throw new HttpRequestException("Reply lost after commit");
            return new(HttpStatusCode.OK) { Content = new StringContent(new JsonObject {
                ["schema"] = NativeSettingsProtocol.Schema, ["id"] = body["id"]!.DeepClone(), ["result"] = new JsonObject()
            }.ToJsonString(), Encoding.UTF8, "application/json") };
        }
    }
}
