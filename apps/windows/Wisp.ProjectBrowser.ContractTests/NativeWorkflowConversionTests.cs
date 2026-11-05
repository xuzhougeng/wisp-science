using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeWorkflowConversionTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>> { ["en"] = new() { ["workflow_studio.portfolio.validation"] = "Choose inputs", ["workflow_studio.portfolio.template_description"] = "Converted with {model}" } }, "en");
        var skills = Enumerable.Range(0, 10).Select(i => new JsonObject { ["id"] = "skill-" + i, ["name"] = "Display " + i }).ToArray();
        var models = new[] { new JsonObject { ["id"] = "model-a", ["label"] = "Configured model" } };
        using var conversion = new NativeWorkflowConversionModel("source-project"); conversion.Configure(skills, models);
        var fake = new Fake();
        Check(!await conversion.GenerateAsync(fake, locale) && fake.Conversions == 0, "missing input never dispatches a conversion");
        conversion.SetRequest("Review evidence"); conversion.SetAutomatic(false);
        foreach (var skill in skills) conversion.SetSource(skill["id"]!.ToString(), true);
        Check(conversion.Sources.Count == 8 && conversion.Ready, "manual source selection is capped at eight canonical IDs");
        conversion.Show(true); var pending = conversion.GenerateAsync(fake, locale); var id = conversion.RequestId;
        conversion.Show(false); conversion.SetRequest("wrong replacement"); conversion.SetModel("wrong model");
        Check(conversion.Busy && conversion.Request == "Review evidence" && conversion.ModelId == "model-a", "hidden in-flight conversion freezes its captured inputs");
        Check(fake.Scope == "source-project" && fake.Args?["expectedProjectId"]?.ToString() == "source-project" && fake.Args?["conversionId"]?.ToString() == id, "conversion retains request and project correlation");
        Check(fake.Args?["request"]?["source_skill_ids"]?[0]?.ToString() == "skill-0", "requests use source IDs, never display names");
        fake.ProgressId = id; fake.ProgressStage = "selecting_sources";
        await conversion.RefreshProgressAsync(fake, id!);
        Check(conversion.StatusKey.EndsWith(".selecting") && fake.ProgressProject == "source-project", "host stages are localized and read in the captured source project");
        foreach (var (stage, label) in new[] { ("reading_sources", "reading"), ("generating", "generating"), ("validating", "validating"), ("repairing", "repairing"), ("saving", "saving") })
        { fake.ProgressStage = stage; await conversion.RefreshProgressAsync(fake, id!); Check(conversion.Stage == label, "all real host stages map to shared WebView labels"); }
        fake.ProgressId = "unrelated-request"; fake.ProgressStage = "generating"; await conversion.RefreshProgressAsync(fake, id!);
        Check(conversion.Stage == "saving", "another conversion's stage cannot replace this progress");
        fake.ProgressId = id; fake.ProgressStage = "unknown-future-stage"; await conversion.RefreshProgressAsync(fake, id!);
        Check(conversion.Stage == "saving", "unknown host stages do not create untranslated labels");
        var reads = fake.ProgressReads; await conversion.RefreshProgressAsync(fake, "stale-request");
        Check(fake.ProgressReads == reads, "stale local correlations are rejected before dispatch");
        fake.Completion.SetResult(Draft()); Check(await pending && conversion.Result != null && !conversion.DialogOpen, "completion survives a hidden initiating view");
        await conversion.RefreshProgressAsync(fake, id!);
        Check(conversion.StatusKey.EndsWith(".ready") && fake.ProgressReads == reads, "terminal conversion stops reads and retains its completed status");
        conversion.Show(true); Check(conversion.RequestId == id && fake.Conversions == 1, "reopening retains the result without replaying the model request");
        Check(NativeWorkflowConversionModel.SourceIds(conversion.Result!).SequenceEqual(["skill-0", "skill-1"]), "provenance deduplicates plan sources independently of executable nodes");
        using var studio = new NativeWorkflowStudioModel(fake, "source-project", locale); await studio.LoadAsync();
        using var defaultModel = new NativeWorkflowConversionModel("source-project"); defaultModel.Configure(skills, studio.Models);
        Check(defaultModel.ModelId == "model-b", "conversion defaults to the active eligible model after joining the profile catalog");
        Check(studio.UseConversion(conversion.Result!) && studio.HasChanges && studio.SelectedId == "" && studio.Tasks.Length == 2, "adopting a result creates an unsaved graph from actual proposal nodes");
        Check(studio.ConversionSourceSha256 == "source-hash" && !studio.Tasks.Any(t => t["skill_ids"] is JsonArray { Count: > 0 }), "source provenance is retained without runtime Skill bindings");
        fake.FailSave = true; Check(!await studio.SaveAsync() && studio.ConversionSourceSha256 == "source-hash" && studio.HasChanges, "failed save retains the converted graph and fingerprint");
        Check(fake.Args?["conversionSourceSha256"]?.ToString() == "source-hash" && fake.Saves == 1, "fingerprint is sent only on explicit save and never retried automatically");
        fake.FailSave = false; Check(await studio.SaveAsync() && studio.SelectedId == "saved-copy" && studio.ConversionSourceSha256 == null, "successful save adopts host identity and clears transient conversion state");
        conversion.SetRequest("Changed research request"); Check(conversion.Result == null && conversion.RequestId == null, "editing inputs invalidates stale results");
        conversion.SetLegacy("legacy-template"); fake.Completion = new(); pending = conversion.GenerateAsync(fake, locale);
        Check(fake.Args?["request"]?["source_skill_ids"] is JsonArray { Count: 0 } && fake.Args?["request"]?["legacy_template_id"]?.ToString() == "legacy-template", "legacy conversion does not combine Skill and template sources");
        fake.Completion.SetResult(new JsonObject { ["proposal"] = new JsonObject() }); Check(!await pending && conversion.Error != null && conversion.Result == null, "malformed replies cannot become editable drafts");
        var shortName = NativeWorkflowConversionModel.ShortName(string.Concat(Enumerable.Repeat("🧬", 70)) + "\nsecond line");
        Check(shortName.EnumerateRunes().Count() == 60 && shortName.EndsWith('…') && !shortName.Contains("second"), "workflow names respect Unicode scalar limits and the first line");
        using var closedWindow = new NativeWorkflowConversionModel("closed-project"); closedWindow.Configure(skills, models); closedWindow.SetRequest("Review evidence");
        var late = new Fake(); var afterClose = closedWindow.GenerateAsync(late, locale); closedWindow.Dispose(); late.Completion.SetResult(Draft());
        Check(!await afterClose && closedWindow.Result == null && closedWindow.Closed, "closing the owning window prevents a late conversion from restoring UI state");
        using var channelFailure = new NativeWorkflowConversionModel("source-project"); channelFailure.Configure(skills, models); channelFailure.SetRequest("Review evidence");
        var unsupported = new Fake(); var withoutProgress = channelFailure.GenerateAsync(unsupported, locale);
        Check(unsupported.ProgressReads == 0, "older hosts without the advertised command are never polled");
        unsupported.Completion.SetResult(Draft()); Check(await withoutProgress, "capability fallback does not prevent conversion");
        using var failedRead = new NativeWorkflowConversionModel("source-project"); failedRead.Configure(skills, models); failedRead.SetRequest("Review evidence");
        var disconnected = new Fake { ProgressSupported = true, FailProgress = true }; var independent = failedRead.GenerateAsync(disconnected, locale);
        Check(disconnected.ProgressReads == 1 && failedRead.Busy && failedRead.Error == null, "failed progress reads do not fail the original conversion");
        disconnected.Completion.SetResult(Draft()); Check(await independent && disconnected.Conversions == 1, "a failed progress channel never replays the model request");
        using var racing = new NativeWorkflowConversionModel("source-project"); racing.Configure(skills, models); racing.SetRequest("Review evidence");
        var delayed = new Fake { ProgressSupported = true, PendingProgress = new() }; var raced = racing.GenerateAsync(delayed, locale);
        var capturedId = racing.RequestId; delayed.Completion.SetResult(Draft()); Check(await raced, "a stalled progress read cannot delay completion");
        delayed.PendingProgress.SetResult(new JsonObject { ["conversion_id"] = capturedId, ["stage"] = "repairing" });
        Check(racing.StatusKey.EndsWith(".ready"), "late progress after completion cannot resurrect busy state");
        using var disposing = new NativeWorkflowConversionModel("source-project"); disposing.Configure(skills, models); disposing.SetRequest("Review evidence");
        var owned = new Fake { ProgressSupported = true, PendingProgress = new() }; var disposedRequest = disposing.GenerateAsync(owned, locale);
        var disposedId = disposing.RequestId; disposing.Dispose();
        owned.PendingProgress.SetResult(new JsonObject { ["conversion_id"] = disposedId, ["stage"] = "generating" });
        owned.Completion.SetResult(Draft());
        Check(!await disposedRequest && disposing.Closed && disposing.Stage == "preparing" && disposing.Result == null, "owner disposal cancels stage reads and invalidates both late reply paths");
    }
    private static JsonObject Draft() => JsonNode.Parse("""
        {"plan":{"planner_model_label":"Configured model","source_sha256":"source-hash","rationale":"Selected evidence methods","tasks":[{"id":"source-summary-only","skill_ids":["skill-1","skill-0","skill-1"]}]},"proposal":{"goal":"Evidence review","context":"","approval_policy":"review_all","tasks":[{"id":"read","task_kind":"agent","instruction":"Review sources","capabilities":["reasoning"],"depends_on":[]},{"id":"write","task_kind":"agent","instruction":"Summarize independently","capabilities":["reasoning"],"depends_on":["read"],"output_schema":{"type":"object"}}]}}
        """)!.AsObject();
    private sealed class Fake : INativeSettingsClient
    {
        public int Conversions, Saves; public string? Scope; public JsonObject? Args; public bool FailSave;
        public TaskCompletionSource<JsonNode?> Completion = new();
        public bool ProgressSupported, FailProgress; public int ProgressReads; public string? ProgressId, ProgressStage, ProgressProject;
        public TaskCompletionSource<JsonNode?>? PendingProgress;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "plan_skill_portfolio") { Conversions++; Scope = projectId; Args = (JsonObject)args.DeepClone(); return Completion.Task; }
            if (command == "native_settings_capabilities") return Task.FromResult<JsonNode?>(new JsonObject { ["commands"] = ProgressSupported ? new JsonArray("native_workflow_conversion_progress") : new JsonArray() });
            if (command == "native_workflow_conversion_progress")
            {
                ProgressReads++; ProgressProject = projectId;
                if (FailProgress) throw new IOException("Synthetic progress read failed");
                return PendingProgress?.Task ?? Task.FromResult<JsonNode?>(new JsonObject { ["conversion_id"] = ProgressId, ["stage"] = ProgressStage });
            }
            if (command == "save_workflow_template") { Saves++; Args = (JsonObject)args.DeepClone(); if (FailSave) throw new IOException("Synthetic save failure"); var saved = (JsonObject)args["template"]!.DeepClone(); saved["id"] = "saved-copy"; return Task.FromResult<JsonNode?>(saved); }
            if (command == "list_models") return Task.FromResult(JsonNode.Parse("""[{"id":"model-a","label":"First"},{"id":"model-b","label":"Active","active":true}]"""));
            if (command == "get_dynamic_agent_options") return Task.FromResult(JsonNode.Parse("""{"capabilities":[],"executors":[],"skills":[],"models":[{"id":"model-a"},{"id":"model-b"}]}"""));
            return Task.FromResult<JsonNode?>(new JsonArray());
        }
    }
}
