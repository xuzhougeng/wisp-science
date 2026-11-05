using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>A mounted, explicit-project review. Choices invalidate file fingerprints;
/// acknowledged roots are never replayed and an unknown write stops the batch.</summary>
public sealed class WorkspaceSessionDeletion(INativeSettingsClient client, string projectId, IEnumerable<string> ids) : IDisposable
{
    public string[] SessionIds { get; } = ids.Distinct(StringComparer.Ordinal).ToArray();
    public event Action? Changed;
    public bool IncludeChildren { get; private set; } = true;
    public bool IncludeArtifacts { get; private set; }
    public bool Loading { get; private set; }
    public bool Deleting { get; private set; }
    public bool MutationUnconfirmed { get; private set; }
    public bool Completed { get; private set; }
    public SessionDeletePreview? Preview { get; private set; }
    public string? Error { get; private set; }
    public HashSet<string> Removed { get; } = new(StringComparer.Ordinal);
    private int generation;
    private bool closed;
    public bool CanDelete => !closed && !Loading && !Deleting && !Completed && !MutationUnconfirmed && Error == null && Preview != null
        && (IncludeChildren || Preview.Children.Explorations == 0)
        && (!IncludeArtifacts || Preview.ArtifactError == null && Preview.Artifacts.Length > 0);
    public IEnumerable<SessionArtifactPreview> ArtifactPreviews => Preview?.Artifacts.SelectMany(f => new[] { f.Preview }.Concat(f.Children.Select(c => c.Preview))) ?? [];
    public Task ChooseChildrenAsync(bool value) { if (Deleting || MutationUnconfirmed) return Task.CompletedTask; IncludeChildren = value; return LoadAsync(); }
    public Task ChooseArtifactsAsync(bool value) { if (Deleting || MutationUnconfirmed) return Task.CompletedTask; IncludeArtifacts = value; return LoadAsync(); }
    public async Task LoadAsync()
    {
        if (closed || Deleting || MutationUnconfirmed) return;
        var current = ++generation; var children = IncludeChildren; var artifacts = IncludeArtifacts;
        Preview = null; Error = null; Loading = true; Notify();
        try
        {
            if (SessionIds.Length == 0 || SessionIds.Length > 1000 || SessionIds.Any(string.IsNullOrWhiteSpace)) throw new InvalidDataException("Choose between 1 and 1000 conversations");
            var value = (await client.InvokeAsync("native_project_session_delete_preview", new()
            {
                ["session_ids"] = JsonSerializer.SerializeToNode(SessionIds), ["include_children"] = children, ["include_artifacts"] = artifacts
            }, projectId))?.Deserialize<SessionDeletePreview>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing deletion preview");
            if (closed || generation != current) return;
            Validate(value, children, artifacts); Preview = value;
        }
        catch (Exception ex) { if (!closed && generation == current) Error = ex.Message; }
        finally { if (!closed && generation == current) { Loading = false; Notify(); } }
    }
    private void Validate(SessionDeletePreview value, bool children, bool artifacts)
    {
        if (value.ProjectId != projectId || !value.SessionIds.SequenceEqual(SessionIds) || value.Children == null || value.Children.Nested == null
            || value.Children.Subagents < 0 || value.Children.Branches < 0 || value.Children.Explorations < 0
            || value.Children.Nested.Any(id => !SessionIds.Contains(id))) throw new InvalidDataException("Deletion preview does not match the selected conversations");
        bool? expected = !value.Children.IsEmpty ? children : value.Children.Nested.Length > 0 ? true : null;
        if (value.IncludeChildren != expected || value.Artifacts == null) throw new InvalidDataException("Deletion choices changed; refresh the preview");
        if (!artifacts || value.ArtifactError != null) return;
        var roots = SessionIds.Where(id => expected != true || !value.Children.Nested.Contains(id)).ToHashSet(StringComparer.Ordinal);
        if (roots.Count != value.Artifacts.Length || !roots.SetEquals(value.Artifacts.Select(f => f.Id))
            || value.Artifacts.Any(f => f.Children == null || expected != true && f.Children.Length > 0
                || f.Children.Select(c => c.Id).Distinct().Count() != f.Children.Length
                || new[] { f.Preview }.Concat(f.Children.Select(c => c.Preview)).Any(p => p == null || string.IsNullOrEmpty(p.Fingerprint) || p.Artifacts == null || p.Files == null || p.Retained == null)))
            throw new InvalidDataException("Incomplete artifact preview; refresh before deleting");
    }
    public async Task<bool> DeleteAsync()
    {
        if (!CanDelete || Preview is not { } review) return false;
        Deleting = true; Notify();
        try
        {
            foreach (var id in SessionIds.Where(id => review.IncludeChildren != true || !review.Children.Nested.Contains(id)))
            {
                if (closed) return false;
                var args = new JsonObject { ["session_id"] = id, ["include_children"] = review.IncludeChildren, ["include_artifacts"] = IncludeArtifacts };
                if (IncludeArtifacts)
                {
                    var family = review.Artifacts.Single(f => f.Id == id);
                    args["artifact_fingerprint"] = family.Preview.Fingerprint;
                    args["child_artifact_fingerprints"] = JsonSerializer.SerializeToNode(family.Children.ToDictionary(c => c.Id, c => c.Preview.Fingerprint));
                }
                var result = (await client.InvokeAsync("native_conversation_delete", args, projectId))?.Deserialize<string[]>()
                    ?? throw new InvalidDataException("Missing deletion acknowledgement");
                if (closed) return false;
                if (!result.Contains(id) || result.Any(string.IsNullOrWhiteSpace)) throw new InvalidDataException("Deletion acknowledgement does not match the conversation");
                Removed.UnionWith(result);
            }
            Completed = true; return true;
        }
        catch (Exception ex) { if (!closed) { MutationUnconfirmed = true; Error = ex.Message; } return false; }
        finally { Deleting = false; Notify(); }
    }
    private void Notify() { if (!closed) Changed?.Invoke(); }
    public void Dispose() { closed = true; generation++; }
}
