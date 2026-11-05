using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeMandateTests
{
    private static void Check(bool value, string why) { if (!value) throw new InvalidOperationException(why); }
    public static async Task RunAsync(string projectFixture)
    {
        var contracts = Path.GetDirectoryName(Path.GetDirectoryName(Path.GetDirectoryName(Path.GetFullPath(projectFixture))))!;
        var workspace = JsonNode.Parse(await File.ReadAllTextAsync(Path.Combine(contracts, "native-assistant/v1/workspace.json")))!["workspace"]!.AsObject();
        var fake = new Fake(workspace); var now = new DateTimeOffset(2026, 10, 30, 9, 0, 0, TimeSpan.Zero);
        using var model = new NativeAutomationModel(new(fake), () => now, TimeZoneInfo.Utc);
        await model.RefreshAsync(); Check(model.Ready && model.Snapshot!.Mandates!.Single().Reports.Length == 1, "shared fixture exposes round, request and report");
        foreach (var locale in new[] { false, true }) for (var index = 0; index < 4; index++)
        {
            var template = NativeMandateTemplates.Create(index, locale, now, TimeZoneInfo.Utc);
            Check(template.Kpis.Count > 0 && template.Goal.Length > 0 && template.EndAt(TimeZoneInfo.Utc) > now.ToUnixTimeSeconds(), "all bilingual templates include a KPI and future deadline");
        }
        model.Begin(0); model.BeginMandate(1); Check(model.Draft == null && model.MandateDraft!.Kpis.Count == 2, "mandate and scheduled-task forms share one layer");
        var draft = model.MandateDraft!; draft.Notes = " first instruction \n\n second instruction "; draft.Until = "2026-10-30";
        var firstKpi = draft.Kpis[0]; model.ChangeMandateKpi(); model.ChangeMandateKpi(2);
        Check(ReferenceEquals(draft, model.MandateDraft) && ReferenceEquals(firstKpi, draft.Kpis[0]) && draft.Kpis.Count == 2
            && draft.Notes.Contains("second instruction") && fake.Writes.Count == 0, "editing KPI rows preserves the rest of the unsaved form");
        var op = draft.Operation(now, TimeZoneInfo.Utc);
        Check(op["draft"]!["ends_at"]!.GetValue<long>() == new DateTimeOffset(2026, 10, 30, 23, 59, 59, TimeSpan.Zero).ToUnixTimeSeconds(), "deadline is the selected local end of day");
        Check(op["draft"]!["constraints"]!["notes"]!.AsArray().Count == 2 && op["draft"]!["start_at"]!.GetValue<long>() == now.ToUnixTimeSeconds(), "creation retains constraints and starts at the next poll");
        draft.Until = "2026-02-30"; await model.SaveMandateAsync(); Check(fake.Writes.Count == 0 && model.MandateError != null, "invalid deadline never dispatches");
        draft.Until = ""; fake.FailWrite = true; await model.SaveMandateAsync(); Check(ReferenceEquals(draft, model.MandateDraft) && fake.Writes.Count == 1 && model.MandateError != null, "failed create preserves complete draft without replay");
        fake.FailWrite = false; await model.SaveMandateAsync(); Check(model.MandateDraft == null && fake.Writes.Count == 2, "explicit retry dispatches once");
        model.BeginMandate(id: "mandate-a"); Check(model.MandateDraft!.Kpis[0].Current == "2", "edit retains reported KPI value");
        model.MandateDraft.ProjectId = "outside"; await model.SaveMandateAsync(); Check(fake.Writes.Count == 2, "edit cannot rebind a mandate");
        model.CancelMandate(); model.BeginMandate(); await model.DeleteMandateAsync("mandate-a");
        Check(model.Escape() && model.MandateDraft != null && model.DeletingMandate == null && fake.Writes.Count == 2, "immediate Escape cancels deletion first without removing the draft");
        Check(model.Escape() && model.MandateDraft == null && !model.Escape(), "following Escape removes only the form before yielding to page");
        model.SetReply("request-a", "The PDF is available"); fake.FailWrite = true; await model.ReplyMandateAsync("mandate-a", "request-a");
        Check(model.ReplyText("request-a").Length > 0 && fake.Writes.Count == 3, "reply failure retains answer and does not replay");
        fake.FailWrite = false; await model.ReplyMandateAsync("mandate-a", "unknown"); Check(fake.Writes.Count == 3, "stale request id cannot send to another request");
        await model.ReplyMandateAsync("mandate-a", "request-a"); Check(model.ReplyText("request-a") == "" && fake.Writes.Count == 4, "accepted reply clears its draft once");
        Check(fake.Scopes.All(p => p == null), "only checked native assistant boundary resolves project scope");
        model.BeginMandate(2); fake.FailRead = true; await model.RefreshAsync();
        Check(model.Snapshot == null && !model.Ready && model.MandateDraft != null, "failed visibility read removes actions and preserves draft");
        fake.FailRead = false; await model.RefreshAsync(); fake.PendingWrite = new(); var pending = model.SaveMandateAsync(); await model.SaveMandateAsync();
        Check(fake.Writes.Count == 5, "double save serialized"); model.Dispose(); fake.PendingWrite.SetResult(null); await pending;
        Check(model.Closed && model.Snapshot == null && model.MandateDraft == null, "late save cannot restore closed form");
        var invalid = new Fake(workspace); invalid.Workspace["mandates"]![0]!["mandate"]!["project_id"] = "hidden";
        using var invalidModel = new NativeAutomationModel(new(invalid)); await invalidModel.RefreshAsync(); Check(!invalidModel.Ready && invalidModel.Error != null, "hidden project in mandate rejected");
        invalid.Workspace["mandates"] = null; await invalidModel.RefreshAsync(); Check(!invalidModel.Ready, "missing state is not a false empty list");
        using var offline = new NativeAutomationModel(new(new Fake(workspace) { FailRead = true }));
        await offline.RefreshAsync(); Check(!offline.Busy && !offline.Ready && offline.Snapshot == null && offline.Error != null,
            "initial read failure ends loading with unavailable data rather than an empty mandate list");
    }
    private sealed class Fake(JsonObject workspace) : INativeSettingsClient
    {
        public JsonObject Workspace = (JsonObject)workspace.DeepClone();
        public bool FailRead, FailWrite;
        public List<JsonObject> Writes = [];
        public List<string?> Scopes = [];
        public TaskCompletionSource<JsonNode?>? PendingWrite;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Scopes.Add(projectId);
            if (command == "native_assistant_workspace")
            {
                if (FailRead) throw new IOException("Read failed"); var copy = Workspace.DeepClone(); copy["day"] = args["day"]!.DeepClone(); return Task.FromResult<JsonNode?>(copy);
            }
            if (command == "native_assistant_mutate")
            {
                Writes.Add((JsonObject)args["operation"]!.DeepClone()); if (FailWrite) throw new IOException("Write result unknown"); return PendingWrite?.Task ?? Task.FromResult<JsonNode?>(null);
            }
            throw new InvalidOperationException(command);
        }
    }
}
