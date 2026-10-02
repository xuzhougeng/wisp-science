using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class WorkspaceActionTests
{
    public static async Task RunAsync()
    {
        var draft = NativeShareDraft.From([
            new("user", "样本 A"),
            new("reasoning", "内部思考"),
            new("assistant", "结论：通过"),
            new("tool", "should hide"),
            new("assistant", "   ")
        ]);
        Check(draft.Select(row => (row.Row.Role, row.Selected)).SequenceEqual([
            ("user", true), ("reasoning", false), ("assistant", true)
        ]), "share draft keeps editable roles and excludes thinking by default");
        Check(NativeShareDraft.Selected(draft, "样本,通过").Select(row => row.Text).SequenceEqual(["xxx A", "结论：xxx"]),
            "share redaction applies longest keywords to selected rows only");
        Check(NativeShareDraft.Width(" 3200 ") == 2400 && NativeShareDraft.Width("bad") == 840, "PNG width is clamped");

        var conversation = new FakeConversation();
        conversation.Outline =
        [
            new(0, "检查样本", 8, 1000, 2000),
            new(1, "检查样本", null, 3000, null)
        ];
        conversation.Snapshot = Snapshot(8, 0, new ConversationItem("user", "检查样本", null, null, null, null),
            new ConversationItem("assistant", "完成", null, null, null, null));
        var outline = new WorkspaceOutlineModel(conversation, "project-a", "session-a");
        await outline.RefreshAsync();
        Check(outline.Entries.Length == 2, "outline loads persisted questions");
        outline.Query = "不存在";
        Check(!outline.Visible.Any(), "outline search filters questions");
        outline.Query = "";
        await outline.OpenQuestionAsync(outline.Entries[0]);
        Check(outline.HistoryItemIndex == 0 && outline.History?.Items[0].Text == "检查样本", "outline history uses user_offset");
        conversation.Snapshot = Snapshot(8, null, new ConversationItem("user", "检查样本", null, null, null, null));
        await outline.OpenQuestionAsync(outline.Entries[0]);
        Check(outline.Error != null && outline.Error.Contains("刷新大纲"), "missing user_offset does not guess a repeated prompt");

        conversation.Inbox =
        [
            new("session-a", "project-a", "研究 A", "等待审阅", 1, 2, "needs_you"),
            new("session-b", "project-b", "研究 B", "已完成", 1, 2, "done")
        ];
        var inbox = new WorkspaceInboxModel(conversation);
        conversation.InboxReady = new();
        var pending = inbox.RefreshAsync("project-a");
        inbox.Reset();
        conversation.InboxReady.SetResult(conversation.Inbox);
        await pending;
        Check(inbox.Entries.Length == 0, "late inbox cannot reopen a reset view");
        conversation.InboxReady = null;
        await inbox.RefreshAsync("project-a");
        Check(inbox.Entries.Single().Id == "session-a", "inbox keeps only needs-you rows");
        await inbox.MarkOpenedAsync(inbox.Entries[0]);
        Check(conversation.Seen == ("project-a", "session-a"), "opened inbox entry is marked seen");

        conversation.Trajectory = NativeTrajectory.Decode(JsonNode.Parse("""
            {"frame_id":"session-a","model":"m","turns":[{"index":1,"started_at":1,"cells":[
              {"kind":"user","summary":"问","detail_input":null,"detail_output":null,"ok":null,"is_error":false,"ts":1,"duration_ms":10,"usage":null}
            ]}],"stats":{"turns":1,"steps":1,"llm_ms":1,"tool_ms":0,"input_tokens":1,"output_tokens":1,"cached_input_tokens":0,"cache_hit_pct":null,"tokens_per_sec":null}}
            """), "session-a");
        var trajectory = new WorkspaceTrajectoryModel(conversation, "project-a", "session-a");
        await trajectory.RefreshAsync();
        Check(trajectory.Rows.Length == 1 && trajectory.Segments.Single().Lane == "input", "trajectory layout uses shared projection");
        var firstTrajectory = trajectory.Snapshot;
        conversation.TrajectoryReady = new();
        var stale = trajectory.RefreshAsync();
        trajectory.Close();
        conversation.TrajectoryReady.SetResult(conversation.Trajectory);
        await stale;
        Check(trajectory.Snapshot == firstTrajectory && !trajectory.Loading, "closed trajectory ignores in-flight refresh after generation bump");

        var archiveClient = new FakeArchive();
        archiveClient.Current = Archive(null);
        var archive = new WorkspaceArchiveModel(archiveClient, "project-a", "session-a");
        await archive.LoadAsync();
        Check(archive.Archive != null && !archive.CanConfirm, "archive requires explicit consent");
        archive.Accepted = true;
        archiveClient.Fail = true;
        await archive.ConfirmAsync();
        Check(archiveClient.Confirms == 1 && archive.Archive?.FrozenAt is null && archive.Error != null,
            "failed archive confirm is not retried and rereads the saved draft");
        archiveClient.Fail = false;
        archive.Accepted = true;
        archiveClient.Current = Archive(9);
        await archive.ConfirmAsync();
        Check(archive.Frozen && archiveClient.Confirms == 2, "successful confirm freezes from server snapshot");
        archiveClient.Fail = true;
        var continued = await archive.ContinueAsync();
        Check(continued is null && archiveClient.Continues == 1, "uncertain continue is not replayed");

        var shareClient = new FakeShare();
        var share = new WorkspaceShareModel(shareClient, "project-a", "session-a");
        await share.LoadAsync();
        Check(share.Selected.Length == 1 && share.Selected[0].Role == "user", "share excludes thinking until selected");
        share.SelectAll(true);
        Check(share.Selected.Length == 2, "share select-all includes reasoning");
        shareClient.Fail = true;
        try { await share.HtmlAsync(false); throw new Exception("expected html failure"); }
        catch (IOException) { }
        Check(shareClient.HtmlCalls == 1, "share export is not retried automatically");

        var panelClient = new FakePanel();
        var panel = new WorkspacePanelModel(panelClient, "project-a", "session-a", new NativePanelTabs(available: NativePanelTabs.All));
        await panel.RefreshAsync("files");
        Check(panel.Files.Length == 2 && panel.Parent == ".", "file listing starts at workspace root");
        Check(WorkspacePanelModel.Child("data", "a") == "data/a", "file actions keep displayed directory");
        panelClient.Fail = true;
        try { await panel.PerformFileActionAsync(NativePanelFileAction.Delete, "README.md"); throw new Exception("expected delete failure"); }
        catch (IOException) { }
        Check(panelClient.Actions == 1, "uncertain file delete is not replayed");

        var firstDirectory = panelClient.PendingFiles = new();
        var firstListing = panel.RefreshAsync("files", "old");
        var secondDirectory = panelClient.PendingFiles = new();
        var secondListing = panel.RefreshAsync("files", "new");
        secondDirectory.SetResult([new("new.txt", false, 1, null)]);
        await secondListing;
        firstDirectory.SetResult([new("old.txt", false, 1, null)]);
        await firstListing;
        Check(panel.Path == "new" && panel.Files.Single().Name == "new.txt" && !panel.Loading,
            "late directory listing cannot replace the current path and files");

        var abandonedDirectory = panelClient.PendingFiles = new();
        var abandonedListing = panel.RefreshAsync("files", "abandoned");
        await panel.RefreshAsync("provenance");
        abandonedDirectory.SetResult([new("abandoned.txt", false, 1, null)]);
        await abandonedListing;
        Check(panel.Tabs.Selected == "provenance" && panel.Path == "new" && panel.Files.Single().Name == "new.txt",
            "switching tabs preserves the last accepted directory against a late response");

        var abandonedContexts = panelClient.PendingContexts = new();
        var hostRead = panel.RefreshAsync("hosts");
        panel.Close();
        abandonedContexts.SetResult(new([], [], true));
        await hostRead;
        Check(panel.Contexts is null && !panel.Loading, "closing a panel rejects late context results");
        panelClient.PendingFiles = null;
        panelClient.PendingContexts = null;

        var contextsClient = new FakePanel
        {
            ContextsValue = new([
                new("local", "local", "本机", "{}", "{}", null, null),
                new("remote-a", "ssh", "Remote", "{}", "{}", null, null)
            ], [], false, new("local"))
        };
        var contextsPanel = new WorkspacePanelModel(contextsClient, "project-a", "session-a", new NativePanelTabs(available: NativePanelTabs.All));
        await contextsPanel.RefreshAsync("hosts");
        await contextsPanel.SetDefaultContextAsync("remote-a");
        Check(contextsPanel.Contexts?.DefaultContext?.ContextId == "remote-a" && !contextsPanel.ContextBusy
            && contextsPanel.Contexts.EnabledIds.Contains("remote-a"), "default remote is confirmed and read back with enabled state without leaving Busy set");
        await contextsPanel.SetDefaultContextAsync("remote-a");
        Check(contextsClient.DefaultWrites == 1, "selecting the existing default does not write");
        contextsClient.Fail = true;
        await contextsPanel.SetDefaultContextAsync("local");
        Check(contextsPanel.ContextUncertain && !contextsPanel.ContextBusy && !contextsPanel.CanChangeContext,
            "lost environment response exposes uncertainty and releases busy state");
        await contextsPanel.SetDefaultContextAsync("local");
        await contextsPanel.SetContextEnabledAsync("remote-a", false);
        Check(contextsClient.DefaultWrites == 2, "uncertain default environment is not replayed");
        contextsClient.Fail = false;
        await contextsPanel.RefreshAsync("hosts");
        Check(!contextsPanel.ContextUncertain && contextsPanel.Contexts?.DefaultContext?.ContextId == "local",
            "read-only refresh resolves a write that reached the host before its response was lost");
        contextsClient.ContextsValue = contextsClient.ContextsValue with { DefaultContext = null };
        await contextsPanel.RefreshAsync("hosts"); await contextsPanel.SetDefaultContextAsync("remote-a");
        Check(contextsClient.DefaultWrites == 2, "old hosts without default capability cannot receive new writes");
        contextsClient.ContextsValue = contextsClient.ContextsValue with { DefaultContext = new("local"), ReadOnly = true };
        await contextsPanel.RefreshAsync("hosts"); await contextsPanel.SetDefaultContextAsync("remote-a");
        Check(contextsClient.DefaultWrites == 2, "read-only environment panels cannot set a default");
        contextsClient.ContextsValue = contextsClient.ContextsValue with { ReadOnly = false };
        await contextsPanel.RefreshAsync("hosts");
        var beforeWrite = contextsPanel.Contexts!;
        var pendingDefault = contextsClient.PendingDefault = new();
        var defaultWrite = contextsPanel.SetDefaultContextAsync("remote-a");
        var duringWrite = contextsClient.PendingContexts = new();
        var staleContextsRead = contextsPanel.RefreshAsync("hosts");
        pendingDefault.SetException(new IOException("response lost"));
        await defaultWrite;
        duringWrite.SetResult(beforeWrite);
        await staleContextsRead;
        Check(contextsPanel.ContextUncertain && !contextsPanel.CanChangeContext && !contextsPanel.ContextBusy,
            "a read begun during the write cannot clear uncertainty after the response is lost");
        contextsClient.PendingContexts = null; contextsClient.PendingDefault = null;
        await contextsPanel.RefreshAsync("hosts");
        Check(!contextsPanel.ContextUncertain && contextsPanel.Contexts?.DefaultContext?.ContextId == "remote-a",
            "only a fresh post-write read resolves the environment state");
        pendingDefault = contextsClient.PendingDefault = new();
        defaultWrite = contextsPanel.SetDefaultContextAsync("local");
        await contextsPanel.RefreshAsync("provenance");
        pendingDefault.SetResult(); await defaultWrite;
        Check(contextsPanel.Tabs.Selected == "provenance" && !contextsPanel.ContextBusy && contextsPanel.ContextUncertain,
            "late successful environment writes do not reopen their tab or trust stale displayed data");
        contextsClient.PendingDefault = null;
        await contextsPanel.RefreshAsync("hosts");
        Check(contextsPanel.Contexts?.DefaultContext?.ContextId == "local" && contextsPanel.CanChangeContext,
            "returning to environments restores the confirmed default and enabled actions");

        panelClient.PendingPreview = new();
        var previewRead = panel.ReadFileAsync("old.pdf");
        panel.DismissPreview();
        panelClient.PendingPreview.SetResult(new("old.pdf", "application/pdf", null, "JVBERg==", false, 4));
        await previewRead;
        Check(panel.Preview == null, "late PDF read cannot reopen a dismissed preview");
        var oldPreview = panelClient.PendingPreview = new();
        var oldRead = panel.ReadFileAsync("old.pdf");
        var newPreview = panelClient.PendingPreview = new();
        var newRead = panel.ReadArtifactAsync("new-artifact");
        newPreview.SetResult(new("new.pdf", "application/pdf", null, "JVBERg==", false, 4)); await newRead;
        oldPreview.SetResult(new("old.pdf", "application/pdf", null, "JVBERg==", false, 4)); await oldRead;
        Check(panel.Preview?.Path == "new.pdf", "older file read cannot replace a newer artifact preview");

        var terminalClient = new FakeTerminal();
        var terminal = new WorkspaceTerminalModel(terminalClient, "project-a", "session-a");
        await terminal.LoadAsync();
        Check(terminal.SelectedId == "terminal-a", "terminal selects the first existing session");
        await terminal.ReadAsync();
        Check(terminal.Output == "hello", "terminal output uses the shared cursor contract");
        terminalClient.Fail = true;
        await terminal.WriteAsync("x"u8.ToArray());
        Check(terminal.InputUncertain && terminalClient.Writes == 1, "uncertain terminal input is not resent");
        await terminal.WriteAsync("y"u8.ToArray());
        Check(terminalClient.Writes == 1, "blocked terminal input stays blocked until the user resumes");
        terminal.ResumeInput();
        terminalClient.Fail = false;
        await terminal.OpenAsync("local");
        terminalClient.Fail = true;
        await terminal.OpenAsync("local");
        Check(terminalClient.Opens == 2 && terminal.Error != null && terminal.Error.Contains("刷新列表"),
            "failed terminal open reconciles the list and does not retry");
        Console.WriteLine("WinUI workspace action models passed.");
    }

    private static void Check(bool passed, string name)
    {
        if (!passed) throw new InvalidOperationException(name);
        Console.WriteLine("PASS workspace: " + name);
    }

    private static ConversationSnapshot Snapshot(ulong sequence, int? userOffset, params ConversationItem[] items) =>
        new(ConversationSnapshot.SchemaId, "host-one", sequence, "project-a", "session-a", items, null, false, false, false, "model-a", null, null, [], userOffset);

    private static NativeResearchArchive Archive(long? frozen) => new("archive-a", "project-a", "session-a", "hash",
        "节点", "报告", [], [new("results/qc.txt", "abc", 12, "keep", false, "", null, "pending")], 1, frozen, []);

    private sealed class FakeConversation : INativeConversationClient
    {
        public ConversationOutlineEntry[] Outline = [];
        public NativeInboxEntry[] Inbox = [];
        public ConversationSnapshot? Snapshot;
        public NativeTrajectory? Trajectory;
        public (string Project, string Session)? Seen;
        public TaskCompletionSource<NativeInboxEntry[]>? InboxReady;
        public TaskCompletionSource<NativeTrajectory>? TrajectoryReady;
        public Task<NativeInboxEntry[]> InboxAsync(string projectId, CancellationToken cancellationToken = default) =>
            InboxReady?.Task ?? Task.FromResult(Inbox);
        public Task MarkSeenAsync(string projectId, string sessionId, CancellationToken cancellationToken = default)
        { Seen = (projectId, sessionId); return Task.CompletedTask; }
        public Task<NativeTrajectory> TrajectoryAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) =>
            TrajectoryReady?.Task ?? Task.FromResult(Trajectory ?? throw new InvalidDataException("missing trajectory"));
        public Task<string> TrajectoryHtmlAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) =>
            Task.FromResult("<html/>");
        public Task<ConversationOutlineEntry[]> OutlineAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) =>
            Task.FromResult(Outline);
        public Task<string> CreateAsync(string projectId, CancellationToken cancellationToken = default) => Task.FromResult("s");
        public Task<ConversationSnapshot> SnapshotAsync(string projectId, string sessionId, long? beforeSeq = null, CancellationToken cancellationToken = default) =>
            Task.FromResult(Snapshot ?? throw new InvalidDataException("missing snapshot"));
        public Task SendAsync(string projectId, string sessionId, Guid requestId, string message, CancellationToken cancellationToken = default) => Task.CompletedTask;
        public Task StopAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) => Task.CompletedTask;
        public Task ApproveAsync(string projectId, string sessionId, string approvalId, bool approved, CancellationToken cancellationToken = default) => Task.CompletedTask;
        public Task SetModelAsync(string projectId, string sessionId, string modelId, CancellationToken cancellationToken = default) => Task.CompletedTask;
    }

    private sealed class FakeArchive : INativeArchiveClient
    {
        public NativeResearchArchive? Current;
        public bool Fail;
        public int Confirms, Continues;
        public Task<NativeResearchArchive?> GetAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) =>
            Task.FromResult(Current);
        public Task<NativeResearchArchive?> PrepareAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) =>
            Task.FromResult(Current);
        public Task<NativeResearchArchive?> ConfirmAsync(string projectId, string sessionId, NativeArchiveConfirmation input, CancellationToken cancellationToken = default)
        {
            Confirms++;
            if (Fail) throw new IOException("lost confirm");
            Current = Current! with { FrozenAt = 9 };
            return Task.FromResult<NativeResearchArchive?>(Current);
        }
        public Task<NativeResearchArchive?> RetryCleanupAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) =>
            Task.FromResult(Current);
        public Task<string> ContinueAsync(string projectId, string sessionId, CancellationToken cancellationToken = default)
        {
            Continues++;
            if (Fail) throw new IOException("lost continue");
            return Task.FromResult("session-next");
        }
    }

    private sealed class FakeShare : INativeShareClient
    {
        public bool Fail;
        public int HtmlCalls;
        public Task<NativeShareRow[]> ReadAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) =>
            Task.FromResult(new[] { new NativeShareRow("user", "样本"), new NativeShareRow("reasoning", "思考") });
        public Task<string> HtmlAsync(string projectId, string sessionId, NativeShareRow[] rows, bool dark = false, CancellationToken cancellationToken = default)
        {
            HtmlCalls++;
            if (Fail) throw new IOException("lost export");
            return Task.FromResult("<html>" + rows.Length + "</html>");
        }
    }

    private sealed class FakePanel : INativePanelClient
    {
        public TaskCompletionSource<NativePanelFileContent>? PendingPreview;
        public TaskCompletionSource<NativePanelFile[]>? PendingFiles;
        public TaskCompletionSource<NativePanelContexts>? PendingContexts;
        public NativePanelContexts ContextsValue = new([], [], true);
        public int DefaultWrites;
        public TaskCompletionSource? PendingDefault;
        public bool Fail;
        public int Actions;
        public Task<NativePanelContexts> ContextsAsync(string project, string session, CancellationToken token = default) =>
            PendingContexts?.Task ?? Task.FromResult(ContextsValue);
        public async Task SetDefaultContextAsync(string project, string session, string contextId, CancellationToken token = default)
        {
            DefaultWrites++;
            ContextsValue = ContextsValue with { DefaultContext = new(contextId), EnabledIds = contextId == "local" ? ContextsValue.EnabledIds : [contextId] };
            if (PendingDefault is { } pending) await pending.Task;
            if (Fail) throw new IOException("lost default response");
        }
        public Task<string[]> SetContextEnabledAsync(string project, string session, string contextId, bool enabled, CancellationToken token = default) =>
            Task.FromResult(Array.Empty<string>());
        public Task ProbeContextAsync(string project, string contextId, CancellationToken token = default) => Task.CompletedTask;
        public Task<NativePanelArtifact[]> ArtifactsAsync(string project, string session, CancellationToken token = default) =>
            Task.FromResult(Array.Empty<NativePanelArtifact>());
        public Task<NativePanelFile[]> FilesAsync(string project, string session, string path = ".", CancellationToken token = default) =>
            PendingFiles?.Task ?? Task.FromResult(new[] { new NativePanelFile("data", true, 0, null), new NativePanelFile("README.md", false, 12, null) });
        public Task SaveFileAsync(string project, string session, string path, string originalText, string text, CancellationToken token = default) => Task.CompletedTask;
        public Task FileActionAsync(string project, string session, NativePanelFileAction action, string path, string? newPath = null, CancellationToken token = default)
        {
            Actions++;
            if (Fail) throw new IOException("lost action");
            return Task.CompletedTask;
        }
        public Task<NativePanelFileContent> ReadFileAsync(string project, string session, string path, CancellationToken token = default) =>
            PendingPreview?.Task ?? Task.FromResult(new NativePanelFileContent(path, "text/plain", "ok", null, false, 2));
        public Task<NativePanelFileContent> ReadArtifactAsync(string project, string session, string artifactId, CancellationToken token = default) =>
            PendingPreview?.Task ?? Task.FromResult(new NativePanelFileContent(artifactId, "text/plain", "ok", null, false, 2));
    }

    private sealed class FakeTerminal : INativeTerminalClient
    {
        public bool Fail;
        public int Writes, Opens;
        public Task<NativeTerminalInfo[]> ListAsync(string project, string session, CancellationToken token = default) =>
            Task.FromResult(new[] { new NativeTerminalInfo("terminal-a", project, "local", "本地", "pty", ".", true) });
        public Task<NativeTerminalInfo> OpenAsync(string project, string session, string context, CancellationToken token = default)
        {
            Opens++;
            if (Fail) throw new IOException("lost open");
            return Task.FromResult(new NativeTerminalInfo("terminal-b", project, context, "新建", "pty", ".", true));
        }
        public Task<NativeTerminalOutput> ReadAsync(string project, string session, string terminal, ulong? cursor, CancellationToken token = default) =>
            Task.FromResult(new NativeTerminalOutput(terminal, 0, 5, Convert.ToBase64String("hello"u8.ToArray()), true, null));
        public Task WriteAsync(string project, string session, string terminal, byte[] bytes, CancellationToken token = default)
        {
            Writes++;
            if (Fail) throw new IOException("lost write");
            return Task.CompletedTask;
        }
        public Task ResizeAsync(string project, string session, string terminal, ushort rows, ushort cols, CancellationToken token = default) => Task.CompletedTask;
        public Task CloseAsync(string project, string session, string terminal, CancellationToken token = default) => Task.CompletedTask;
    }
}
