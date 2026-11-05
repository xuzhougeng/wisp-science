using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeAutomationTests
{
    private static void Check(bool value, string reason) { if (!value) throw new InvalidOperationException(reason); }
    public static async Task RunAsync()
    {
        var now = new DateTimeOffset(2026, 10, 30, 9, 0, 0, TimeSpan.Zero);
        Check(NativeScheduleDraft.NextSlot("09:00", null, now, TimeZoneInfo.Utc) == now.AddDays(1).ToUnixTimeSeconds(), "daily time must be strictly later than now");
        Check(NativeScheduleDraft.NextSlot("17:00", 5, now, TimeZoneInfo.Utc) == now.AddHours(8).ToUnixTimeSeconds(), "weekly uses the selected local weekday");
        Check(!NativeScheduleDraft.ValidTime("24:00") && !NativeScheduleDraft.ValidTime("9:7"), "malformed wall times are not normalized silently");
        var dst = TimeZoneInfo.FindSystemTimeZoneById(OperatingSystem.IsWindows() ? "Eastern Standard Time" : "America/New_York");
        var gap = NativeScheduleDraft.NextSlot("02:30", null, new DateTimeOffset(2026, 3, 8, 0, 0, 0, TimeSpan.FromHours(-5)), dst);
        Check(gap == new DateTimeOffset(2026, 3, 8, 3, 30, 0, TimeSpan.FromHours(-4)).ToUnixTimeSeconds(), "spring gap follows JS Date's one-hour shift");
        var fold = NativeScheduleDraft.NextSlot("01:30", null, new DateTimeOffset(2026, 11, 1, 0, 0, 0, TimeSpan.FromHours(-4)), dst);
        Check(fold == new DateTimeOffset(2026, 11, 1, 1, 30, 0, TimeSpan.FromHours(-4)).ToUnixTimeSeconds(), "fall overlap uses its earlier occurrence");
        Check(NativeAutomationTemplates.Create(0, true).Weekday == 1 && NativeAutomationTemplates.Create(2, false).Time == "17:00", "template cadence matches WebView");

        var fake = new Fake(); using var model = new NativeAutomationModel(new(fake), () => now, TimeZoneInfo.Utc);
        model.Begin(); Check(model.Draft == null, "unread workspace cannot create a scheduled task");
        await model.RefreshAsync(); model.Begin(0);
        Check(model.Draft?.ProjectId == "p" && fake.Operations.Count == 0, "opening a template only stages an editable draft");
        model.Draft!.Cadence = NativeScheduleCadence.Hourly; model.Draft.Hours = 999;
        await model.CreateAsync(); var created = fake.Operations.Single();
        Check(created["project_id"]!.GetValue<string>() == "p" && created["interval_secs"]!.GetValue<long>() == 604800 && created["start_at"] == null, "hourly clamps to 168 hours without a wall-clock start");
        Check(model.Draft == null && model.Snapshot?.Schedules?.Length == 1, "create reloads authoritative schedules and closes only its form");
        Check(fake.Scopes.All(scope => scope == null), "native automation commands never bind the currently open project");
        await model.DeleteAsync("s"); Check(model.Deleting == "s" && fake.Operations.Count == 1, "first delete click only arms confirmation");
        Check(model.Escape() && model.Deleting == null, "Escape cancels delete confirmation before page navigation");
        await model.SetEnabledAsync("hidden", true); Check(fake.Operations.Count == 1, "unknown schedule IDs cannot mutate");
        await model.SetEnabledAsync("s", false); Check(fake.Operations.Last()["action"]!.GetValue<string>() == "set_enabled", "toggle uses native scheduler boundary");
        model.Begin(); model.Draft!.Prompt = "   "; await model.CreateAsync(); Check(model.FormError != null, "blank prompt is rejected before dispatch");
        model.Draft.Prompt = "Keep this draft"; fake.FailMutation = true; var before = fake.Operations.Count;
        await model.CreateAsync(); Check(fake.Operations.Count == before + 1 && model.Draft?.Prompt == "Keep this draft" && model.FormError != null, "ambiguous mutation preserves input without replay");
        fake.FailMutation = false; fake.FailRead = true; await model.RefreshAsync();
        Check(model.Snapshot == null && !model.Ready, "failed authoritative read removes stale actionable schedules");
        await model.RunAsync("s"); Check(fake.Operations.Count == before + 1, "failed visibility read blocks schedule runs");
        fake.FailRead = false; fake.HideProject = true; await model.RefreshAsync();
        Check(model.Draft?.ProjectId == "" && model.Snapshot?.Schedules?.Length == 0, "hidden project is cleared from the retained draft");
        await model.CreateAsync(); Check(fake.Operations.Count == before + 1, "draft with hidden target cannot be created");
        fake.HideProject = false; await model.RefreshAsync(); model.CancelForm();
        await model.SaveDailyAsync(true, "24:00"); Check(fake.Operations.Count == before + 1, "invalid daily time never dispatches");
        await model.RunDailyAsync(); Check(fake.Operations.Last()["action"]!.GetValue<string>() == "run_daily_recap", "daily execution uses the shared host");
        Check(model.Snapshot!.DailyRecap!.Running, "immediate run response starts polling before the background flag is acquired");
        var dailyWrites = fake.Operations.Count; await model.RunDailyAsync(); Check(fake.Operations.Count == dailyWrites, "running recap cannot be started twice");
        await model.SetMemoryTidyAsync(false); Check(!model.Snapshot!.MemoryTidy!.Enabled, "memory toggle uses returned global state");
        await model.RunMemoryTidyAsync(); Check(model.Snapshot!.MemoryTidy!.Running, "memory run keeps its immediate running response");
        var tidyWrites = fake.Operations.Count; await model.RunMemoryTidyAsync(); Check(fake.Operations.Count == tidyWrites, "running memory tidy cannot be started twice");
        await model.SettleMemoryAsync("unknown", true); Check(fake.Operations.Count == tidyWrites, "unknown proposal cannot be applied");
        fake.FailMutation = true; await model.SettleMemoryAsync("merge", true);
        Check(fake.Operations.Count == tidyWrites + 1 && model.Snapshot!.MemoryTidy!.Proposals.Length == 0 && model.Error != null, "failed stale apply reloads proposals without replaying the write");
        fake.FailMutation = false;
        Check(fake.Scopes.All(scope => scope == null), "daily recap and memory tidy stay globally scoped");
        fake.Pending = new(); var pending = model.RefreshAsync(); model.Dispose(); fake.Pending.SetResult(fake.Workspace(now.Date)); await pending;
        Check(model.Snapshot == null && model.Closed, "late snapshot cannot revive a closed page");
        var malformed = new Fake { InvalidSchedule = true }; using var invalid = new NativeAutomationModel(new(malformed)); await invalid.RefreshAsync();
        Check(invalid.Snapshot == null && invalid.Error != null, "schedule outside the visible projects is rejected");
        var unknownAction = new Fake { InvalidTidy = true }; using var invalidTidy = new NativeAutomationModel(new(unknownAction)); await invalidTidy.RefreshAsync();
        Check(invalidTidy.Snapshot == null && invalidTidy.Error != null, "unknown memory proposal action cannot become an Apply button");
        var pendingFake = new Fake(); using var pendingModel = new NativeAutomationModel(new(pendingFake)); await pendingModel.RefreshAsync(); pendingModel.Begin(1);
        pendingFake.PendingMutation = new(); var saving = pendingModel.CreateAsync(); await pendingModel.CreateAsync();
        Check(pendingFake.Operations.Count == 1, "double submit is serialized");
        pendingModel.Dispose(); pendingFake.PendingMutation.SetResult(null); await saving;
        Check(pendingModel.Snapshot == null, "late mutation completion does not reload a dismissed page");
        var tidyFake = new Fake(); using var tidyModel = new NativeAutomationModel(new(tidyFake)); await tidyModel.RefreshAsync();
        tidyFake.PendingMutation = new(); var applying = tidyModel.SettleMemoryAsync("merge", false); tidyModel.Dispose();
        tidyFake.PendingMutation.SetResult(JsonSerializer.SerializeToNode(tidyFake.Tidy(), Fake.Json)); await applying;
        Check(tidyModel.Snapshot == null && tidyModel.Closed, "late tidy mutation cannot restore a dismissed page");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public List<JsonObject> Operations = []; public List<string?> Scopes = [];
        public bool FailRead, FailMutation, HideProject, InvalidSchedule, InvalidTidy;
        private bool tidyEnabled = true, tidyRunning, settled;
        public static readonly JsonSerializerOptions Json = new() { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower };
        public AssistantMemoryTidy Tidy() => new(tidyEnabled, null, null, tidyRunning,
            settled ? [] : [new("merge", InvalidTidy ? "unexpected" : "merge", [new("a", "Keep figures editable"), new("b", "Keep source figures")], "Keep editable source figures", "Same preference")]);
        public TaskCompletionSource<JsonNode?>? Pending, PendingMutation;
        public JsonNode Workspace(DateTime day) => JsonSerializer.SerializeToNode(new AssistantWorkspace(day.ToString("yyyy-MM-dd"),
            HideProject ? [] : [new("p", "Research", "")], [], HideProject ? [] : [new("s", InvalidSchedule ? "hidden" : "p", null, false, "Watch", "Read records", null, 86400, true, 1793437200, null, 0, 0)], new(true, "09:00", null, 0, null, false), Tidy(), []), Json)!;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Scopes.Add(projectId);
            if (command == "native_assistant_workspace")
            {
                if (FailRead) throw new IOException("Simulated visibility failure");
                return Pending?.Task ?? Task.FromResult<JsonNode?>(Workspace(DateTime.Parse(args["day"]!.GetValue<string>())));
            }
            if (command == "native_assistant_mutate")
            {
                Operations.Add((JsonObject)args["operation"]!.DeepClone());
                var action = args["operation"]!["action"]!.GetValue<string>();
                if (action is "apply_memory_tidy" or "dismiss_memory_tidy") settled = true;
                if (FailMutation) throw new IOException("Simulated ambiguous mutation failure");
                if (PendingMutation != null) return PendingMutation.Task;
                if (action is "set_memory_tidy") tidyEnabled = args["operation"]!["enabled"]!.GetValue<bool>();
                if (action is "run_memory_tidy") tidyRunning = true;
                if (action.Contains("memory_tidy")) return Task.FromResult(JsonSerializer.SerializeToNode(Tidy(), Json));
                if (action.Contains("daily_recap")) return Task.FromResult(JsonSerializer.SerializeToNode(new AssistantDailyRecap(true, "09:00", null, 0, null, true), Json));
                return PendingMutation?.Task ?? Task.FromResult<JsonNode?>(null);
            }
            throw new InvalidOperationException(command);
        }
    }
}
