using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeIdleCompactionTests
{
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        const long now = 1_800_000_000_000;
        Check(!NativeIdleCompaction.ShouldPrompt(0, now - 100_000_000, now, true), "Zero disables idle advice.");
        Check(!NativeIdleCompaction.ShouldPrompt(24, 2000, now, true), "Ranking timestamps never trigger an idle prompt.");
        Check(!NativeIdleCompaction.ShouldPrompt(24, now - 86_399_999, now, true), "The threshold is not rounded up.");
        Check(NativeIdleCompaction.ShouldPrompt(24, now - 86_400_000, now, true), "The exact threshold is inclusive.");
        Check(!NativeIdleCompaction.ShouldPrompt(24, now + 1, now, true), "Future activity is not idle.");
        Check(!NativeIdleCompaction.ShouldPrompt(24, now - 86_400_000, now, false), "A blank conversation has nothing to compact.");
        Check(!NativeIdleCompaction.ShouldPrompt(ulong.MaxValue, 1_000_000_000_000, long.MaxValue, true), "Large preferences cannot overflow into a spurious prompt.");
        Check(NativeIdleCompaction.Hours(new JsonObject()) == 24 && NativeIdleCompaction.Hours(new JsonObject { ["semantic_compact_idle_hours"] = 0 }) == 0,
            "Missing preference defaults to 24 hours; explicit zero remains disabled.");
        Check(NativeIdleCompaction.Hours(new JsonObject { ["semantic_compact_idle_hours"] = 3L }) == 3
            && NativeIdleCompaction.Hours(JsonNode.Parse("{\"semantic_compact_idle_hours\":0}")) == 0,
            "Typed and deserialized JSON numbers share the same preference semantics.");
        Check(NativeIdleCompaction.Hours(new JsonObject { ["semantic_compact_idle_hours"] = -1 }) == 24
            && NativeIdleCompaction.Hours(new JsonObject { ["semantic_compact_idle_hours"] = "0" }) == 24,
            "Invalid negative and string preferences do not disable the prompt.");
        var old = DateTimeOffset.UtcNow.AddHours(-48).ToUnixTimeMilliseconds();
        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "old", lastActivity: old); model.Draft = "Unsent research question";
        Check(model.CanShowIdleCompaction && model.IdleCompactionPrompt?.Hours == 24 && host.Sends == 0, "Opening an old conversation offers advice without sending.");
        model.DismissIdleCompaction(); await model.OpenAsync("p", "old", lastActivity: old);
        Check(model.IdleCompactionPrompt == null && host.PreferenceReads == 1, "Not now/Escape suppress repeated advice for this window and session.");
        await model.OpenAsync("p", "next", lastActivity: old); model.Draft = "Keep the draft";
        Check(model.AcceptIdleCompaction() && model.Compaction is { Semantic: true, Started: false, Visible: true }
            && model.IdleCompactionPrompt == null && model.Draft == "Keep the draft" && host.Sends == 0,
            "Accept opens semantic confirmation only and preserves the draft.");
        model.DismissCompaction(); await model.OpenAsync("p", "next", lastActivity: old);
        Check(model.IdleCompactionPrompt == null, "Accept also records the session dismissal.");
        host.Hours = 0; await model.OpenAsync("p", "disabled", lastActivity: old);
        Check(model.IdleCompactionPrompt == null, "The committed zero-hour preference is respected.");
        host.Hours = 1; await model.OpenAsync("p", "custom", lastActivity: DateTimeOffset.UtcNow.AddHours(-2).ToUnixTimeMilliseconds());
        Check(model.IdleCompactionPrompt?.Hours == 1, "Prompt copy uses the committed configured threshold.");
        host.HasUser = false; await model.OpenAsync("p", "empty", lastActivity: old);
        Check(model.IdleCompactionPrompt == null, "Assistant-only or empty transcripts never prompt."); host.HasUser = true;
        foreach (var mode in new[] { "running", "readonly", "acp" })
        {
            host.Mode = mode; var reads = host.PreferenceReads; await model.OpenAsync("p", mode, lastActivity: old);
            Check(model.IdleCompactionPrompt == null && host.PreferenceReads == reads, "Running/read-only/ACP conversations do not read or prompt.");
        }
        host.Mode = ""; host.Fail = true; await model.OpenAsync("p", "failed", lastActivity: old);
        Check(model.IdleCompactionPrompt == null && model.ConnectionError == null && !model.Loading, "Optional preference read failure does not block the conversation.");
        host.Fail = false; host.Pending = new(); var opening = model.OpenAsync("p", "pending", lastActivity: old);
        await model.OpenAsync("p", "other"); host.Pending.SetResult(new JsonObject { ["semantic_compact_idle_hours"] = 24 }); await opening;
        Check(model.IdleCompactionPrompt == null, "Late preferences cannot target a newly selected conversation.");
        host.Pending = new(); opening = model.OpenAsync("p", "activity", lastActivity: old);
        host.ExtraUser = true; await model.RefreshAsync(); host.Pending.SetResult(new JsonObject()); await opening;
        Check(model.IdleCompactionPrompt == null, "A new turn while loading preferences cancels stale idle advice.");
        host.Pending = null; host.ExtraUser = false; await model.OpenAsync("p", "epoch", lastActivity: old);
        host.Epoch = "restart"; await model.RefreshAsync();
        Check(!model.CanShowIdleCompaction && !model.AcceptIdleCompaction() && host.Sends == 0, "Host restart invalidates a visible prompt.");
        model.Pause(); Check(model.IdleCompactionPrompt == null, "Leaving the workspace removes its prompt.");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public ulong Hours = 24; public string Mode = "", Epoch = "e"; public bool HasUser = true, ExtraUser, Fail;
        public int PreferenceReads, Sends; public TaskCompletionSource<JsonNode?>? Pending; private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "get_settings")
            {
                PreferenceReads++; if (Fail) throw new IOException("Synthetic preference failure");
                return Pending?.Task ?? Task.FromResult<JsonNode?>(new JsonObject { ["semantic_compact_idle_hours"] = Hours });
            }
            JsonNode? result = null;
            if (command == "native_conversation_snapshot")
            {
                ConversationItem Row(string role, string text) => new(role, text, null, null, null, null);
                var items = new List<ConversationItem>(); if (HasUser) items.Add(Row("user", "Research question"));
                items.Add(Row("assistant", "An older response")); if (ExtraUser) items.Add(Row("user", "New activity"));
                result = JsonSerializer.SerializeToNode(new ConversationSnapshot(ConversationSnapshot.SchemaId, Epoch, ++sequence, projectId!, args["session_id"]!.GetValue<string>(),
                    items.ToArray(), null, Mode == "running", false, Mode == "readonly", Mode == "acp" ? "acp:agent" : "m", null, null, [], AcpAgentId: Mode == "acp" ? "agent" : null), ConversationSnapshot.JsonOptions);
            }
            else if (command is "list_models" or "list_acp_agents") result = new JsonArray();
            else if (command == "native_conversation_send") Sends++;
            return Task.FromResult(result);
        }
    }
}
