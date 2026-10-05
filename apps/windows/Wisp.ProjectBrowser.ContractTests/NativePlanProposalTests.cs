using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativePlanProposalTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        foreach (var acp in new[] { false, true })
        {
            var host = new Fake { Acp = acp }; var model = await Open(host);
            model.Draft = "只执行第一步，保留第二步";
            var target = model.LatestProposal!;
            Check(model.CanDecidePlan(target), "latest native and ACP plans expose decisions");
            host.ModeReply = new(TaskCreationOptions.RunContinuationsAsynchronously);
            var approve = model.DecidePlanAsync(target, true);
            await model.DecidePlanAsync(target, true);
            Check(host.Writes.Count == 1 && host.Sent.Count == 0 && model.Busy, "mode acknowledgement precedes send and duplicate clicks do not write");
            host.ModeReply.SetResult(); await approve;
            Check(host.Sent.SequenceEqual(["只执行第一步，保留第二步"]) && model.Draft == "", $"approve sends the user's existing draft once: sent={host.Sent.Count}, draft={model.Draft}, error={model.OperationError}, connection={model.ConnectionError}");
            Check(host.Writes[0] == (acp ? "acp:default" : "native:false") && !host.PlanMode, "exact advertised exit mode is confirmed before execution");
        }
        var saveHost = new Fake(); var saveModel = await Open(saveHost); saveModel.Draft = "未发送的修改意见";
        await saveModel.DecidePlanAsync(saveModel.LatestProposal!, false);
        Check(!saveHost.PlanMode && saveHost.Sent.Count == 0 && saveModel.Draft == "未发送的修改意见", "save exits mode without dispatching or consuming the draft");
        var defaultHost = new Fake(); var defaultModel = await Open(defaultHost);
        await defaultModel.DecidePlanAsync(defaultModel.LatestProposal!, true);
        Check(defaultHost.Sent.Single() == "批准并执行", "empty draft approval uses the explicit execution instruction");

        foreach (var scenario in new[] { "lost-mode", "unconfirmed-mode", "changed-plan", "changed-draft", "navigation", "lost-send" })
        {
            var host = new Fake(); var model = await Open(host); model.Draft = "my draft"; var target = model.LatestProposal!;
            host.LoseMode = scenario == "lost-mode"; host.KeepPlanMode = scenario == "unconfirmed-mode";
            host.LoseSend = scenario == "lost-send";
            host.ModeReply = new(TaskCreationOptions.RunContinuationsAsynchronously);
            var decision = model.DecidePlanAsync(target, true);
            if (scenario == "changed-plan") host.Content = "Revised **plan**";
            if (scenario == "changed-draft") model.Draft = "new draft";
            if (scenario == "navigation") { await model.OpenAsync("p", "other"); model.Draft = "other draft"; }
            host.ModeReply.SetResult(); await decision;
            if (scenario == "lost-send")
            {
                await model.DecidePlanAsync(target, true); await model.RefreshAsync();
                Check(host.Sent.Count == 1 && model.UncertainSend && model.Draft == "my draft", "uncertain dispatch preserves draft and never replays");
            }
            else
            {
                Check(host.Sent.Count == 0, scenario + " must not dispatch an execution turn");
                if (scenario == "navigation") Check(model.Draft == "other draft" && model.OperationError == null, "late mode completion cannot change the new session");
                else Check(model.Draft == (scenario == "changed-draft" ? "new draft" : "my draft") && model.OperationError != null, "partial decision preserves draft and explains outcome");
            }
            if (scenario is "lost-mode" or "unconfirmed-mode")
            {
                await model.RefreshAsync(); await model.DecidePlanAsync(target, true);
                Check(host.Writes.Count == 1 && model.PlanDecisionUncertain(target), "an uncertain mode mutation requires explicit acknowledgement before another attempt");
            }
        }
        var scopedHost = new Fake(); var scoped = await Open(scopedHost); var old = scoped.LatestProposal!;
        scopedHost.Content = "replacement"; await scoped.RefreshAsync(); await scoped.DecidePlanAsync(old, true);
        scopedHost.Running = true; await scoped.RefreshAsync(); await scoped.DecidePlanAsync(scoped.LatestProposal!, true);
        scopedHost.Running = false; scopedHost.ReadOnly = true; await scoped.RefreshAsync(); await scoped.DecidePlanAsync(scoped.LatestProposal!, true);
        Check(scopedHost.Writes.Count == 0, "stale, running and read-only proposals cannot change mode");
        var ready = scoped.Snapshot! with { ReadOnly = false };
        Check(NativePlanProposals.Latest(ready with { Items = [ready.Items[1]] }) == null, "a proposal without an owning user turn has no decisions");
        Check(NativePlanProposals.Latest(ready with { Items = [.. ready.Items, ready.Items[0]] }) == null, "a new user turn retires the previous proposal");
        Check(NativePlanProposals.Latest(ready with { Items = [ready.Items[0], ready.Items[1] with { Proposal = new([], "native") }] }) == null,
            "malformed or empty proposals remain readable without execution controls");
        var acpState = new NativeAcpSessionState("s", JsonNode.Parse("""{"currentModeId":"plan_mode","availableModes":[{"id":"plan_mode"},{"id":"agent"}]}""")!.AsObject(), []);
        Check(NativePlanProposals.AcpExitMode(acpState) == "agent", "ACP without default exits to its first advertised non-plan mode");
        acpState.Modes!["availableModes"] = JsonNode.Parse("""[{"id":"plan_mode"}]""");
        Check(NativePlanProposals.AcpExitMode(acpState) == null, "an agent without an exit mode cannot offer a nonworking decision");
        Console.WriteLine("Native plan proposals: mode ordering, draft preservation, exact scope, stale revisions, uncertainty and no replay passed.");
    }
    private static async Task<WorkspaceConversationModel> Open(Fake host)
    {
        var model = new WorkspaceConversationModel(new NativeConversationClient(host), host); await model.OpenAsync("p", "s"); return model;
    }
    private sealed class Fake : INativeSettingsClient
    {
        public bool Acp, PlanMode = true, Running, ReadOnly, LoseMode, KeepPlanMode, LoseSend;
        public string Content = "Inspect **samples**\n\n```python\nprint(1)\n```";
        public TaskCompletionSource? ModeReply;
        public List<string> Writes = [], Sent = [];
        private ulong sequence;
        public async Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            var session = args["session_id"]?.GetValue<string>() ?? "s";
            if (command == "native_conversation_snapshot")
            {
                var state = new NativeAcpSessionState(session, new JsonObject { ["currentModeId"] = PlanMode ? "plan" : "default",
                    ["availableModes"] = JsonNode.Parse("""[{"id":"default","name":"Agent"},{"id":"plan","name":"Plan"}]""") }, []);
                var proposal = new NativePlanProposal([new(Content, "pending", "high")], Acp ? "acp" : "native");
                var snapshot = new ConversationSnapshot(ConversationSnapshot.SchemaId, "host", ++sequence, "p", session,
                    [new("user", "plan it", null, null, null, null), new("plan", "raw plan", null, null, null, null, Proposal: proposal)], null,
                    Running, false, ReadOnly, Acp ? "acp:qa" : "model", null, null, [], PlanMode: Acp ? null : PlanMode, AcpState: Acp ? state : null);
                return JsonSerializer.SerializeToNode(snapshot, ConversationSnapshot.JsonOptions);
            }
            if (command is "native_conversation_plan" or "native_conversation_acp_setting")
            {
                Check(session == "s" && projectId == "p", "plan changes target the captured project and session");
                Writes.Add(command == "native_conversation_plan" ? "native:" + args["enabled"]!.GetValue<bool>().ToString().ToLowerInvariant()
                    : "acp:" + args["change"]!["id"]!.GetValue<string>());
                if (ModeReply != null) await ModeReply.Task;
                if (LoseMode) throw new IOException("mode reply lost");
                if (!KeepPlanMode) PlanMode = false;
                if (command == "native_conversation_plan") return JsonValue.Create(PlanMode);
            }
            if (command == "native_conversation_send")
            {
                Check(!PlanMode && session == "s", "execution is never sent in plan mode or into another session");
                Sent.Add(args["message"]!.GetValue<string>());
                if (LoseSend) throw new IOException("send reply lost");
            }
            return command is "list_models" or "list_acp_agents" ? new JsonArray() : null;
        }
    }
}
