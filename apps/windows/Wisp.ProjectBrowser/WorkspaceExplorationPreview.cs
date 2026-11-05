using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public enum ExplorationReviewTab { Files, Artifacts, Runs, Decisions, ExternalEffects }
public sealed record ExplorationReviewRow(string Title, string Detail, string? DeltaKind = null, bool Code = false, bool Warning = false);
public sealed record ExplorationReviewSection(string? HeadingKey, ExplorationReviewRow[] Rows);

public sealed class WorkspaceExplorationPreview(INativeSettingsClient client, string projectId, NativeExplorationSummary owner) : WorkspaceActionModel
{
    public NativeExplorationPreview? Preview { get; private set; }
    public NativeExplorationActionResult? Result { get; private set; }
    public bool MutationUnconfirmed { get; private set; }
    public bool ShowFailure => MutationUnconfirmed || Error != null;
    public bool Mutating { get; private set; }
    public bool CanDismiss => !Mutating;
    public bool CanRead => !Closed && !Busy && Result == null;
    public bool CanAct => CanRead && Preview != null && !MutationUnconfirmed;
    public bool CanPromote => CanAct && Preview!.Preview.Exploration.Status == "active" && Preview.Preview.Eligibility.Eligible;
    public bool CanDiscard => CanAct && Preview!.Preview.Exploration.Status is "active" or "failed";
    public bool CanResolveManually => CanAct && Preview!.Preview.Eligibility.ManualResolutionAvailable;
    protected override string UnconfirmedFailure(Exception exception) => exception.Message;
    private JsonObject Args() => new() { ["session_id"] = owner.Exploration.FrameId, ["exploration_id"] = owner.Exploration.Id };
    public Task<bool> LoadAsync()
    {
        if (!CanRead) return Task.FromResult(false);
        return RunAsync(async () => NativeExplorationPreview.Decode(await client.InvokeAsync("native_conversation_exploration_preview", Args(), projectId), projectId, owner), value => Preview = value);
    }
    public Task<bool> OpenFoldersAsync()
    {
        if (!CanResolveManually) return Task.FromResult(false);
        return RunAsync(async () => NativeExplorationActionResult.Decode(await client.InvokeAsync("native_conversation_exploration_manual_open", Args(), projectId), projectId, owner, "manual_open"), _ => { });
    }
    public async Task<bool> CompleteAsync(string action)
    {
        if (!(action switch { "promote" => CanPromote, "discard" => CanDiscard, "manual_finish" => CanResolveManually, _ => false })) return false;
        var args = Args(); if (action == "promote") args["expected_guard_hash"] = Preview!.Preview.Eligibility.ExpectedGuardHash;
        Mutating = true;
        try
        {
            return await RunAsync(async () =>
            {
                try { return NativeExplorationActionResult.Decode(await client.InvokeAsync("native_conversation_exploration_" + action, args, projectId), projectId, owner, action); }
                catch { MutationUnconfirmed = true; throw; }
            }, value => Result = value);
        }
        finally { Mutating = false; if (!Closed) Notify(); }
    }
    public static string BlockerText(NativePromotionBlocker blocker, NativeLocalization locale) => blocker.Code switch
    {
        "MainlineAdvanced" => locale.Text("exploration.blocker_mainline_advanced"),
        "ExternalReferenceChanged" => locale.Text("exploration.blocker_external_reference_changed"),
        "ExplorationBusy" => locale.Text("exploration.blocker_busy"),
        "ExplorationNotPromotable" => locale.Text("exploration.blocker_not_promotable"), _ => blocker.Message,
    };
    public static string TabKey(ExplorationReviewTab tab) => tab switch { ExplorationReviewTab.Files => "exploration.tab_files", ExplorationReviewTab.Artifacts => "exploration.tab_artifacts", ExplorationReviewTab.Runs => "exploration.tab_runs", ExplorationReviewTab.Decisions => "exploration.tab_decisions", _ => "exploration.tab_effects" };
    public static int Count(NativeExplorationDiff diff, ExplorationReviewTab tab) => tab switch
    { ExplorationReviewTab.Files => diff.Files.Length, ExplorationReviewTab.Artifacts => diff.Artifacts.Length, ExplorationReviewTab.Runs => diff.Runs.Length, ExplorationReviewTab.Decisions => diff.Decisions.Length + diff.ResearchEdges.Length, _ => diff.ExternalEffects.Length + diff.ExternalResources.Length };
    public static ExplorationReviewSection[] Sections(NativeExplorationPreviewData preview, ExplorationReviewTab tab)
    {
        var diff = preview.Diff;
        ExplorationReviewRow File(NativeExplorationFile file) => new(file.Path, "", file.Kind, Code: true);
        return tab switch
        {
            ExplorationReviewTab.Files => [new("exploration.diff_branch_changes", diff.Files.Select(File).ToArray()),
                .. preview.MainlineChanges.Files.Length > 0 ? new[] { new ExplorationReviewSection("exploration.diff_mainline_changes", preview.MainlineChanges.Files.Select(File).ToArray()) } : []],
            ExplorationReviewTab.Artifacts => [new(null, diff.Artifacts.Select(row => new ExplorationReviewRow(row.LogicalKey, row.AfterVersionId, Code: true)).ToArray())],
            ExplorationReviewTab.Runs => [new(null, diff.Runs.Select(row => new ExplorationReviewRow(row.Title, row.Status)).ToArray())],
            ExplorationReviewTab.Decisions => [new(null, diff.Decisions.Select(row => new ExplorationReviewRow(row.Title, row.Kind))
                .Concat(diff.ResearchEdges.Select(row => new ExplorationReviewRow(row.Relation, row.SourceId + " → " + row.TargetId, Code: true))).ToArray())],
            _ => [new(null, diff.ExternalEffects.Select(row => new ExplorationReviewRow(row.TargetSummary, row.EffectKind + " · " + row.Recoverability, Warning: true))
                .Concat(diff.ExternalResources.Select(row => new ExplorationReviewRow(row.Kind, row.Uri, Code: true))).ToArray())],
        };
    }
}
