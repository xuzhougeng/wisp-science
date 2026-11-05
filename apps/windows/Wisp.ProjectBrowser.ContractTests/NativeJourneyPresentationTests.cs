using System.Text.Json;
using System.Text;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeJourneyPresentationTests
{
    public static async Task RunAsync()
    {
        await Previews();
        Check(!NativeJourneyLayout.ForWidth(1201).Stacked && NativeJourneyLayout.ForWidth(1201).AsideWidth == 342, "desktop source width follows WebView");
        Check(NativeJourneyLayout.ForWidth(1103).AsideWidth == 238 && NativeJourneyLayout.ForWidth(960).Stacked, "medium month/source sidebar and compact stacked ordering");
        Check(NativeJourneyLayout.ForWidth(640).Narrow && NativeJourneyLayout.ForWidth(640).DateWidth == 68, "narrow timeline date column");
        var longEdge = JourneyGraphRoute.Between(40, 64, 760, 64);
        Check(longEdge.Segments.Length == 3 && longEdge.Segments[1].IsLine && longEdge.Segments[1].End.Y > 122 && longEdge.Segments[1].End.Y < 156, "long edges use the gap between rows, leaving intermediate nodes readable");
        var reverseEdge = JourneyGraphRoute.Between(280, 64, 40, 64);
        Check(reverseEdge.Start.X == 280 && reverseEdge.Segments[^1].End.X == 216 && reverseEdge.Segments[^1].Control2.X > 216, "reverse relationship arrow points to the target's right edge");
        var verticalEdge = JourneyGraphRoute.Between(40, 64, 40, 156);
        Check(verticalEdge.Start.Y == 122 && verticalEdge.Segments[^1].End.Y == 156 && verticalEdge.Label.X > 216, "same-column relationships route around the node edge");
        var day = new JourneyDay(new DateOnly(2026, 10, 30), [new("run", "Run", 5, "run", SourceId: "r"), new("same-run", "Finished", 4, "run", SourceId: "r"), new("progress", "Progress", 3, "progress"), new("archive", "Archive", 2, "archive")]);
        Check(NativeJourneyPresentation.Headline(day).Id == "archive" && NativeJourneyPresentation.Experiments(day) == 1, "archive heading priority and unique run count");
        Check(NativeJourneyPresentation.RecapItems(" first\r\n\r\n second ").Select(i => i.Text).SequenceEqual(["first", "second"]), "recap edit shares one-item-per-line projection");
        var fake = new Fake(); using var model = new WorkspaceJourneyModel(fake, "project-a");
        var first = new TaskCompletionSource<JourneyPage>(); var second = new TaskCompletionSource<JourneyPage>();
        fake.Read = (_, _) => first.Task; var oldRead = model.OpenAsync(new DateTime(2026, 10, 30));
        fake.Read = (_, _) => second.Task; var newRead = model.ShiftMonthAsync(1);
        second.SetResult(new([new("new", "November", WorkspaceCalendarModel.Unix(new(2026, 11, 3)), "progress")], false)); await newRead;
        first.SetResult(new([new("old", "October", 0)], false)); await oldRead;
        Check(model.Month.Month == 11 && model.Page!.Entries.Single().Id == "new" && model.MarkedDays.Contains(new(2026, 11, 3)), "late month read cannot overwrite current range or calendar marks");
        fake.Read = (_, _) => Task.FromResult(new JourneyPage([], false)); await model.SelectDayAsync(new(2026, 11, 3));
        Check(model.FocusDay?.Day == 3 && model.MarkedDays.Contains(new(2026, 11, 3)), "day reads keep the month's activity marks");
        model.OpenNote(); model.NoteTitle = "Research finding"; model.NoteBody = "first\n第二行\n"; model.NoteCategory = "finding";
        fake.FailWrite = true; Check(!await model.SaveNoteAsync(), "uncertain write reports failure");
        Check(model.NoteOpen && model.NoteBody == "first\n第二行\n" && model.SaveError != null && fake.Writes == 1, "failed note keeps draft and does not retry");
        fake.FailWrite = false; Check(await model.SaveNoteAsync() && !model.NoteOpen && fake.Writes == 2, "explicit successful save closes only note and reloads its date");
        Check(fake.LastNote!.Category == "finding" && fake.LastNote.OccurredAt == WorkspaceCalendarModel.Unix(new(2026, 11, 3)), "note stores local research midnight separately from server record time");
        model.OpenNote(); model.NoteTitle = "Pending"; var write = new TaskCompletionSource<string>(); fake.Write = () => write.Task;
        var saving = model.SaveNoteAsync(); model.CloseNote(); write.SetResult("saved"); await saving;
        Check(!model.NoteOpen && !model.Saving, "closing a pending note never reopens it on reply");
        var graphReply = new TaskCompletionSource<JourneyGraph>(); fake.Graph = () => graphReply.Task; var readingGraph = model.LoadGraphAsync(); model.Dispose(); graphReply.SetResult(new([new("late", "decision", "Late reply", null, "{}")], [])); await readingGraph;
        Check(model.Graph is { Nodes.Length: 0 } && !model.GraphBusy, "closed page ignores graph reply");
        await Contracts();
    }
    private static async Task Contracts()
    {
        var transport = new Transport(); var client = new NativeJourneyClient(transport);
        transport.Reply = JsonNode.Parse("""{"version_id":"immutable-1","filename":"figure.svg","version_number":1,"source":{"run_id":null,"run_title":"","run_status":"","context_id":"local","generated_at":null,"inputs":[]},"text":null,"mime":"image/svg+xml","base64":"PHN2Zy8+","truncated":false,"content_error":null}""");
        await client.ThumbnailAsync("p", "immutable-1");
        Check(transport.Command == "native_research_journey_artifact" && transport.Project == "p" && transport.Args!["thumbnail"]!.GetValue<bool>() && transport.Args["version_id"]!.GetValue<string>() == "immutable-1", "thumbnail stays scoped to an immutable version");
        await client.ArtifactAsync("p", "immutable-1");
        Check(!transport.Args!.ContainsKey("thumbnail"), "detail reads retain the original wire shape");
        transport.Reply = JsonNode.Parse("""{"nodes":[{"id":"n1","kind":"decision","title":"Decision","ref_id":null,"metadata_json":"{\"evidence\":true}"}],"edges":[]} """);
        var graph = await client.GraphAsync("p");
        Check(transport.Command == "native_research_journey_graph" && transport.Project == "p" && transport.Args!.Count == 0 && graph.Nodes[0].MetadataJson == "{\"evidence\":true}", "graph uses explicit scope and raw metadata string");
        transport.Reply = JsonValue.Create("entry"); await client.AddAsync("p", new("Title", "Body", "finding", 42));
        Check(transport.Command == "native_research_journey_add" && transport.Project == "p" && transport.Args!["input"]!["occurred_at"]!.GetValue<long>() == 42 && transport.Args.Count == 1, "journal input follows existing Rust envelope");
        var recapJson = """{"id":"recap","day_start":42,"status":"draft","headline":"Day","done":[{"text":"Found evidence","refs":[0]}],"findings":[],"issues":[],"next":[],"sources":[{"kind":"record","id":"entry","title":"Title"}],"model":"side-model","generated_at":99} """;
        transport.Reply = JsonNode.Parse("{\"entries\":[],\"truncated\":false,\"recaps\":[" + recapJson + "]}");
        var page = await client.ReadAsync("p", 1, 50); var recap = page.Recaps.Single();
        Check(recap.Done.Single().Refs.Single() == 0 && recap.DayStart == 42 && recap.Sources.Single().Id == "entry", "journey keeps full recap citations and local day");
        transport.Reply = JsonNode.Parse(recapJson); await client.EditRecapAsync("p", new(recap.Id, "confirmed", recap.Headline, recap.Done, [], [], []));
        Check(transport.Command == "native_research_journey_recap" && transport.Args!["edit"]!["status"]!.GetValue<string>() == "confirmed" && transport.Args["edit"]!["done"]![0]!["refs"]![0]!.GetValue<int>() == 0, "confirmation retains citations and existing edit shape");
        var calls = transport.Calls; transport.Fail = true;
        try { await client.AddAsync("p", new("Title", "Body", "finding", 42)); throw new Exception("Expected write failure"); } catch (IOException) { }
        Check(transport.Calls == calls + 1, "lost journal write is never replayed");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private static async Task Previews()
    {
        Check(JourneyOutputPreview.Kind("FIGURE.SVG") == "image" && JourneyOutputPreview.Kind("counts.tsv") == "csv" && !JourneyOutputPreview.CanLoad("pdf"), "card kinds follow WebView's image/table/document preview choices");
        Check(new[] { "report.doc", "report.ODT", "sheet.xlsb", "slides.ppsx" }.All(path => JourneyOutputPreview.Kind(path) == "document" && !JourneyOutputPreview.CanLoad(JourneyOutputPreview.Kind(path))), "legacy office containers retain document placeholders instead of text reads");
        Check(new[] { "script.pyw", "app.tsx", "main.go", "query.sql", "config.yaml", "changes.patch", "records.jsonl" }.All(path => JourneyOutputPreview.Kind(path) == "code"), "code thumbnails use the same extension families as WebView");
        var text = string.Concat(Enumerable.Repeat("😀中", 400));
        Check(JourneyOutputPreview.Excerpt(text).EnumerateRunes().Count() == 600 && !JourneyOutputPreview.Excerpt(text).Contains('\uFFFD'), "600-character excerpt never cuts a Unicode scalar");
        var table = JourneyOutputPreview.Table("a\tb\tc\td\te\n" + string.Join('\n', Enumerable.Repeat("1\t2\t3\t4\t5", 10)));
        Check(table.Length == 7 && table.All(row => row.Length == 4) && table[1][3] == "4", "table card limits rows and columns without losing TSV boundaries");
        var markdown = JourneyOutputPreview.Markdown("# Heading\n\nplain text\n<script>literal</script>");
        Check(markdown.Length == 3 && markdown[0] == ("Heading", true) && markdown[2].Text == "<script>literal</script>", "markdown card preserves literal text without interpreting markup");
        var fake = new ThumbnailFake(); using var loader = new JourneyThumbnailLoader(fake, "p");
        var reads = Enumerable.Range(0, 4).Select(i => loader.LoadAsync("v" + i, CancellationToken.None)).ToArray();
        using var cancel = new CancellationTokenSource(); var queued = loader.LoadAsync("queued", cancel.Token);
        Check(fake.Requests.Count == 4 && fake.Projects.All(p => p == "p"), "only four visible thumbnail reads enter the scoped client concurrently");
        cancel.Cancel(); try { await queued; throw new Exception("Expected cancellation"); } catch (OperationCanceledException) { }
        loader.Dispose(); foreach (var (id, completion) in fake.Requests) completion.SetResult(new(id, "result.txt", 1, new(null, "", "", "", null, []), "old", "text/plain", null, false, null));
        foreach (var read in reads) try { await read; throw new Exception("Expected disposed reply to be rejected"); } catch (OperationCanceledException) { }
        Check(fake.Requests.Count == 4, "cancelled queued cards never reach the host");
    }
    private sealed class ThumbnailFake : INativeJourneyClient
    {
        public readonly Dictionary<string, TaskCompletionSource<JourneyArtifact>> Requests = []; public readonly List<string> Projects = [];
        public Task<JourneyPage> ReadAsync(string project, long from, long until, CancellationToken token = default) => throw new NotSupportedException();
        public Task<JourneyArtifact> ThumbnailAsync(string project, string version, CancellationToken token = default) { Projects.Add(project); var reply = new TaskCompletionSource<JourneyArtifact>(); Requests.Add(version, reply); return reply.Task; }
    }
    private sealed class Fake : INativeJourneyClient
    {
        public Func<long, long, Task<JourneyPage>> Read = (_, _) => Task.FromResult(new JourneyPage([], false));
        public Func<Task<JourneyGraph>> Graph = () => Task.FromResult(new JourneyGraph([], []));
        public Func<Task<string>>? Write;
        public bool FailWrite; public int Writes; public JourneyNote? LastNote;
        public Task<JourneyPage> ReadAsync(string project, long from, long until, CancellationToken token = default) => Read(from, until);
        public Task<JourneyGraph> GraphAsync(string project, CancellationToken token = default) => Graph();
        public Task<string> AddAsync(string project, JourneyNote input, CancellationToken token = default)
        { Writes++; LastNote = input; if (FailWrite) throw new IOException("Lost write reply"); return Write?.Invoke() ?? Task.FromResult("entry"); }
    }
    private sealed class Transport : INativeSettingsClient
    {
        public JsonNode? Reply; public string? Command, Project; public JsonObject? Args; public int Calls; public bool Fail;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        { Calls++; Command = command; Project = projectId; Args = arguments; if (Fail) throw new IOException("Lost reply"); return Task.FromResult(Reply?.DeepClone()); }
    }
}
