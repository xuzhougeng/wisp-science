using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed class WorkspaceSessionTransfer : IDisposable
{
    private readonly INativeSettingsClient client;
    public BrowserSession Source { get; }
    public string Mode { get; }
    public ProjectSummary[] Targets { get; }
    public string TargetProjectId { get; private set; }
    public bool IncludeArtifacts { get; private set; }
    public bool Loading { get; private set; }
    public bool Transferring { get; private set; }
    public bool MutationUnconfirmed { get; private set; }
    public NativeSessionTransferPreview? Preview { get; private set; }
    public NativeSessionTransferResult? Result { get; private set; }
    public string? Error { get; private set; }
    public event Action? Changed;
    private int generation;
    private bool closed;
    public bool CanTransfer => !closed && !Loading && !Transferring && !MutationUnconfirmed && Result == null
        && Error == null && Preview != null && (!IncludeArtifacts || Preview.Artifacts != null && Preview.ArtifactError == null);
    public WorkspaceSessionTransfer(INativeSettingsClient client, BrowserSession source, string mode, IEnumerable<ProjectSummary> projects)
    {
        if (mode is not ("copy" or "move")) throw new ArgumentException("Unknown transfer mode", nameof(mode));
        this.client = client; Source = source; Mode = mode;
        Targets = projects.Where(p => p.Id != source.ProjectId && !string.IsNullOrWhiteSpace(p.Id)
            && !p.Id.StartsWith("assistant:", StringComparison.Ordinal)).DistinctBy(p => p.Id).ToArray();
        TargetProjectId = Targets.FirstOrDefault()?.Id ?? "";
    }
    public Task ChooseTargetAsync(string target)
    {
        if (closed || Transferring || MutationUnconfirmed || Result != null || !Targets.Any(p => p.Id == target)) return Task.CompletedTask;
        TargetProjectId = target; IncludeArtifacts = false; return LoadAsync();
    }
    public void ChooseArtifacts(bool value)
    {
        if (closed || Transferring || MutationUnconfirmed || Result != null || Mode != "move") return;
        IncludeArtifacts = value; Notify();
    }
    public async Task LoadAsync()
    {
        if (closed || Transferring || MutationUnconfirmed || Result != null) return;
        var current = ++generation; var target = TargetProjectId;
        Preview = null; Error = null;
        if (string.IsNullOrEmpty(target)) { Loading = false; Notify(); return; }
        Loading = true; Notify();
        try
        {
            var value = (await client.InvokeAsync("native_conversation_transfer_preview", new()
            { ["session_id"] = Source.Id, ["target_project_id"] = target, ["mode"] = Mode }, Source.ProjectId))
                ?.Deserialize<NativeSessionTransferPreview>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing transfer preview");
            if (closed || current != generation) return;
            if (value.Schema != NativeSessionTransferPreview.SchemaName || value.ProjectId != Source.ProjectId || value.SessionId != Source.Id
                || value.TargetProjectId != target || value.Mode != Mode || !IsRevision(value.Revision) || value.MessageCount < 0
                || (Mode == "copy" ? value.Artifacts != null || value.ArtifactError != null : (value.Artifacts != null) == (value.ArtifactError != null)))
                throw new InvalidDataException("Transfer preview does not match the selected conversation and project");
            if (value.Artifacts is { } files && (string.IsNullOrEmpty(files.Fingerprint) || files.Artifacts == null || files.Files == null || files.Retained == null
                || files.Files.Any(string.IsNullOrWhiteSpace) || files.Files.Distinct(StringComparer.Ordinal).Count() != files.Files.Length))
                throw new InvalidDataException("Incomplete artifact preview");
            Preview = value;
        }
        catch (Exception ex) { if (!closed && current == generation) Error = ex.Message; }
        finally { if (!closed && current == generation) { Loading = false; Notify(); } }
    }
    public async Task<bool> TransferAsync()
    {
        if (!CanTransfer || Preview is not { } review) return false;
        var current = generation; var files = IncludeArtifacts; Transferring = true; Notify();
        try
        {
            var result = (await client.InvokeAsync("native_conversation_transfer", new()
            {
                ["session_id"] = Source.Id, ["target_project_id"] = review.TargetProjectId, ["mode"] = Mode,
                ["revision"] = review.Revision, ["include_artifacts"] = files,
                ["artifact_fingerprint"] = files ? review.Artifacts!.Fingerprint : null
            }, Source.ProjectId))?.Deserialize<NativeSessionTransferResult>(ConversationSnapshot.JsonOptions)
                ?? throw new InvalidDataException("Missing transfer acknowledgement");
            if (closed || current != generation) return false;
            if (result.Schema != review.Schema || result.ProjectId != Source.ProjectId || result.SessionId != Source.Id
                || result.TargetProjectId != review.TargetProjectId || result.Mode != Mode || result.IncludeArtifacts != files
                || string.IsNullOrWhiteSpace(result.FrameId) || result.FrameId == Source.Id)
                throw new InvalidDataException("Transfer acknowledgement does not match the reviewed operation");
            Result = result; return true;
        }
        catch (Exception ex) { if (!closed && current == generation) { MutationUnconfirmed = true; Error = ex.Message; } return false; }
        finally { if (!closed && current == generation) { Transferring = false; Notify(); } }
    }
    internal static bool IsRevision(string? value) => value?.Length == 64 && value.All(Uri.IsHexDigit);
    private void Notify() { if (!closed) Changed?.Invoke(); }
    public void Dispose() { closed = true; generation++; }
}
