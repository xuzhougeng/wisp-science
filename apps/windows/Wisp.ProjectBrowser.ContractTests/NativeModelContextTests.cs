using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeModelContextTests
{
    private static void Check(bool pass, string message) { if (!pass) throw new InvalidOperationException(message); }
    private static ConversationItem Item(string role, string text) => new(role, text, null, null, null, null);
    private static ConversationSnapshot Snapshot(string session = "s", string text = "kept") => new(ConversationSnapshot.SchemaId, "host", 1, "p", session,
        [Item("user", text), Item("assistant", "answer"), Item("usage", "{}")], null, false, false, false, "m", null, null, [], UserOffset: 4);
    private static JsonNode Response(string session = "s") => JsonNode.Parse("""
        {"project_id":"p","session_id":"s","items":[{"role":"system","text":"Exact system\nSecond line"},
         {"role":"user","kind":"checkpoint","text":"## Checkpoint\nPrior evidence"},{"role":"user","text":"kept"},{"role":"assistant","text":"answer"}],
         "state":{"head_epoch":2,"in_context_from_user_index":4,"context_epochs":[{"epoch":2,"parent_epoch":1,"strategy":"manual","kind":"semantic","before_tokens":9000,"after_tokens":2000,"initial_head_seq":12,"checkpoint_seq":13}],
         "compactions":[],"undone_epochs":[]}}
        """)!.AsObject().WithSession(session);
    private static JsonObject WithSession(this JsonObject value, string session) { value["session_id"] = session; return value; }
    public static async Task RunAsync()
    {
        var fake = new Fake(); var model = new NativeModelContextModel(fake);
        model.Bind(Snapshot()); Check(fake.Reads == 0 && model.Available && !model.Active, "Hidden model view never reads eagerly.");
        await model.SelectAsync(true);
        Check(model.Active && model.View?.Items[1].Role == "checkpoint" && model.View.State.HeadEpoch == 2, "Legacy checkpoint kind normalizes without consuming a user turn.");
        Check(model.DisplaySnapshot?.HistoryState == null && model.DisplaySnapshot?.NextBeforeSeq == null, "Model rows cannot inherit historical mutation or paging identities.");
        model.Bind(Snapshot() with { Sequence = 2 }); Check(fake.Reads == 1, "Snapshot sequence alone cannot re-read model context.");
        model.Bind(Snapshot() with { Running = true }); var afterStart = fake.Reads;
        var future = Snapshot() with { Running = true, Items = [Item("user", "kept"), Item("assistant", "answer"), Item("user", "next"), Item("assistant", "streaming"), Item("usage", "{}"), Item("queued_user", "queued")] };
        model.Bind(future);
        Check(fake.Reads == afterStart && model.DisplaySnapshot!.Items.Last().Text == "streaming"
            && model.DisplaySnapshot.Items.Count(row => row.Text == "next") == 1, "New running turns append once, excluding usage and queued UI rows.");
        model.Bind(future with { Items = future.Items.Select(row => row.Text == "streaming" ? row with { Text = "streaming more" } : row).ToArray() });
        Check(model.DisplaySnapshot!.Items.Last().Text == "streaming more", "Stream revisions replace the live suffix instead of duplicating it.");
        Check(afterStart == 1, "Starting a turn retains the completed head and appends the new live turn without a redundant read.");
        await model.SelectAsync(false); Check(!model.Active && model.View == null, "Full transcript drops only the model projection.");
        model.Bind(Snapshot() with { Running = true }); await model.SelectAsync(true);
        model.Bind(Snapshot() with { Running = true, Items = [Item("user", "kept"), Item("assistant", "updated while reading"), Item("usage", "{}")] });
        Check(model.DisplaySnapshot!.Items.Last().Text == "updated while reading" && model.DisplaySnapshot.Items.Count(row => row.Role == "user") == 1
            && model.DisplaySnapshot.Items.Any(row => row.Role == "checkpoint"), "Opening during a live turn replaces its partial persisted suffix, preserving system/checkpoint rows.");
        var beforeSettled = fake.Reads;
        model.Bind(Snapshot() with { Running = false, Items = [Item("user", "kept"), Item("assistant", "updated while reading"), Item("usage", "{}")] });
        Check(fake.Reads == beforeSettled + 1, "A completed turn reloads the persisted head even when its final content matches the last stream poll.");
        await model.SelectAsync(false);
        fake.Pending = new(); var load = model.SelectAsync(true); model.Bind(Snapshot("other"));
        fake.Pending.SetResult(Response()); await load; fake.Pending = null;
        Check(!model.Active && model.View == null && !model.Busy, "Session changes reject late context replies.");
        model.Bind(Snapshot()); fake.Fail = true; await model.SelectAsync(true); var failedReads = fake.Reads;
        model.Bind(Snapshot(text: "changed")); Check(model.Error != null && fake.Reads == failedReads, "A failed read needs explicit retry even when polling advances.");
        fake.Fail = false; await model.RetryAsync(); Check(model.View != null && model.Error == null, "Explicit retry recovers.");
        await model.SelectAsync(false); fake.Pending = new(); load = model.SelectAsync(true); await model.SelectAsync(false);
        fake.Pending.SetResult(Response()); await load; fake.Pending = null; Check(model.View == null, "Closing while loading rejects its result.");
        model.Bind(Snapshot() with { AcpAgentId = "agent" }); await model.SelectAsync(true); Check(!model.Active && !model.Available, "ACP cannot request built-in context.");
        model.Bind(Snapshot()); var versions = 0;
        fake.OnRead = () => { model.Bind(Snapshot(text: "revision " + ++versions)); return Response(); };
        await model.SelectAsync(true);
        Check(versions == 3 && model.ChangedDuringRead && model.View == null && !model.Busy, "Three conflicting reads stop with a visible retry state.");
        fake.OnRead = null; await model.RetryAsync(); Check(model.View != null, "Stable retry after contention succeeds.");
        var malformed = Response(); malformed["state"]!["context_epochs"]!.AsArray().Add(malformed["state"]!["context_epochs"]![0]!.DeepClone());
        var rejected = false; try { NativeModelContext.Decode(malformed, "p", "s"); } catch (InvalidDataException) { rejected = true; }
        Check(rejected, "Duplicate epoch identities are rejected.");
        rejected = false; try { NativeModelContext.Decode(Response("wrong"), "p", "s"); } catch (InvalidDataException) { rejected = true; }
        Check(rejected, "Response scope is independently validated.");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public int Reads; public bool Fail; public TaskCompletionSource<JsonNode?>? Pending; public Func<JsonNode?>? OnRead;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Check(command == "native_conversation_context" && args.Count == 1 && args["session_id"] != null && projectId == "p", "Only the scoped read-only route is allowed.");
            Reads++; if (Fail) throw new IOException("Synthetic model context failure");
            return Pending?.Task ?? Task.FromResult<JsonNode?>(OnRead?.Invoke() ?? Response());
        }
    }
}
