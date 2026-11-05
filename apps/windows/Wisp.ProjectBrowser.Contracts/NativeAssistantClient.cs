using System.Text.Json;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser.Contracts;

// Projection of wisp_dto::native_assistant and ResearchAssistantPlanItem.
public sealed record AssistantProject(string Id, string Name, string Description);
public sealed record AssistantPlan(string Id, string Day, string Title, string? ProjectId, string? ProjectName, string? SessionId, string Status);
public sealed record AssistantWorkspace(string Day, AssistantProject[] Projects, AssistantPlan[] Plan,
    AssistantSchedule[]? Schedules = null, AssistantDailyRecap? DailyRecap = null, AssistantMemoryTidy? MemoryTidy = null,
    AssistantMandateOverview[]? Mandates = null);
// Projections of wisp_dto::ScheduleRecord and DailyRecapAutomation.
public sealed record AssistantSchedule(string Id, string ProjectId, string? FrameId, bool ReplacePreviousTurn,
    string Name, string Prompt, string? Skill, long IntervalSecs, bool Enabled, long NextRunAt, long? LastRunAt, long CreatedAt, long UpdatedAt);
public sealed record AssistantDailyRecap(bool Enabled, string Time, long? LastRunAt, int Drafted, string? Error, bool Running);
// Projections of wisp_dto::MemoryTidyAutomation/Proposal/Entry.
public sealed record AssistantMemoryEntry(string Id, string Content);
public sealed record AssistantMemoryProposal(string Id, string Action, AssistantMemoryEntry[] Memories, string Text, string Reason);
public sealed record AssistantMemoryTidy(bool Enabled, long? LastRunAt, string? Error, bool Running, AssistantMemoryProposal[] Proposals);
// Projection of wisp_dto::AssistantWeixinStatus.
public sealed record NativeAssistantWeixinStatus(bool Enabled, bool Bound, string State, string Detail);

public sealed class NativeAssistantClient(INativeSettingsClient transport)
{
    public const string ProjectId = "assistant:research", SessionId = "research-assistant";
    private static readonly JsonSerializerOptions Json = new() { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower };
    public async Task OpenAsync(CancellationToken token = default)
    {
        var result = await transport.InvokeAsync("native_assistant_open", new(), null, token);
        if (result?["project_id"]?.GetValue<string>() != ProjectId || result?["session_id"]?.GetValue<string>() != SessionId)
            throw new InvalidDataException("Assistant identity mismatch");
    }
    public async Task<AssistantWorkspace> ReadAsync(DateTime day, CancellationToken token = default)
    {
        var key = day.ToString("yyyy-MM-dd", System.Globalization.CultureInfo.InvariantCulture);
        var result = await transport.InvokeAsync("native_assistant_workspace", new() { ["day"] = key }, null, token);
        var value = result?.Deserialize<AssistantWorkspace>(Json) ?? throw new InvalidDataException("Missing assistant workspace");
        if (value.Day != key || value.Projects == null || value.Plan == null
            || value.Projects.Any(p => p == null || string.IsNullOrWhiteSpace(p.Id) || p.Id.StartsWith("assistant:", StringComparison.Ordinal))
            || value.Projects.Select(p => p.Id).Distinct(StringComparer.Ordinal).Count() != value.Projects.Length
            || value.Plan.Any(p => p == null || string.IsNullOrWhiteSpace(p.Id) || p.ProjectId != null && !value.Projects.Any(project => project.Id == p.ProjectId)))
            throw new InvalidDataException("Invalid assistant workspace visibility or date");
        return value;
    }
    public async Task<AssistantWorkspace> ReadAutomationAsync(DateTime day, CancellationToken token = default)
    {
        var value = await ReadAsync(day, token);
        if (value.Schedules == null || value.DailyRecap == null || value.DailyRecap.Time == null
            || value.Schedules.Any(s => s == null || string.IsNullOrWhiteSpace(s.Id) || s.IntervalSecs <= 0
                || !value.Projects.Any(p => p.Id == s.ProjectId))
            || value.Schedules.Select(s => s.Id).Distinct(StringComparer.Ordinal).Count() != value.Schedules.Length)
            throw new InvalidDataException("Invalid assistant automation snapshot");
        ValidateTidy(value.MemoryTidy);
        NativeMandateContract.Validate(value.Mandates, value.Projects);
        return value;
    }
    public async Task<AssistantMemoryTidy> MutateMemoryTidyAsync(JsonObject operation, CancellationToken token = default)
    {
        var value = (await MutateAsync(operation, token))?.Deserialize<AssistantMemoryTidy>(Json);
        ValidateTidy(value); return value!;
    }
    public async Task<AssistantDailyRecap> MutateDailyRecapAsync(JsonObject operation, CancellationToken token = default)
    {
        var value = (await MutateAsync(operation, token))?.Deserialize<AssistantDailyRecap>(Json);
        return value?.Time != null ? value : throw new InvalidDataException("Invalid daily recap response");
    }
    private static void ValidateTidy(AssistantMemoryTidy? value)
    {
        if (value?.Proposals == null || value.Proposals.Any(p => p == null || string.IsNullOrWhiteSpace(p.Id)
            || p.Action is not ("merge" or "retire") || p.Memories == null || p.Memories.Length == 0
            || p.Memories.Any(m => m == null || string.IsNullOrWhiteSpace(m.Id) || m.Content == null)
            || p.Memories.Select(m => m.Id).Distinct(StringComparer.Ordinal).Count() != p.Memories.Length
            || p.Text == null || p.Reason == null)
            || value.Proposals.Select(p => p.Id).Distinct(StringComparer.Ordinal).Count() != value.Proposals.Length)
            throw new InvalidDataException("Invalid or missing memory tidy state. Use the matching native desktop host.");
    }
    public Task<JsonNode?> MutateAsync(JsonObject operation, CancellationToken token = default)
        => transport.InvokeAsync("native_assistant_mutate", new() { ["operation"] = operation }, null, token);
}
