using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeErrorResumeTests
{
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        (string Message, string? Key)[] hints = [
            ("ACP turn failed: api: 401 invalid api key", "acp_no_fallback"),
            ("api: 400 insufficient_quota context_length_exceeded", "balance"),
            ("api: 402 too many tokens", "balance"), ("api: 429 context_length_exceeded", "context"),
            ("api: 400 image_url unsupported", "image"), ("api: 400 unknown model", "model_name"),
            ("api: 401 Unauthorized", "auth"), ("api: 429 rate_limit", "rate"), ("api: 503 overloaded", "server"),
            ("http: DNS resolution failed", "network"), ("api: 400 no tool output found for tool call", "tool_pairing"),
            ("api: 400 missing x-opencode-session", "opencode_session"), ("api: 400 malformed request", "bad_request"),
            ("local disk failed", null), ("api: 418 teapot", null), ("maximum context length", null)
        ];
        foreach (var (message, key) in hints)
            Check(NativeErrorPresentation.HintKey(message) == (key == null ? null : "err.hint." + key), "Provider hints preserve WebView cause precedence: " + message);
        Check(NativeErrorPresentation.Message(new("assistant", "Error: api: 503", null, null, null, null)) == "api: 503"
            && NativeErrorPresentation.Message(new("user", "Error: api: 503", null, null, null, null)) == null,
            "Only assistant error rows remove the exact Error prefix; user text remains ordinary text.");

        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "source"); model.Draft = "Keep this new question";
        model.AddReference(new(new("session", "source-ref"), "Source reference", ""));
        var original = model.Snapshot!; var display = NativeErrorPresentation.WithLiveError(original);
        Check(original.Items.Length == 2 && display.Items.Length == 3 && display.Items[1] == original.Items[1],
            "Live errors append a view row without replacing persisted completed work or mutating the snapshot.");
        Check(NativeErrorPresentation.WithLiveError(display).Items.Length == 3
            && NativeErrorPresentation.WithLiveError(original with { Running = true }).Items.Length == 2,
            "Repeated projections cannot duplicate the error, and a running turn does not surface a stale failure.");
        var target = model.ErrorTarget(display, 2)!;
        Check(model.ShowsErrorResume(target) && model.CanResumeError(target), "A settled mutable non-ACP error can resume.");
        await model.ResumeErrorAsync(target); await model.ResumeErrorAsync(target); await model.RefreshAsync();
        Check(host.Resumes == 1 && host.LastSend?["message"]?.GetValue<string>() == ""
            && host.LastSend?["resume"]?.GetValue<bool>() == true && host.LastSend?["resume_after"]?.GetValue<string>() == target.RequestId
            && host.LastSend?["attachments"] == null && host.LastSend?["references"] == null,
            "One explicit action sends an empty resume request without replaying composer data.");
        Check(model.Draft == "Keep this new question" && model.References.Single().Reference.Id == "source-ref"
            && model.ErrorWasResumed(target), "Acknowledgement hides the selected error and preserves draft and references.");

        foreach (var mode in new[] { "readonly", "acp", "acp-prefix", "running", "stopping", "missing-request" })
        {
            host.Mode = mode; await Failure();
            var candidate = model.ErrorTarget(NativeErrorPresentation.WithLiveError(model.Snapshot!), 2);
            Check(!model.CanResumeError(candidate), "Ineligible error cannot send: " + mode);
            if (mode is "readonly" or "acp" or "acp-prefix") Check(!model.ShowsErrorResume(candidate), "Read-only/ACP rows omit Resume.");
        }
        host.Mode = ""; await Failure(); var stale = Target();
        Check(!model.ShowsErrorResume(stale with { Failure = "ACP turn failed: disconnected" }),
            "An older ordinary error cannot offer native Resume for a current ACP failure.");
        host.Request = Guid.NewGuid().ToString(); await model.RefreshAsync();
        await model.ResumeErrorAsync(stale); Check(host.Resumes == 1, "A changed request invalidates the old button.");
        stale = Target(); host.Epoch = "new-host"; await model.RefreshAsync();
        await model.ResumeErrorAsync(stale); Check(host.Resumes == 1, "A replaced host invalidates the old button.");
        stale = Target(); await model.OpenAsync("p", "other");
        await model.ResumeErrorAsync(stale); Check(host.Resumes == 1, "Leaving the conversation cannot retarget its action.");

        await model.OpenAsync("p", "source"); await Failure(); host.Unknown = true;
        var unknown = Target(); await model.ResumeErrorAsync(unknown); await model.RefreshAsync(); await model.ResumeErrorAsync(unknown);
        Check(host.Resumes == 2 && model.UncertainSend && !model.CanResumeError(unknown), "Unknown outcome remains visible without automatic or double-click replay.");
        Check(model.ResumeFailureDetail == "Unknown resume outcome", "The unconfirmed resume detail remains separate for localized presentation.");
        await model.OpenAsync("p", "other"); await model.OpenAsync("p", "source");
        Check(model.UncertainSend, "Uncertainty survives navigation back to its source conversation.");
        host.Request = host.LastSend!["request_id"]!.GetValue<string>(); host.Error = null; await model.RefreshAsync();
        Check(!model.UncertainSend && model.ResumeFailureDetail == null, "The exact request acknowledgement clears uncertainty and its notice.");

        host.Unknown = false; await Failure(); host.Pending = new(); var late = Target();
        var pending = model.ResumeErrorAsync(late);
        Check(!model.CanResumeError(late), "Busy resume disables repeated clicks while the transport is pending.");
        await model.OpenAsync("p", "other"); model.Draft = "Other conversation draft";
        host.Pending.SetResult(); await pending;
        Check(model.CurrentSessionId == "other" && model.Draft == "Other conversation draft" && model.OperationError == null,
            "Late acknowledgement cannot navigate or change another conversation's composer.");

        async Task Failure() { host.Error = host.Mode == "acp-prefix" ? "ACP turn failed: disconnected" : "api: 503 overloaded"; host.Request = Guid.NewGuid().ToString(); await model.RefreshAsync(); }
        NativeErrorTarget Target() => model.ErrorTarget(NativeErrorPresentation.WithLiveError(model.Snapshot!), 2)!;
    }

    private sealed class Fake : INativeSettingsClient
    {
        public string? Error = "api: 503 overloaded"; public string Request = Guid.NewGuid().ToString(), Epoch = "e", Mode = "";
        public int Resumes; public bool Unknown; public JsonObject? LastSend; public TaskCompletionSource? Pending; private ulong sequence;
        public async Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "native_conversation_snapshot")
            {
                var id = args["session_id"]!.GetValue<string>(); var source = id == "source";
                return JsonSerializer.SerializeToNode(new ConversationSnapshot(ConversationSnapshot.SchemaId, Epoch, ++sequence, projectId!, id,
                    source ? [new("user", "Analyze results", null,null,null,null),new("assistant", "Saved completed work",null,null,null,null)] : [],
                    null, source && Mode == "running", source && Mode == "stopping", Mode == "readonly", Mode == "acp" ? "acp:a" : "m",
                    source && Mode != "missing-request" ? Request : null, source ? Error : null, [], ComposerReferences: true,
                    AcpAgentId: Mode == "acp" ? "a" : null), ConversationSnapshot.JsonOptions);
            }
            if (command == "native_conversation_send")
            {
                Resumes++; LastSend = (JsonObject)args.DeepClone(); if (Unknown) throw new IOException("Unknown resume outcome");
                if (Pending != null) await Pending.Task;
                Request = args["request_id"]!.GetValue<string>(); Error = null; return new JsonObject();
            }
            return command is "list_models" or "list_acp_agents" ? new JsonArray() : null;
        }
    }
}
