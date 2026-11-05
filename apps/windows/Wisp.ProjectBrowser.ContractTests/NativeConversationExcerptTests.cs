using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeConversationExcerptTests
{
    private static void Check(bool value, string message) { if (!value) throw new Exception(message); }
    private static readonly NativeLocalization Locale = new(new Dictionary<string, Dictionary<string, string>> {
        ["en"] = new() { ["excerpt.source"] = "{title} · turns {start}–{end}", ["excerpt.source_single"] = "{title} · turn {n}", ["excerpt.untitled"] = "Untitled session" }
    });
    private static NativeHistoryTarget Target(WorkspaceConversationModel model, int row = 0) => model.HistoryTarget(model.Snapshot!, row)!;
    public static async Task RunAsync()
    {
        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "source"); model.Draft = "keep source draft";
        var first = Target(model); var last = Target(model, 2);
        Check(await model.SelectExcerptAsync(last, Locale) == null && model.ExcerptStart == last && host.Reads == 0 && host.Creates == 0,
            "first click only selects, using an absolute paged user turn");
        Check(model.MessageActions(model.Snapshot!, 0).Single(a => a.Kind == "excerpt").LabelKey == "msg.excerpt_end", "all eligible user rows offer the second click");
        var id = await model.SelectExcerptAsync(first, Locale);
        Check(id == "quoted-1" && host.Start == 2 && host.End == 3 && host.Creates == 1, "reverse selection is normalized before reading and creating once");
        Check(model.Draft == "keep source draft" && model.Quotes.Length == 0 && model.ExcerptStart == null, "source draft and quotes are untouched");
        await model.OpenAsync("p", id!);
        Check(model.Draft == "" && model.References.Length == 0 && model.Attachments.Length == 0 && model.Quotes.Single().Source == "Source · turns 3–4",
            "destination is a blank composer with a named inclusive excerpt");
        Check(model.ExcerptTruncated && NativeComposerQuote.Compose("my question", model.Quotes).StartsWith("Quoted turns from an earlier conversation `Source · turns 3–4`:\n> User:"),
            "truncation is surfaced and read-only quoted context has WebView's message preamble");
        await model.OpenAsync("p", "source"); Check(model.Draft == "keep source draft", "source draft survives new-session navigation");
        await model.SelectExcerptAsync(first, Locale); Check(model.CancelExcerpt() && model.ExcerptStart == null && host.Reads == 1, "cancel never reads or creates");
        host.ReadOnly = true; await model.RefreshAsync(); Check(model.CanExcerpt(first), "read-only and branch sources may be quoted");
        host.Running = true; await model.RefreshAsync(); Check(model.CanExcerpt(first) && !model.CanExcerpt(last), "later running turn does not block completed history");
        host.Running = false; host.Acp = true; await model.RefreshAsync(); Check(!model.CanExcerpt(first), "ACP is excluded independently of branching capability");
        host.Acp = false; await model.RefreshAsync(); await model.SelectExcerptAsync(first, Locale);
        host.Replace = true; await model.RefreshAsync(); Check(model.ExcerptStart == null && !model.CanExcerpt(first), "replaced persisted identity invalidates the range");
        host.Replace = false; await model.RefreshAsync();
        Check(model.ExcerptStart == null, "restoring a replaced turn does not restore a cancelled excerpt selection");

        host.PendingRead = new(TaskCreationOptions.RunContinuationsAsynchronously);
        await model.SelectExcerptAsync(first, Locale); var pending = model.SelectExcerptAsync(last, Locale);
        Check(model.ExcerptBusy && !model.CanExcerpt(first), "pending read blocks repeat clicks");
        await model.SelectExcerptAsync(last, Locale);
        await model.OpenAsync("p", "elsewhere"); host.PendingRead.SetResult(); await pending; host.PendingRead = null;
        Check(host.Creates == 1 && model.CurrentSessionId == "elsewhere", "late read cannot create or navigate after leaving the source");

        await model.OpenAsync("p", "source"); host.PendingCreate = new(TaskCreationOptions.RunContinuationsAsynchronously);
        await model.SelectExcerptAsync(first, Locale); pending = model.SelectExcerptAsync(first, Locale);
        await model.OpenAsync("p", "elsewhere"); host.PendingCreate.SetResult(); Check(await pending == null, "late creation never takes navigation back"); host.PendingCreate = null;
        await model.OpenAsync("p", "quoted-2"); Check(model.Quotes.Single().Source == "Source · turn 3", "acknowledged late creation keeps its quote for later opening");

        await model.OpenAsync("p", "source"); host.FailCreate = true;
        await model.SelectExcerptAsync(first, Locale); await model.SelectExcerptAsync(first, Locale);
        var creates = host.Creates; Check(model.ExcerptUncertain && model.ExcerptFailure != null, "lost creation acknowledgement is explicit");
        await model.OpenAsync("p", "elsewhere"); await model.OpenAsync("p", "source");
        await model.SelectExcerptAsync(first, Locale); Check(host.Creates == creates && model.ExcerptStart == null, "returning to source does not replay uncertain creation");
        model.AcknowledgeExcerptCreation(); host.FailCreate = false; host.BadIdentity = true;
        await model.SelectExcerptAsync(first, Locale); await model.SelectExcerptAsync(first, Locale);
        Check(!model.ExcerptUncertain && model.ExcerptFailure != null && host.Creates == creates, "mismatched read cannot create but remains safely retryable");
        host.BadIdentity = false; host.FailRead = true;
        await model.SelectExcerptAsync(first, Locale); await model.SelectExcerptAsync(first, Locale);
        Check(host.Creates == creates && !model.ExcerptUncertain && model.ExcerptStart == null, "read failure leaves the source intact");
        Check(!model.CanExcerpt(first with { ProjectId = "foreign" }) && !model.CanExcerpt(first with { Role = "assistant" }), "scope and role are checked at execution");
        var safe = NativeComposerQuote.FromExcerpt(new("s", "x`\ny", 0, 0, "hello", false), Locale);
        Check(safe.Message.StartsWith("Quoted turns from an earlier conversation `x\\` y · turn 1`"), "source caption escapes backticks like WebView");
        Check(new NativeComposerQuote("s", 0, "excerpt", "  first line\nsecond line").Label == "first line", "quote chip uses the first line");
        Check(new NativeComposerQuote("s", 0, "excerpt", string.Concat(Enumerable.Repeat("🧪", 41))).Label == string.Concat(Enumerable.Repeat("🧪", 40)) + "…",
            "quote chip truncation preserves Unicode scalar boundaries");
    }

    private sealed class Fake : INativeSettingsClient
    {
        public bool ReadOnly, Running, Acp, Replace, FailCreate, FailRead, BadIdentity;
        public int Reads, Creates, Start, End; private ulong sequence;
        public TaskCompletionSource? PendingRead, PendingCreate;
        public async Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command is "list_models" or "list_acp_agents") return new JsonArray();
            if (command == "native_conversation_snapshot")
            {
                var turns = Enumerable.Range(0, 4).Select(n => new NativeTurnIdentity(n, n * 2 + 1, Replace && n == 2 ? "changed" : "turn-" + n)).ToArray();
                var value = new ConversationSnapshot(ConversationSnapshot.SchemaId, "e", ++sequence, "p", args["session_id"]!.ToString(),
                    [new("user", "first", null, null, null, null), new("assistant", "answer", null, null, null, null), new("user", "last", null, null, null, null)],
                    null, Running, false, ReadOnly, "", null, null, [], AcpAgentId: Acp ? "agent" : null, UserOffset: 2, HistoryState: new("r", false, false, turns));
                return JsonSerializer.SerializeToNode(value, ConversationSnapshot.JsonOptions);
            }
            if (command == "native_conversation_excerpt")
            {
                Reads++; Check(projectId == "p", "explicit project scope required");
                Start = args["start"]!["user_index"]!.GetValue<int>(); End = args["end"]!["user_index"]!.GetValue<int>();
                if (PendingRead != null) await PendingRead.Task;
                if (FailRead) throw new IOException("synthetic read failure");
                return JsonSerializer.SerializeToNode(new NativeConversationExcerpt(BadIdentity ? "foreign" : "p",
                    args["start"]!.Deserialize<NativeTurnIdentity>(ConversationSnapshot.JsonOptions)!, args["end"]!.Deserialize<NativeTurnIdentity>(ConversationSnapshot.JsonOptions)!,
                    new(args["session_id"]!.ToString(), "Source", Start, End, "User:\nquestion\n\nAssistant:\nanswer", true)), ConversationSnapshot.JsonOptions);
            }
            if (command == "native_conversation_create")
            {
                var number = ++Creates; if (PendingCreate != null) await PendingCreate.Task;
                if (FailCreate) throw new IOException("synthetic lost create acknowledgement");
                return JsonValue.Create("quoted-" + number);
            }
            if (command.Contains("send")) throw new Exception("Quoting must not send any message");
            return null;
        }
    }
}
