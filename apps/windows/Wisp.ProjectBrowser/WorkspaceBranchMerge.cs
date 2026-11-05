using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed class WorkspaceBranchMerge(INativeSettingsClient client, string projectId, string branchId) : WorkspaceActionModel
{
    public NativeBranchPreview? Preview { get; private set; }
    public NativeBranchMergeResult? Result { get; private set; }
    public string Draft { get; set; } = "";
    public bool MutationUnconfirmed { get; private set; }
    public bool Generating { get; private set; }
    public bool Merging { get; private set; }
    public bool CanDismiss => !Generating && !Merging;
    public bool CanGenerate => !Closed && !Busy && Preview != null && !MutationUnconfirmed && Result == null;
    public bool CanMerge => CanGenerate && !string.IsNullOrWhiteSpace(Draft);
    protected override string UnconfirmedFailure(Exception exception) => exception.Message;
    private T Read<T>(JsonNode? value)
    {
        var reply = value?.Deserialize<NativeBranchReply<T>>();
        if (reply == null || reply.ProjectId != projectId || reply.Result == null) throw new InvalidDataException("Branch response did not match the project");
        return reply.Result;
    }
    public async Task LoadAsync()
    {
        var loaded = await RunAsync(async () =>
        {
            var preview = Read<NativeBranchPreview>(await client.InvokeAsync("native_conversation_branch_preview", new() { ["session_id"] = branchId }, projectId));
            if (preview.BranchSessionId != branchId || string.IsNullOrWhiteSpace(preview.MainSessionId) || preview.MainSessionId == branchId
                || string.IsNullOrWhiteSpace(preview.GuardHash) || preview.Messages == null || preview.CheckpointUserIndex < 0 || preview.NewMessageCount < 0)
                throw new InvalidDataException("Branch preview did not match the conversation");
            return preview;
        }, value => Preview = value);
        if (loaded && !Closed) await GenerateAsync();
    }
    public async Task<bool> GenerateAsync(string? guidance = null)
    {
        if (!CanGenerate || guidance != null && (string.IsNullOrWhiteSpace(guidance) || string.IsNullOrWhiteSpace(Draft)
            || guidance.EnumerateRunes().Count() > 8000 || Draft.EnumerateRunes().Count() > 64000)) return false;
        var args = new JsonObject { ["session_id"] = branchId, ["expected_guard_hash"] = Preview!.GuardHash };
        if (guidance != null) { args["current_version"] = Draft; args["user_guidance"] = guidance; }
        Generating = true;
        try
        {
            return await RunAsync(async () =>
            {
                var text = Read<string>(await client.InvokeAsync("native_conversation_branch_generate", args, projectId));
                if (string.IsNullOrWhiteSpace(text)) throw new InvalidDataException("The generated branch summary was empty");
                return text;
            }, value => Draft = value);
        }
        finally { Generating = false; if (!Closed) Notify(); }
    }
    public async Task<bool> MergeAsync()
    {
        if (!CanMerge) return false;
        var summary = Draft; var preview = Preview!; Merging = true;
        try
        {
            return await RunAsync(async () =>
            {
                try
                {
                    var merged = Read<NativeBranchMergeResult>(await client.InvokeAsync("native_conversation_branch_merge",
                        new() { ["session_id"] = branchId, ["expected_guard_hash"] = preview.GuardHash, ["summary"] = summary }, projectId));
                    if (merged.BranchSessionId != branchId || merged.MainSessionId != preview.MainSessionId || merged.SummaryMessageSeq < 0)
                        throw new InvalidDataException("Merge acknowledgement did not match the reviewed branch");
                    return merged;
                }
                catch { MutationUnconfirmed = true; throw; }
            }, value => Result = value);
        }
        finally { Merging = false; if (!Closed) Notify(); }
    }
}
