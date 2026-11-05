using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeUsageTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var days = Enumerable.Range(0, 14).Select(i => new NativeUsageDay($"2026-07-{i + 1:00}", i == 6 ? 10 : i == 13 ? 30 : 0, false)).ToArray();
        var daily = NativeUsagePresentation.Activity(days, NativeUsageActivityMode.Daily);
        Check(daily.Length == 14 && daily[6].Tokens == 10 && daily[13].Level > daily[6].Level, "daily activity retains backend day values and relative intensity");
        var weekly = NativeUsagePresentation.Activity(days, NativeUsageActivityMode.Weekly);
        Check(weekly.Take(7).Count(cell => cell.Level > 0) == 3 && weekly.Skip(7).Count(cell => cell.Level > 0) == 7, "weekly activity matches WebView bottom-filled week columns");
        var cumulative = NativeUsagePresentation.Activity(days, NativeUsageActivityMode.Cumulative);
        Check(cumulative[0].Tokens == 10 && cumulative[7].Tokens == 40 && cumulative[7].Period == "2026-07-14", "cumulative activity sums weeks and labels the current week end");
        days[13] = days[13] with { Future = true, Tokens = 999 };
        Check(NativeUsagePresentation.Activity(days, NativeUsageActivityMode.Daily)[13].Level == 0
            && NativeUsagePresentation.Activity(days, NativeUsageActivityMode.Weekly)[7].Tokens == 0, "future values never inflate the current activity scale or weekly totals");
        Check(NativeUsagePresentation.Activity([], NativeUsageActivityMode.Cumulative).Length == 0
            && NativeUsagePresentation.Months(days).SequenceEqual(new[] { (0, 7) }), "empty activity and repeated month labels remain well-defined");
        var overview = JsonNode.Parse("""
            {"models":[{"model":"a","tokens":5},{"model":"b","tokens":7},{"model":"unknown","tokens":2},{"model":"unused","tokens":-1}],
            "tools":[{"kind":"shell","name":"shell","calls":999},{"kind":"mcp","name":"search","calls":3},{"kind":"skill","name":"research","calls":5}],
            "workspaces":[{"project_id":"p","name":" ","workspace_dir":"D:\\p","updated_at":12,"session_count":2,"input":10,"output":20,"reasoning":30,"cached":40}]}
            """)!;
        var profiles = JsonNode.Parse("""[{"id":"a","model":"shared","label":"A"},{"id":"b","model":"shared","label":"B"}]""");
        var slices = NativeUsagePresentation.Models(overview, profiles, "Unknown", "Other");
        Check(slices.Length == 2 && slices[0].Label == "shared" && slices[0].Tokens == 12 && slices[1].Label == "Unknown", "profile IDs merge under their actual model label and nonpositive counts are omitted");
        overview["models"] = new JsonArray(Enumerable.Range(1, 9).Select(i => (JsonNode)new JsonObject { ["model"] = "model-" + i, ["tokens"] = i }).ToArray());
        slices = NativeUsagePresentation.Models(overview, null, "Unknown", "Other");
        Check(slices.Length == 8 && slices[^1].Label == "Other" && slices[^1].Tokens == 3, "model distribution preserves the top seven plus the long-tail total");
        var tools = NativeUsagePresentation.Tools(overview, "Other tools");
        Check(tools.Length == 2 && tools[0].Kind == "skill" && tools[0].Calls == 5, "tool ranks include only SKILL and MCP calls");
        foreach (var i in Enumerable.Range(1, 10)) overview["tools"]!.AsArray().Add(new JsonObject { ["kind"] = "mcp", ["name"] = "tool-" + i, ["calls"] = 1 });
        tools = NativeUsagePresentation.Tools(overview, "Other tools"); Check(tools.Length == 10 && tools[^1].Calls == 3 && tools[^1].Kind == "other", "tool rank caps its long tail without adding built-in shell counts");
        var workspace = NativeUsagePresentation.Workspaces(overview).Single();
        Check(workspace.DisplayName == "p" && NativeUsagePresentation.Totals([workspace]).Values.SequenceEqual(new long[] { 10, 20, 30, 40 }), "workspace fallback identity and all four token metrics are preserved");
        Check(NativeUsagePresentation.Tokens(-1) == "0" && NativeUsagePresentation.Tokens(999) == "999" && NativeUsagePresentation.Tokens(12300) == "12.3k"
            && NativeUsagePresentation.Sum([long.MaxValue, 1]) == long.MaxValue, "token presentation follows WebView scale without overflow");
        Check(NativeUsageLayout.OverviewColumns(1707, 992) && !NativeUsageLayout.OverviewColumns(1050, 824)
            && !NativeUsageLayout.OverviewColumns(1051, 814) && NativeUsageLayout.ModelBesideLegend(801)
            && !NativeUsageLayout.ModelBesideLegend(720) && NativeUsageLayout.SummaryColumns(350) == 3
            && NativeUsageLayout.SummaryColumns(double.NaN) == 1, "dashboard breakpoints preserve readable tables and summary tiles");

        var fake = new Fake(); using var pager = new NativeUsagePager(fake, "opened-context");
        var first = pager.SelectAsync(workspace); Check(pager.Loading && pager.Items == null && fake.Calls[0].Args["limit"]!.GetValue<int>() == 20, "selection loads a fresh first page of twenty sessions");
        fake.Replies[0].SetResult(Page(41, "first")); await first;
        Check(pager.PageCount == 3 && pager.Items!.Single().Id == "first" && fake.Calls[0].Context == "opened-context" && fake.Calls[0].Args["projectId"]!.GetValue<string>() == "p", "page request preserves explicit workspace argument and outer context without activation");
        var next = pager.GoAsync(2); Check(fake.Calls[1].Args["offset"]!.GetValue<long>() == 40 && pager.Items == null, "pagination replaces rather than appends session rows");
        fake.Replies[1].SetResult(Page(21, "obsolete"));
        await Until(() => fake.Calls.Count == 3); Check(fake.Calls[2].Args["offset"]!.GetValue<long>() == 20, "a shrinking result clamps to the last valid page");
        fake.Replies[2].SetResult(Page(21, "last")); await next; Check(pager.Page == 1 && pager.Items!.Single().Id == "last", "clamped page displays only its acknowledged rows");
        var stale = pager.SelectAsync(workspace with { Id = "old" }); var current = pager.SelectAsync(workspace with { Id = "new" });
        fake.Replies[4].SetResult(Page(1, "new")); await current; fake.Replies[3].SetResult(Page(1, "old")); await stale;
        Check(pager.Selected!.Id == "new" && pager.Items!.Single().Id == "new", "late workspace replies cannot overwrite the current selection");
        var abandoned = pager.RetryAsync(); pager.Back(); fake.Replies[5].SetResult(Page(1, "abandoned")); await abandoned;
        Check(pager.Selected == null && pager.Items == null && !pager.Loading, "Back invalidates a pending reply immediately");
        var failed = pager.SelectAsync(workspace); fake.Replies[6].SetException(new IOException("offline")); await failed;
        Check(pager.Error == "offline" && pager.Items == null, "failed reads expose an error, not an empty success");
        var retry = pager.RetryAsync(); fake.Replies[7].SetResult(Page(0, null)); await retry;
        Check(pager.Items!.Count == 0 && pager.Total == 0 && pager.Error == null && pager.Selected != null, "retry can show an empty workspace without dropping the detail context");
        var malformed = pager.RetryAsync(); fake.Replies[8].SetResult(new JsonObject { ["total"] = 1L }); await malformed;
        Check(pager.Error != null && pager.Items == null, "a missing items array is an error rather than a successful empty page");
        var disposed = pager.RetryAsync(); pager.Dispose(); fake.Replies[9].SetResult(Page(1, "late")); await disposed;
        Check(pager.Items == null && pager.Selected == null, "disposed pages ignore late reads");
        Check(fake.Calls.All(call => call.Command == "get_session_token_usage"), "usage browsing never activates a project or writes settings");
    }
    private static JsonObject Page(long total, string? id) => new() { ["total"] = total, ["items"] = id == null ? new JsonArray() : new JsonArray(new JsonObject { ["id"] = id, ["title"] = id, ["updated_at"] = 1, ["input"] = 2, ["output"] = 3, ["reasoning"] = 4, ["cached"] = 5 }) };
    private static async Task Until(Func<bool> ready) { for (var i = 0; i < 100 && !ready(); i++) await Task.Delay(1); Check(ready(), "expected page request"); }
    private sealed class Fake : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args, string? Context)> Calls { get; } = [];
        public List<TaskCompletionSource<JsonNode?>> Replies { get; } = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        { Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId)); var reply = new TaskCompletionSource<JsonNode?>(); Replies.Add(reply); return reply.Task; }
    }
}
