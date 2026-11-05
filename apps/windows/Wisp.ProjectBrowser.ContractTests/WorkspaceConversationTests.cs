using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class WorkspaceConversationTests
{
    public static async Task RunAsync(string projectFixture)
    {
        var snapshotPath = Path.GetFullPath(Path.Combine(Path.GetDirectoryName(projectFixture)!, "../../native-conversations/v1/snapshot.json"));
        await WelcomePresentationAsync(snapshotPath);
        await ProjectDraftAsync(snapshotPath);
        await MemoInsertAsync(snapshotPath);
        await ModelSelectionAsync(snapshotPath);

        var client = new Fake();
        var model = new WorkspaceConversationModel(client, client);
        client.FailRead = true;
        await model.OpenAsync("project-a", "session-a");
        Check(client.Seen == 0, "failed open does not mark seen");
        client.FailRead = false;
        client.Reads = [Fixture(snapshotPath)];
        await model.OpenAsync("project-a", "session-a");
        Check(client.Seen == 1, "successful open marks seen once");

        client.Reads = [Fixture(snapshotPath, running: true) with { UserOffset = 4 }];
        await model.OpenAsync("project-a", "session-a");
        Check(await model.RevealChatQuestionAsync(new(4, null)) && model.ScrollTarget == 0 && !model.ShowingHistory,
            "project search can jump within a running conversation's loaded transcript");
        Check(!await model.RevealChatQuestionAsync(new(0, 8)) && !model.ShowingHistory,
            "an older match cannot replace a running conversation with a history page");

        client.Reads =
        [
            Fixture(snapshotPath, sequence: 7),
            Fixture(snapshotPath, sequence: 6),
            Fixture(snapshotPath, sequence: 1, epoch: "host-two"),
            Fixture(snapshotPath, sequence: 99)
        ];
        await model.OpenAsync("project-a", "session-a");
        await model.RefreshAsync(); Check(model.Snapshot?.Sequence == 7, "stale sequence is ignored");
        await model.RefreshAsync(); Check(model.Snapshot?.Epoch == "host-two", "host restart replaces the cursor");
        await model.RefreshAsync(); Check(model.Snapshot?.Epoch == "host-two" && model.VisibleItems.Length == 2, "retired host cannot return");

        client = new Fake { FailSend = true, Reads = [Fixture(snapshotPath)] };
        model = new WorkspaceConversationModel(client, client);
        await model.OpenAsync("project-a", "session-a");
        model.Draft = "do work"; await model.SendAsync();
        Check(model.UncertainSend && model.Draft == "do work" && !model.CanSend, "ambiguous send keeps the draft");
        await model.SendAsync(); Check(client.Sends == 1, "uncertain send is not replayed");
        var requestId = client.LastRequestId;
        client.FailSend = false;
        client.Reads = [Fixture(snapshotPath, sequence: 9, running: true, requestId: requestId)];
        await model.RefreshAsync();
        Check(model.Draft == "" && !model.UncertainSend && model.Snapshot?.Running == true, "snapshot request_id acknowledges the send");

        client = new Fake { Reads = [Fixture(snapshotPath), Fixture(snapshotPath, session: "session-b")] };
        model = new WorkspaceConversationModel(client, client);
        await model.OpenAsync("project-a", "session-a"); model.Draft = "unsent";
        var hold = new TaskCompletionSource<ConversationSnapshot>();
        client.Hold = hold;
        var stale = model.RefreshAsync();
        while (client.Hold != null) await Task.Yield();
        await model.OpenAsync("project-a", "session-b");
        hold.SetResult(Fixture(snapshotPath, sequence: 999));
        await stale;
        Check(model.Snapshot?.SessionId == "session-b", "late read cannot reopen the previous session");
        await model.OpenAsync("project-a", "session-a");
        Check(model.Draft == "unsent" && client.Sends == 0, "draft survives navigation and does not stop the agent");

        client = new Fake { Reads = [Fixture(snapshotPath) with { NextBeforeSeq = 10 }] };
        model = new WorkspaceConversationModel(client);
        await model.OpenAsync("project-a", "session-a");
        hold = client.Hold = new();
        var oldHistory = model.OlderAsync();
        client.Reads = [Fixture(snapshotPath, session: "session-b")];
        await model.OpenAsync("project-a", "session-b");
        hold.SetResult(Fixture(snapshotPath) with { NextBeforeSeq = 5 });
        await oldHistory;
        Check(model.History == null && !model.ShowingHistory, "late history cannot contaminate a different conversation");

        client.Reads = [Fixture(snapshotPath) with { NextBeforeSeq = 10 }];
        await model.OpenAsync("project-a", "session-a");
        hold = client.Hold = new();
        oldHistory = model.OlderAsync();
        model.Latest();
        hold.SetResult(Fixture(snapshotPath) with { NextBeforeSeq = 5 });
        await oldHistory;
        Check(model.History == null && !model.ShowingHistory, "return to latest invalidates an in-flight older page");

        hold = client.Hold = new();
        oldHistory = model.OlderAsync();
        client.Reads = [Fixture(snapshotPath) with { UserOffset = 0 }];
        await model.OpenQuestionAsync(new(0, "检查样本", 20, null, null));
        hold.SetResult(Fixture(snapshotPath) with { UserOffset = 9 });
        await oldHistory;
        Check(model.ShowingHistory && model.History?.UserOffset == 0 && model.ScrollTarget == 0,
            "an older page cannot replace a newer outline navigation");

        hold = client.Hold = new();
        var outlineRead = model.OpenQuestionAsync(new(0, "检查样本", 20, null, null));
        model.Latest();
        hold.SetException(new IOException("late history failure"));
        await outlineRead;
        Check(!model.ShowingHistory && model.History == null && model.OperationError == null,
            "leaving outline history also discards its late failure");

        using (var cancelledOutline = new CancellationTokenSource())
        {
            hold = client.Hold = new();
            var cancelledRead = model.OpenQuestionAsync(new(0, "检查样本", 20, null, null), cancelledOutline.Token);
            cancelledOutline.Cancel();
            hold.SetResult(Fixture(snapshotPath) with { UserOffset = 0 });
            Check(!await cancelledRead && model.History == null && !model.ShowingHistory,
                "closing the outline rejects a late page even when the transport ignores cancellation");
        }

        client = new Fake { Reads = [Fixture(snapshotPath)] };
        model = new WorkspaceConversationModel(client, client);
        await model.OpenAsync("project-a", "session-a"); model.Draft = "hello";
        client.FailRead = true; await model.RefreshAsync();
        Check(model.VisibleItems.Length == 2 && !model.CanSend, "offline refresh keeps transcript and blocks send");
        client.FailRead = false; client.Reads = [Fixture(snapshotPath, sequence: 8)]; await model.RefreshAsync();
        Check(model.ConnectionError == null && model.CanSend, "reconnect restores sending");

        client = new Fake { FailSend = true, Reads = [Fixture(snapshotPath)] };
        model = new WorkspaceConversationModel(client, client);
        await model.OpenAsync("project-a", "session-a");
        model.Draft = "recover me"; await model.SendAsync();
        requestId = client.LastRequestId;
        client.Reads = [Fixture(snapshotPath, session: "session-b")];
        await model.OpenAsync("project-a", "session-b");
        client.FailSend = false;
        client.Reads = [Fixture(snapshotPath, sequence: 10, requestId: requestId, error: "model unavailable")];
        await model.OpenAsync("project-a", "session-a");
        Check(!model.UncertainSend && model.Draft == "recover me" && model.CanSend && client.Sends == 1,
            "accepted failure restores the draft after navigation");

        client = new Fake { Reads = [Fixture(snapshotPath)] };
        model = new WorkspaceConversationModel(client, client);
        await model.OpenAsync("project-a", "session-a");
        await model.StopAsync();
        Check(!model.CanStop && client.Stops == 0, "an idle session cannot receive a stop request");
        client.Reads = [Fixture(snapshotPath, sequence: 8, running: true)];
        await model.RefreshAsync();
        client.StopHold = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var stopping = model.StopAsync();
        Check(model.IsStopping && !model.CanStop && model.CanCompose && client.Stops == 1,
            "stop is acknowledged immediately without locking the draft while the host request is pending");
        await model.StopAsync();
        Check(client.Stops == 1, "a pending stop cannot be issued twice");
        client.Reads = [Fixture(snapshotPath, sequence: 9, running: true) with { Stopping = true }];
        client.StopHold.SetResult(); await stopping;
        Check(model.IsStopping && !model.CanStop && client.LastCommand == "stop" && client.LastSession == "session-a", "acknowledgement retains stopping until the host reports completion");
        client.Reads = [Fixture(snapshotPath, sequence: 10)];
        await model.RefreshAsync();
        Check(!model.IsStopping && !model.ShowStopButton, "a completed turn restores the idle send button");
        await model.ApproveAsync(new ConversationApproval("exact-id", "session-a", "Run?", "shell", "echo test"), false);
        Check(client.LastCommand == "approve" && client.LastApproval == "exact-id" && client.LastApproved == false,
            "approval uses the exact one-shot identity");

        client.Reads = [Fixture(snapshotPath, sequence: 11, running: true)];
        await model.RefreshAsync();
        client.StopHold = new(TaskCreationOptions.RunContinuationsAsynchronously); var delayedStop = model.StopAsync();
        client.Reads = [Fixture(snapshotPath, session: "session-b", running: true)];
        await model.OpenAsync("project-a", "session-b");
        Check(!model.IsStopping && model.CanStop, "another running session never inherits the pending stop");
        client.Reads = [Fixture(snapshotPath, sequence: 12, running: true)];
        await model.OpenAsync("project-a", "session-a");
        Check(model.IsStopping && !model.CanStop, "reopening the original turn cannot duplicate its pending stop");
        client.StopHold.SetException(new IOException("synthetic interruption failure")); await delayedStop;
        Check(!model.IsStopping && model.CanStop, "a failed acknowledgement unlocks a deliberate retry");

        client = new Fake { FailSend = true, Reads = [Fixture(snapshotPath)] };
        model = new WorkspaceConversationModel(client, client);
        await model.OpenAsync("project-a", "session-a");
        model.Draft = "uncertain"; await model.SendAsync();
        client.Reads = [Fixture(snapshotPath, session: "session-b")];
        await model.OpenAsync("project-a", "session-b");
        client.Reads = [Fixture(snapshotPath, sequence: 8)];
        await model.OpenAsync("project-a", "session-a");
        Check(model.UncertainSend && model.Draft == "uncertain" && !model.CanSend, "uncertain send survives leave and return");
        model.AcknowledgeUncertainSend();
        Check(!model.UncertainSend && model.CanSend, "user acknowledgement unblocks send without retrying");

        model.RevealExcerpt("检查 样本");
        Check(model.ScrollTarget == 0, "excerpt selects the rendered user message");
        var first = model.ScrollRevision;
        model.RevealExcerpt("正在 检查样本…");
        Check(model.ScrollTarget == 1, "excerpt selects the later assistant message");
        model.ClearExcerpt(first);
        Check(model.RevealedExcerpt != null, "older highlight clear cannot drop a newer excerpt");
        model.ClearExcerpt(model.ScrollRevision);
        Check(model.RevealedExcerpt == null, "matching revision clears the excerpt");
        model.RevealExcerpt("missing");
        Check(model.OperationError != null && model.ScrollTarget == 1, "missing excerpt keeps the last successful target");
        model.Pause();

        Check(NativePanelPaths.Destination(".", "分析.R") == "分析.R", "file action stays in the workspace root");
        Check(NativePanelPaths.Destination("results", "figure 1.svg") == "results/figure 1.svg", "file action stays in the displayed directory");
        foreach (var name in new[] { "", " ", ".", "..", "../file", "a/b", "a\\b", "a\n", "a\0" })
        {
            try { NativePanelPaths.Destination(".", name); throw new Exception("expected invalid name: " + name); }
            catch (InvalidOperationException) { }
        }
        Console.WriteLine("WinUI live conversation model passed.");
        client = new Fake { Reads = [Fixture(snapshotPath)] };
        model = new WorkspaceConversationModel(client);
        await model.OpenAsync("project-a", "session-a");
        await model.SetPlanModeAsync(true);
        Check(!model.CanChangePlanMode && client.PlanWrites == 0, "older hosts do not expose an inert Plan toggle");
        client.Reads = [Fixture(snapshotPath, sequence: 8) with { PlanMode = false }];
        await model.RefreshAsync();
        model.Draft = "preserve this draft";
        var planHold = client.PlanHold = new();
        var changePlan = model.SetPlanModeAsync(true);
        Check(model.Busy && !model.CanSend && !model.CanChangePlanMode, "pending mode change blocks a competing send or toggle");
        client.Reads = [Fixture(snapshotPath, sequence: 9) with { PlanMode = true }];
        planHold.SetResult(); await changePlan;
        Check(model.Snapshot?.PlanMode == true && model.Draft == "preserve this draft", "confirmed Plan mode refresh preserves the draft");
        await model.SetPlanModeAsync(true);
        Check(client.PlanWrites == 1, "refresh and selecting the current mode do not write");
        client.Reads = [Fixture(snapshotPath, sequence: 10, running: true) with { PlanMode = true }];
        await model.RefreshAsync(); await model.SetPlanModeAsync(false);
        Check(client.PlanWrites == 1, "running turns cannot change their mode");
        client.Reads = [Fixture(snapshotPath, sequence: 11) with { PlanMode = true, ReadOnly = true }];
        await model.RefreshAsync(); await model.SetPlanModeAsync(false);
        Check(client.PlanWrites == 1, "read-only conversations cannot change their mode");
        client.Reads = [Fixture(snapshotPath, sequence: 12) with { PlanMode = true }];
        await model.RefreshAsync();
        client.FailPlan = true;
        client.Reads = [Fixture(snapshotPath, sequence: 13) with { PlanMode = true }];
        await model.SetPlanModeAsync(false);
        Check(client.PlanWrites == 2 && model.Snapshot?.PlanMode == true && model.OperationError != null
            && model.Draft == "preserve this draft", "lost Plan write is not replayed and preserves the authoritative mode and draft");
        client.FailPlan = false;
        planHold = client.PlanHold = new();
        changePlan = model.SetPlanModeAsync(false);
        client.Reads = [Fixture(snapshotPath, session: "session-b") with { PlanMode = true }];
        await model.OpenAsync("project-a", "session-b");
        planHold.SetResult(); await changePlan;
        Check(model.Snapshot?.SessionId == "session-b" && model.Snapshot.PlanMode == true,
            "late mode confirmation cannot reopen or alter a different conversation");
        client = new Fake { Reads = [Fixture(snapshotPath)] };
        model = new WorkspaceConversationModel(client);
        await model.OpenAsync("project-a", "session-a");
        await model.SetFastModeAsync(true);
        Check(!model.CanChangeFastMode && client.FastWrites == 0, "unsupported hosts and models cannot write Fast");
        client.Reads = [Fixture(snapshotPath, sequence: 8) with { FastMode = new(false, true) }];
        await model.RefreshAsync();
        model.Draft = "keep fast draft";
        var fastHold = client.FastHold = new();
        var changeFast = model.SetFastModeAsync(true);
        Check(model.Busy && !model.CanSend && !model.CanChangeFastMode && client.FastModel == "model-a",
            "pending Fast writes block sending and carry the displayed model identity");
        client.Reads = [Fixture(snapshotPath, sequence: 9) with { FastMode = new(true, false) }];
        fastHold.SetResult(); await changeFast;
        Check(model.Snapshot?.FastMode is { Enabled: true, Inherited: false } && model.Draft == "keep fast draft",
            "Fast confirmation refreshes effective and inherited state without clearing the draft");
        await model.SetFastModeAsync(true);
        Check(client.FastWrites == 1, "Fast polling and unchanged selections never write");
        client.Reads = [Fixture(snapshotPath, sequence: 10, running: true) with { FastMode = new(true, false) }];
        await model.RefreshAsync(); await model.SetFastModeAsync(false);
        Check(client.FastWrites == 1, "running turns cannot change Fast");
        client.Reads = [Fixture(snapshotPath, sequence: 11) with { FastMode = new(true, false), ReadOnly = true }];
        await model.RefreshAsync(); await model.SetFastModeAsync(false);
        Check(client.FastWrites == 1, "read-only conversations cannot change Fast");
        client.Reads = [Fixture(snapshotPath, sequence: 12) with { FastMode = new(true, false) }];
        await model.RefreshAsync();
        fastHold = client.FastHold = new();
        changeFast = model.SetFastModeAsync(false);
        fastHold.SetCanceled(); await changeFast;
        Check(!model.Busy && model.ConnectionError != null && !model.CanSend && model.Draft == "keep fast draft",
            "cancelled mode wait releases Busy but blocks sends until its outcome is read back");
        await model.SetFastModeAsync(false);
        Check(client.FastWrites == 2, "cancelled Fast write is not replayed while its result is unknown");
        client.Reads = [Fixture(snapshotPath, sequence: 13) with { FastMode = new(false, true) }];
        await model.RefreshAsync();
        Check(model.CanSend && model.ConnectionError == null && model.Snapshot?.FastMode?.Enabled == false,
            "authoritative read restores sending after a cancelled mode wait");
        using var alreadyCancelled = new CancellationTokenSource();
        alreadyCancelled.Cancel();
        await model.SetFastModeAsync(true, alreadyCancelled.Token);
        Check(client.FastWrites == 2 && !model.Busy, "already cancelled operations never start a host write");
    }

    private static async Task WelcomePresentationAsync(string path)
    {
        var empty = Fixture(path) with { Items = [] };
        var client = new Fake { Reads = [empty] };
        var model = new WorkspaceConversationModel(client);
        Check(model.ShowWelcome, "a project without a session shows the centered welcome");
        var hold = client.Hold = new TaskCompletionSource<ConversationSnapshot>();
        var opening = model.OpenAsync("project-a", "session-a");
        Check(!model.ShowWelcome, "loading a session does not flash an empty welcome");
        hold.SetResult(empty); await opening;
        Check(model.ShowWelcome, "an empty loaded session keeps the welcome above its composer");
        client.Reads = [empty with { Sequence = 8, Running = true }]; await model.RefreshAsync();
        Check(!model.ShowWelcome, "a running conversation is never presented as a new conversation");
        client.Reads = [Fixture(path, sequence: 9)]; await model.RefreshAsync();
        Check(!model.ShowWelcome, "received messages replace the welcome with the transcript");
        client.Reads = [empty with { Sequence = 10, NextBeforeSeq = 1 }]; await model.RefreshAsync();
        await model.OlderAsync();
        Check(!model.ShowWelcome, "an empty historical page does not offer a new conversation");
        model.Latest(); client.FailRead = true; await model.RefreshAsync();
        Check(!model.ShowWelcome, "a failed read remains visible instead of being covered by welcome content");
    }

    private static void Check(bool passed, string name)
    {
        if (!passed) throw new InvalidOperationException(name);
        Console.WriteLine("PASS conversation: " + name);
    }

    private static async Task MemoInsertAsync(string path)
    {
        var client = new Fake(); var model = new WorkspaceConversationModel(client, client);
        Check(!model.InsertMemoBody("private"), "memo insert requires a composer scope");
        await model.OpenProjectDraftAsync("project-a"); model.Draft = "existing\n";
        Check(model.InsertMemoBody("first\nsecond") && model.Draft == "existing\n\nfirst\nsecond", "memo body appends to the unsent project draft");
        Check(client.Creates == 0 && client.Sends == 0 && model.CurrentSessionId == null, "memo insert never creates or sends a session");
        await model.OpenProjectDraftAsync("project-b"); Check(model.Draft == "", "memo text does not leak to another project");
        await model.OpenProjectDraftAsync("project-a"); Check(model.Draft == "existing\n\nfirst\nsecond", "inserted memo survives local draft navigation");
        client.Reads = [Fixture(path) with { ReadOnly = true }]; await model.OpenAsync("project-a", "session-a");
        var previous = model.Draft; Check(!model.InsertMemoBody("blocked") && model.Draft == previous, "read-only conversations reject memo insertion");
        model.Pause();
    }

    private static async Task ProjectDraftAsync(string path)
    {
        var client = new Fake { Reads = [Fixture(path, session: "new") with { Items = [] }] };
        var model = new WorkspaceConversationModel(client, client);
        await model.OpenProjectDraftAsync("project-a");
        Check(model.CanCompose && !model.CanSubmit && model.ShowWelcome && client.Creates == 0 && client.Seen == 0,
            "opening an empty project exposes an editable composer without creating or opening a session");
        model.Draft = "first question";
        Check(model.CanSubmit && client.Creates == 0, "typing enables first send without persistence");
        await model.OpenProjectDraftAsync("project-b");
        Check(model.Draft == "", "a new project's first draft starts independently");
        model.Draft = "different question";
        await model.OpenProjectDraftAsync("project-a");
        Check(model.Draft == "first question", "unsent first draft survives project navigation");
        var hold = client.CreateHold = new();
        var creating = model.EnsureDraftSessionAsync();
        Check(client.Creates == 1 && !model.CanCompose && !model.CanSubmit, "creation disables competing first actions");
        Check(await model.EnsureDraftSessionAsync() == null && client.Creates == 1, "double submit cannot create two sessions");
        hold.SetResult("new");
        Check(await creating == "new" && model.Draft == "first question" && model.CanSend && client.Sends == 0,
            "confirmed creation carries the first draft into the opened session without sending by itself");
        await model.SendAsync();
        Check(client.Sends == 1 && client.LastSession == "new" && client.LastMessage == "first question" && model.Draft == "",
            "first send uses the confirmed session and consumes exactly the submitted draft");

        client = new Fake { Reads = [Fixture(path, session: "new") with { Items = [] }], FailCreate = true };
        model = new WorkspaceConversationModel(client, client);
        await model.OpenProjectDraftAsync("project-a"); model.Draft = "keep after lost reply";
        Check(await model.EnsureDraftSessionAsync() == null && model.DraftCreationUncertain && !model.CanSubmit,
            "ambiguous create preserves text and blocks replay");
        await model.OpenProjectDraftAsync("project-b"); await model.OpenProjectDraftAsync("project-a");
        await model.EnsureDraftSessionAsync();
        Check(client.Creates == 1 && model.Draft == "keep after lost reply" && model.DraftCreationUncertain,
            "navigation cannot reset ambiguous creation protection");
        client.FailCreate = false; model.AcknowledgeDraftCreation();
        Check(await model.EnsureDraftSessionAsync() == "new" && client.Creates == 2 && client.Sends == 0,
            "only an explicit acknowledgment enables another creation attempt");

        client = new Fake { Reads = [Fixture(path, session: "new") with { Items = [] }] };
        model = new WorkspaceConversationModel(client, client);
        await model.OpenProjectDraftAsync("project-a"); model.Draft = "original project";
        hold = client.CreateHold = new(); creating = model.EnsureDraftSessionAsync();
        await model.OpenProjectDraftAsync("project-b"); model.Draft = "current project";
        hold.SetResult("new");
        Check(await creating == null && model.CurrentProjectId == "project-b" && model.Draft == "current project" && model.Snapshot == null,
            "late creation cannot replace another project's editor");
        await model.OpenProjectDraftAsync("project-a");
        Check(await model.EnsureDraftSessionAsync() == "new" && client.Creates == 1 && model.Draft == "original project",
            "returning reuses the acknowledged late session instead of creating a duplicate");

        client = new Fake { FailRead = true };
        model = new WorkspaceConversationModel(client, client);
        await model.OpenProjectDraftAsync("project-a"); model.Draft = "keep after read failure";
        Check(await model.EnsureDraftSessionAsync() == null && client.Creates == 1 && model.Draft == "keep after read failure",
            "failure to open a confirmed session retains the first draft");
        await model.OpenProjectDraftAsync("project-b"); await model.OpenProjectDraftAsync("project-a");
        client.FailRead = false; client.Reads = [Fixture(path, session: "new") with { Items = [] }];
        Check(await model.EnsureDraftSessionAsync() == "new" && client.Creates == 1,
            "retrying the read does not repeat a confirmed create");
        await model.OpenProjectDraftAsync("project-b");
        using var cancelled = new CancellationTokenSource(); cancelled.Cancel();
        Check(await model.EnsureDraftSessionAsync(cancelled.Token) == null && client.Creates == 1,
            "a cancelled first action does not start a host write");
    }

    private static async Task ModelSelectionAsync(string path)
    {
        var client = new Fake { Reads = [Fixture(path)] };
        var model = new WorkspaceConversationModel(client, client);
        await model.OpenAsync("project-a", "session-a"); model.Draft = "keep this draft";
        Check(await model.SelectModelAsync("next") && client.ModelWrites == 1 && model.Draft == "keep this draft" && client.Creates == 0,
            "confirmed HTTP switch retains draft and conversation; only accepted writes may remember consent");
        client.FailModel = true;
        Check(!await model.SelectModelAsync("failed") && client.ModelWrites == 2 && model.OperationError != null,
            "failed model switch does not allow suppressing future confirmation");
        using var cancelled = new CancellationTokenSource(); cancelled.Cancel();
        Check(!await model.SelectModelAsync("cancelled", cancelled.Token) && client.ModelWrites == 2, "cancelled switch does not write");
        client.FailModel = false;
        var hold = client.ModelHold = new(); var pending = model.SelectModelAsync("late");
        client.Reads = [Fixture(path, session: "session-b")]; await model.OpenAsync("project-a", "session-b");
        hold.SetResult(); Check(!await pending, "a reply to a previous conversation must not remember the old checkbox");
        client.ModelHold = null;
        foreach (var snapshot in new[] { Fixture(path) with { Running = true }, Fixture(path) with { ReadOnly = true },
            Fixture(path) with { AcpAgentId = "local", ModelId = "acp:local" } })
        {
            client.Reads = [snapshot]; await model.OpenAsync("project-a", "session-a");
            Check(!await model.SelectModelAsync("blocked") && client.ModelWrites == 3, "running/read-only/ACP sessions cannot switch HTTP model");
        }
    }

    private static ConversationSnapshot Fixture(string path, string session = "session-a", ulong sequence = 7,
        string epoch = "host-one", bool running = false, string? requestId = null, string? error = null)
    {
        var node = JsonNode.Parse(File.ReadAllText(path))!;
        node["session_id"] = session;
        node["sequence"] = sequence;
        node["epoch"] = epoch;
        node["running"] = running;
        node["request_id"] = requestId;
        node["error"] = error;
        node["approvals"] = new JsonArray();
        return ConversationSnapshot.Decode(node, "project-a", session);
    }

    private sealed class Fake : INativeConversationClient, INativeSettingsClient
    {
        public List<ConversationSnapshot> Reads = [];
        public bool FailSend, FailRead;
        public int Seen, Sends, Creates, Stops;
        public TaskCompletionSource? StopHold;
        public bool FailCreate;
        public TaskCompletionSource<string>? CreateHold;
        public string? LastMessage;
        public int PlanWrites;
        public int FastWrites;
        public string? FastModel;
        public TaskCompletionSource? FastHold;
        public int ModelWrites;
        public bool FailModel;
        public TaskCompletionSource? ModelHold;
        public bool FailPlan;
        public TaskCompletionSource? PlanHold;
        public string? LastCommand, LastSession, LastApproval, LastRequestId;
        public bool? LastApproved;
        public TaskCompletionSource<ConversationSnapshot>? Hold;
        public Task<ConversationSnapshot> SnapshotAsync(string projectId, string sessionId, long? beforeSeq = null, CancellationToken cancellationToken = default)
        {
            if (Hold is { } hold) { var pending = hold.Task; Hold = null; return pending; }
            if (FailRead) throw new IOException("offline");
            if (Reads.Count > 1) { var next = Reads[0]; Reads.RemoveAt(0); return Task.FromResult(next); }
            return Task.FromResult(Reads[0]);
        }
        public Task MarkSeenAsync(string projectId, string sessionId, CancellationToken cancellationToken = default)
        { Seen++; return Task.CompletedTask; }
        public Task SendAsync(string projectId, string sessionId, Guid requestId, string message, CancellationToken cancellationToken = default)
        {
            Sends++; LastCommand = "send"; LastSession = sessionId; LastRequestId = requestId.ToString(); LastMessage = message;
            if (FailSend) throw new IOException("response lost");
            return Task.CompletedTask;
        }
        public Task StopAsync(string projectId, string sessionId, CancellationToken cancellationToken = default)
        { Stops++; LastCommand = "stop"; LastSession = sessionId; return StopHold?.Task ?? Task.CompletedTask; }
        public Task ApproveAsync(string projectId, string sessionId, string approvalId, bool approved, CancellationToken cancellationToken = default)
        { LastCommand = "approve"; LastSession = sessionId; LastApproval = approvalId; LastApproved = approved; return Task.CompletedTask; }
        public Task SetModelAsync(string projectId, string sessionId, string modelId, CancellationToken cancellationToken = default)
        { ModelWrites++; if (FailModel) throw new IOException("Model response lost"); return ModelHold?.Task ?? Task.CompletedTask; }
        public Task SetPlanModeAsync(string projectId, string sessionId, bool enabled, CancellationToken cancellationToken = default)
        { PlanWrites++; if (FailPlan) throw new IOException("Plan response lost"); return PlanHold?.Task ?? Task.CompletedTask; }
        public Task SetFastModeAsync(string projectId, string sessionId, string modelId, bool enabled, CancellationToken cancellationToken = default)
        { FastWrites++; FastModel = modelId; return FastHold?.Task ?? Task.CompletedTask; }
        public Task<string> CreateAsync(string projectId, CancellationToken cancellationToken = default)
        {
            Creates++;
            if (FailCreate) throw new IOException("creation response lost");
            if (CreateHold is { } hold) { CreateHold = null; return hold.Task; }
            return Task.FromResult("new");
        }
        public Task<NativeInboxEntry[]> InboxAsync(string projectId, CancellationToken cancellationToken = default) => Task.FromResult(Array.Empty<NativeInboxEntry>());
        public Task<NativeTrajectory> TrajectoryAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) => throw new NotSupportedException();
        public Task<string> TrajectoryHtmlAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) => Task.FromResult("");
        public Task<ConversationOutlineEntry[]> OutlineAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) => Task.FromResult(Array.Empty<ConversationOutlineEntry>());
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default) =>
            Task.FromResult<JsonNode?>(command == "list_models" ? new JsonArray() : null);
    }
}
