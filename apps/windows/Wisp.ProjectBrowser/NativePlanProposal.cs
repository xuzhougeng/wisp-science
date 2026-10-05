using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativePlanTarget(string ProjectId, string SessionId, string Key, NativePlanProposal Proposal);

public static class NativePlanProposals
{
    public static NativePlanTarget? Latest(ConversationSnapshot? snapshot)
    {
        if (snapshot == null) return null;
        var user = Array.FindLastIndex(snapshot.Items, row => row.Role == "user");
        var index = Array.FindLastIndex(snapshot.Items, row => row.Role == "plan");
        if (user < 0 || index <= user || snapshot.Items[index].Proposal is not { Valid: true } proposal) return null;
        var turn = (snapshot.UserOffset ?? 0) + snapshot.Items.Count(row => row.Role == "user") - 1;
        var identity = snapshot.HistoryState?.Turns.FirstOrDefault(row => row.UserIndex == turn);
        var key = JsonSerializer.Serialize(new { snapshot.ProjectId, snapshot.SessionId, turn, identity?.UserSeq, identity?.Digest, proposal });
        return new(snapshot.ProjectId, snapshot.SessionId, key, proposal);
    }
    public static string? AcpExitMode(NativeAcpSessionState? state)
    {
        if (state?.CurrentMode?.Contains("plan", StringComparison.OrdinalIgnoreCase) != true) return null;
        return state.ModeChoices.FirstOrDefault(row => row.Id == "default")?.Id
            ?? state.ModeChoices.FirstOrDefault(row => !row.Id.Contains("plan", StringComparison.OrdinalIgnoreCase))?.Id;
    }
}

public sealed partial class WorkspaceConversationModel
{
    private readonly HashSet<string> uncertainPlanDecisions = [];
    public NativePlanTarget? LatestProposal => NativePlanProposals.Latest(Snapshot);
    public bool PlanDecisionUncertain(NativePlanTarget target) => uncertainPlanDecisions.Contains(target.Key);
    public bool ProposalModeActive(NativePlanTarget target) => Snapshot != null && LatestProposal?.Key == target.Key
        && target.ProjectId == projectId && target.SessionId == sessionId
        && (target.Proposal.Source == "native"
            ? Snapshot.AcpState == null && Snapshot.AcpAgentId == null && !Snapshot.ModelId.StartsWith("acp:", StringComparison.Ordinal) && Snapshot.PlanMode == true
            : NativePlanProposals.AcpExitMode(Snapshot.AcpState) != null);
    public bool CanDecidePlan(NativePlanTarget target) => CanAttach && !Effort.Busy && !Options.Busy && !QueueUncertain
        && Snapshot is { Running: false, Stopping: false } && ProposalModeActive(target) && !PlanDecisionUncertain(target);
    public void AcknowledgePlanDecision(NativePlanTarget target)
    {
        if (Busy || ConnectionError != null || LatestProposal?.Key != target.Key) return;
        uncertainPlanDecisions.Remove(target.Key); OperationError = null; Notify();
    }
    public async Task DecidePlanAsync(NativePlanTarget target, bool execute, CancellationToken token = default)
    {
        if (!CanDecidePlan(target) || token.IsCancellationRequested) return;
        var current = generation; var originalDraft = Draft;
        var originalContext = JsonSerializer.Serialize(new { Attachments, References, Quotes });
        var exitMode = target.Proposal.Source == "acp" ? NativePlanProposals.AcpExitMode(Snapshot!.AcpState) : null;
        var confirmed = false;
        Busy = true; OperationError = null; Notify();
        try
        {
            // Leaving plan mode must be acknowledged before an execution turn.
            if (exitMode != null)
                await client.AcpSettingAsync(target.ProjectId, target.SessionId, new() { ["kind"] = "mode", ["id"] = exitMode }, token);
            else await client.SetPlanModeAsync(target.ProjectId, target.SessionId, false, token);
            if (current != generation) return;
            await RefreshAsync(token);
            if (current != generation) return;
            confirmed = ConnectionError == null && Snapshot is { Running: false, Stopping: false, ReadOnly: false }
                && (exitMode == null ? Snapshot.PlanMode == false : Snapshot.AcpState?.CurrentMode == exitMode);
            if (!confirmed) throw new InvalidDataException("尚未确认退出计划模式。");
            if (LatestProposal?.Key != target.Key) { confirmed = false; OperationError = "计划内容已变化；已退出计划模式，请核对后再发送。"; }
            else if (execute && (Draft != originalDraft || JsonSerializer.Serialize(new { Attachments, References, Quotes }) != originalContext))
            { confirmed = false; OperationError = "输入内容已变化；已退出计划模式，当前草稿已保留，请核对后再发送。"; }
        }
        catch (Exception ex)
        {
            uncertainPlanDecisions.Add(target.Key);
            confirmed = false;
            if (current == generation) OperationError = "计划模式切换未确认，不会自动重试或启动执行。" + ex.Message;
        }
        finally { if (current == generation) { Busy = false; Notify(); } }
        if (current != generation || !confirmed || !execute || token.IsCancellationRequested) return;
        if (string.IsNullOrWhiteSpace(Draft)) Draft = "批准并执行";
        await SendAsync(token);
    }
}
