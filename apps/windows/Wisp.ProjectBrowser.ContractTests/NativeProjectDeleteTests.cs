using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeProjectDeleteTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync(string fixtureDirectory)
    {
        var fixture = JsonNode.Parse(File.ReadAllText(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-projects/v1/delete.json"))))!;
        var host = new Fake(); var clock = new Clock();
        using var cancelled = new NativeProjectDeleteModel(host, "research-1", clock);
        cancelled.ConfirmData(); cancelled.Dispose(); clock.Advance(5000);
        Check(!await cancelled.DeleteAsync() && host.Calls == 0, "closed confirmation cannot delete after the timer expires");
        using var choice = new NativeProjectDeleteModel(host, "research-1", clock);
        choice.ConfirmData();
        Check(choice.ConfirmingData && choice.RemainingSeconds == 5 && !await choice.DeleteAsync() && host.Calls == 0, "data deletion cannot bypass the countdown");
        clock.Advance(4999);
        Check(choice.RemainingSeconds == 1 && !await choice.DeleteAsync(), "five full seconds are required");
        choice.Back(); choice.ConfirmData();
        Check(choice.RemainingSeconds == 5, "Back and reopen restarts the countdown");
        clock.Advance(5000);
        Check(await choice.DeleteAsync() && choice.Deleted && host.Scope == "research-1" && host.Data, "explicit project and destructive choice acknowledged");
        Check(!await choice.DeleteAsync() && host.Calls == 1, "success never replays");
        using var keep = new NativeProjectDeleteModel(host, "research-1", clock);
        keep.ConfirmData(); keep.Back();
        Check(await keep.DeleteAsync() && !host.Data && JsonNode.DeepEquals(host.Last, fixture["result"]), "Back preserves files and matches the shared Rust fixture");
        host.Fail = true;
        using var failed = new NativeProjectDeleteModel(host, "research-1", clock);
        var count = host.Calls;
        Check(!await failed.DeleteAsync() && failed.Error != null && failed.Attempted && !failed.Deleted, "lost response never claims deletion");
        failed.Back(); failed.ConfirmData();
        Check(!await failed.DeleteAsync() && host.Calls == count + 1, "failure cannot be replayed by Back or changing choice");
        host.Fail = false; host.WrongScope = true;
        using var wrong = new NativeProjectDeleteModel(host, "research-1", clock);
        Check(!await wrong.DeleteAsync() && !wrong.Deleted, "mismatched receipt cannot claim success");
        host.WrongScope = false; host.WrongChoice = true;
        using var wrongChoice = new NativeProjectDeleteModel(host, "research-1", clock);
        Check(!await wrongChoice.DeleteAsync() && !wrongChoice.Deleted, "wrong deletion mode cannot claim success");
        host.WrongChoice = false; host.MissingDeletion = true;
        using var missing = new NativeProjectDeleteModel(host, "research-1", clock);
        Check(!await missing.DeleteAsync() && !missing.Deleted, "missing deletion acknowledgement is rejected");
        host.MissingDeletion = false; host.Pending = new();
        using var closing = new NativeProjectDeleteModel(host, "research-1", clock);
        var pending = closing.DeleteAsync(); count = host.Calls;
        Check(!await closing.DeleteAsync() && host.Calls == count, "double submission is blocked");
        closing.Dispose(); host.Pending.SetResult(new JsonObject { ["project_id"] = "research-1", ["delete_data"] = false, ["deleted"] = true });
        Check(!await pending && !closing.Deleted, "closed surface ignores late replies");
    }
    private sealed class Clock : TimeProvider
    {
        private long now;
        public override long TimestampFrequency => 1000;
        public override long GetTimestamp() => now;
        public void Advance(long milliseconds) => now += milliseconds;
    }
    private sealed class Fake : INativeSettingsClient
    {
        public int Calls; public string? Scope; public bool Data, Fail, WrongScope, WrongChoice, MissingDeletion; public JsonObject? Last;
        public TaskCompletionSource<JsonNode?>? Pending;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Check(command == "native_project_delete", "expected native delete command");
            Calls++; Scope = projectId; Data = args["delete_data"]!.GetValue<bool>();
            if (Fail) throw new IOException("Lost reply");
            if (Pending != null) return Pending.Task;
            Last = new JsonObject { ["project_id"] = WrongScope ? "other" : projectId, ["delete_data"] = WrongChoice ? !Data : Data, ["deleted"] = !MissingDeletion };
            return Task.FromResult<JsonNode?>(Last);
        }
    }
}
