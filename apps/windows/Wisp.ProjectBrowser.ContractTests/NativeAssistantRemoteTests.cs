using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeAssistantRemoteTests
{
    private static void Check(bool value, string reason) { if (!value) throw new InvalidOperationException(reason); }
    public static async Task RunAsync()
    {
        var fake = new Fake(); using var model = new NativeAssistantRemoteModel(fake);
        await model.StartBindingAsync(); Check(fake.Calls.Count == 0, "binding waits for authoritative status");
        await model.RefreshAsync(); Check(model.CanBind && !model.CanChange, "unbound channel can bind but cannot toggle");
        await model.SetEnabledAsync(true); Check(fake.Calls.Count == 1, "unbound enable does not dispatch");
        await model.StartBindingAsync(); Check(model.QrImage != null && model.Busy, "explicit bind opens a QR flow");
        var count = fake.Calls.Count;
        await model.StartBindingAsync(); await model.UnbindAsync(); await model.RefreshAsync();
        Check(fake.Calls.Count == count, "in-flight QR excludes competing commands");
        await model.PollAsync(); Check(model.QrImage != null, "pending scan retains the QR");
        fake.Poll = "confirmed"; await model.PollAsync();
        Check(model.CanChange && model.QrImage == null && model.Status?.Bound == true, "confirmed binding reads authoritative status");
        await model.SetEnabledAsync(true); Check(model.Status?.Enabled == true, "toggle refreshes independent assistant status");
        await model.UnbindAsync(); Check(model.Status?.Bound == false, "unbind refreshes the assistant binding only");
        Check(fake.Calls.All(c => c.Project == null), "all assistant channel requests use global scope");
        Check(fake.Calls.Where(c => c.Command is "weixin_bind_poll" or "set_weixin_channel" or "weixin_unbind")
            .All(c => c.Args["destination"]?.GetValue<string>() == "assistant"), "every binding mutation names the assistant destination");

        fake.Poll = "expired"; await model.StartBindingAsync(); await model.PollAsync();
        Check(model.Expired && !model.Busy && model.QrImage == null, "expiry hides the QR and permits an explicit new bind");
        fake.Fail = "weixin_bind_poll"; await model.StartBindingAsync(); await model.PollAsync(); count = fake.Calls.Count;
        await model.PollAsync(); Check(fake.Calls.Count == count && model.Error != null && !model.Busy, "an ambiguous poll is not replayed");
        fake.Fail = "assistant_weixin_status"; await model.RefreshAsync();
        Check(model.Status == null && !model.CanBind && model.Error != null, "failed read cannot leave stale actionable status");
        fake.Fail = null; await model.RefreshAsync(); fake.InvalidQr = true; await model.StartBindingAsync();
        Check(model.QrImage == null && model.Error != null, "invalid QR cannot begin polling");

        var closingFake = new Fake(); using var closing = new NativeAssistantRemoteModel(closingFake);
        await closing.RefreshAsync(); closingFake.Pending = new(); var pending = closing.StartBindingAsync();
        closing.Dispose(); closingFake.Pending.SetResult(Fake.Binding()); await pending; await closing.PollAsync();
        Check(closing.Closed && closing.QrImage == null && closing.Status == null && closingFake.Calls.Count == 2, "late start cannot revive a closed dialog or poll");
        var finalFake = new Fake(); using var final = new NativeAssistantRemoteModel(finalFake);
        await final.RefreshAsync(); await final.StartBindingAsync(); finalFake.Pending = new(); var poll = final.PollAsync();
        final.Dispose(); finalFake.Pending.SetResult(JsonValue.Create("confirmed")); await poll;
        Check(final.Status == null && finalFake.Calls.Count == 3, "late confirmation cannot refresh a dismissed dialog");
        var boundedFake = new Fake(); using var bounded = new NativeAssistantRemoteModel(boundedFake);
        await bounded.RefreshAsync(); await bounded.StartBindingAsync();
        for (var i = 0; i < 180; i++) await bounded.PollAsync();
        Check(bounded.Expired && !bounded.Busy, "polling is bounded even without an expiry response");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public readonly List<(string Command, JsonObject Args, string? Project)> Calls = [];
        public string Poll = "wait";
        public string? Fail;
        public bool InvalidQr, Bound, Enabled;
        public TaskCompletionSource<JsonNode?>? Pending;
        public static JsonObject Binding() => new() { ["qrcode"] = "in-memory-only", ["qr_image"] = "data:image/svg+xml;base64,PHN2Zy8+" };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)args.DeepClone(), projectId));
            if (Fail == command) throw new IOException("Simulated transport failure");
            if (Pending != null) return Pending.Task;
            JsonNode? result = null;
            switch (command)
            {
                case "assistant_weixin_status": result = new JsonObject { ["enabled"] = Enabled, ["bound"] = Bound, ["state"] = Enabled ? "running" : "stopped", ["detail"] = "" }; break;
                case "weixin_bind_start": result = InvalidQr ? new JsonObject { ["qrcode"] = "bad", ["qr_image"] = "https://invalid.example/qr" } : Binding(); break;
                case "weixin_bind_poll": if (Poll == "confirmed") Bound = true; result = JsonValue.Create(Poll); break;
                case "set_weixin_channel": Enabled = args["enabled"]!.GetValue<bool>(); break;
                case "weixin_unbind": Enabled = Bound = false; break;
                default: throw new InvalidOperationException("Unexpected command " + command);
            }
            return Task.FromResult(result);
        }
    }
}
