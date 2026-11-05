using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class ProjectChatHistoryTests
{
    public static async Task RunAsync(string fixture)
    {
        var page = ProjectBrowserClient.Decode(File.ReadAllText(fixture), "history", "project_chat_matches").Page!;
        var hit = page.Items.Single();
        Check(hit.Before == "…<script>" && hit.Matched == "100%_" && page.NextCursor == new ProjectChatCursor(100, "shelved", 42),
            "the shared response preserves literal matches and the full paging key");
        var service = new DeferredService();
        using var search = new ProjectChatHistoryModel(service, "review.sqlite", "research-1");
        var old = search.EditAsync("old", TimeSpan.Zero);
        var current = search.EditAsync("100%_", TimeSpan.Zero);
        service.Calls[1].Result.SetResult(page); await current;
        service.Calls[0].Result.SetResult(new([], null)); await old;
        Check(search.Items.Single() == hit && service.Calls[0].Token.IsCancellationRequested, "late results cannot replace the new query");

        var more = search.ContinueAsync(); await search.ContinueAsync();
        Check(service.Calls.Count == 3, "a second click does not request the same page twice");
        service.Calls[2].Result.SetException(new IOException("disconnected")); await more;
        Check(search.Error != null && search.Items.Single() == hit && search.Next == page.NextCursor, "pagination failure preserves the page and cursor");
        var retry = search.ContinueAsync();
        Check(service.Calls[3].Cursor == page.NextCursor && service.Calls[3].Query == "100%_", "retry repeats the same page");
        service.Calls[3].Result.SetResult(new([hit with { MessageSeq = 41 }], null)); await retry;
        Check(search.Error == null && search.Next == null && search.Items.Select(h => h.MessageSeq).SequenceEqual(new long[] { 42, 41 }),
            "retry appends once in server order");

        var cleared = search.EditAsync("gone", TimeSpan.Zero); await search.EditAsync(" ", TimeSpan.Zero);
        service.Calls[4].Result.SetResult(page); await cleared;
        Check(search.Items.Count == 0 && !search.Loading, "clearing input invalidates the previous request");
        var closing = search.EditAsync("closing", TimeSpan.Zero); search.Dispose();
        service.Calls[5].Result.SetResult(page); await closing;
        Check(service.Calls[5].Token.IsCancellationRequested && search.Items.Count == 0, "a closed dialog ignores late replies");
    }

    private sealed class DeferredService : IProjectChatHistory
    {
        public List<(string Query, ProjectChatCursor? Cursor, CancellationToken Token, TaskCompletionSource<ProjectChatMatches> Result)> Calls = [];
        public Task<ProjectChatMatches> SearchChatsAsync(string database, string project, string query, ProjectChatCursor? cursor, CancellationToken token)
        {
            Check(database == "review.sqlite" && project == "research-1", "every page stays in its original database and project");
            var result = new TaskCompletionSource<ProjectChatMatches>(TaskCreationOptions.RunContinuationsAsynchronously);
            Calls.Add((query, cursor, token, result)); return result.Task;
        }
        public Task<ProjectChatLocation?> LocateChatAsync(string database, string project, ProjectChatHit hit, CancellationToken token) =>
            throw new NotSupportedException();
    }
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
}
