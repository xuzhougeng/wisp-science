using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public enum ContextRunAction { Cancel, ForceCancel, Cleanup }

public sealed partial class WorkspacePanelModel
{
    private readonly Dictionary<(string Context, string Run), NativeMethodReview> methodReviews = [];
    public NativeMethodReview ContextMethodReview(string context, NativeRun run)
    {
        if (closed || run.ContextId != context || run.Kind != "method_search") throw new InvalidOperationException("Method-search scope changed");
        if (!methodReviews.TryGetValue((context, run.Id), out var review))
            methodReviews[(context, run.Id)] = review = new((op, token) => !closed
                ? client.MethodSearchAsync(projectId, sessionId, context, run.Id, op, token)
                : Task.FromException<NativeMethodDetail>(new InvalidOperationException("Method-search scope changed")));
        review.Observe(run); return review;
    }
    public WorkspaceArtifactModel MethodOutputPreview(string context, NativeMethodDetail detail)
    {
        if (closed || detail.Run.ContextId != context || !methodReviews.TryGetValue((context, detail.Run.Id), out var review) ||
            !ReferenceEquals(review.Detail, detail)) throw new InvalidOperationException("Method-search review changed");
        return WorkspaceArtifactModel.ForMethodOutputs(client, projectId, sessionId, context, detail);
    }
    public Task<NativeRunArtifact[]> ReadContextRunArtifactsAsync(string context, CancellationToken token)
        => client.ContextRunArtifactsAsync(projectId, sessionId, context, token);

    public WorkspaceArtifactModel ContextArtifactPreview(NativeRunArtifact link)
    {
        if (closed || (sessionId != null && link.ReaderSession != sessionId))
            throw new InvalidOperationException("Artifact reader scope changed");
        // A project with no selected conversation uses the verified artifact owner for the reader.
        // An exploration must always read from its own frame to resolve the checkpoint version.
        return new(client, projectId, link.ReaderSession, [
            new(link.ArtifactId, link.Filename, link.Kind, "artifact:" + link.ArtifactId, null, 0, null)
        ], allowLibrary: sessionId != null);
    }
    private readonly HashSet<string> changingContextRuns = [];
    private sealed record PendingRead(string? Error);
    private readonly Dictionary<string, PendingRead> unverifiedContextRuns = [];
    public bool RunActionPending(string id) => changingContextRuns.Contains(id) || unverifiedContextRuns.ContainsKey(id);
    public string? RunActionError(string id) => unverifiedContextRuns.GetValueOrDefault(id)?.Error;

    public static ContextRunAction? CancelAction(NativeRun run) => run.Kind == "method_search" ? null : run.Status switch
    {
        "submitted" or "running" => ContextRunAction.Cancel,
        "cancelling" => ContextRunAction.ForceCancel, _ => null
    };
    public static bool HasWorkspaceActions(NativeRun run) => run.Kind is not ("method_search" or "file_transfer")
        && run.Status is "succeeded" or "failed" or "cancelled" or "timed_out" or "lost"
        && run.CleanedAt == null && !string.IsNullOrWhiteSpace(run.RemoteWorkdir);

    public async Task<bool> ChangeContextRunAsync(string context, NativeRun run, ContextRunAction action, CancellationToken token)
    {
        if (closed || token.IsCancellationRequested || run.CanManage != true || run.ContextId != context || RunActionPending(run.Id)
            || (action == ContextRunAction.Cleanup ? !HasWorkspaceActions(run) : action != CancelAction(run))) return false;
        changingContextRuns.Add(run.Id);
        try
        {
            JsonObject operation = action == ContextRunAction.Cleanup ? new() { ["action"] = "cleanup" }
                : new() { ["action"] = "cancel", ["force"] = action == ContextRunAction.ForceCancel };
            await client.ApplyContextRunAsync(projectId, sessionId, context, run.Id, operation, token);
            unverifiedContextRuns[run.Id] = new(null);
            return true;
        }
        catch (Exception ex) { unverifiedContextRuns[run.Id] = new(ex.Message); return false; }
        finally { changingContextRuns.Remove(run.Id); }
    }
    public WorkspaceRunReviewModel ContextRunReview(string context, NativeRun run, NativeLocalization locale)
    {
        if (closed || run.ContextId != context || !HasWorkspaceActions(run) || !run.CanManage.HasValue)
            throw new InvalidOperationException("No workspace to review in this environment");
        return new((id, op, token) => id == run.Id && !closed
            ? client.ReviewContextRunAsync(projectId, sessionId, context, id, op, token)
            : Task.FromException<NativeRunReviewReply>(new InvalidOperationException("Run review scope changed")), locale: locale);
    }
}
