using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

// The native save picker is injected: cancellation never writes, and a late
// picker/preview cannot export a conversation after its surface has closed.
public sealed class WorkspaceSessionExport(INativeSettingsClient client, BrowserSession source) : WorkspaceActionModel
{
    public bool MutationUnconfirmed { get; private set; }
    public NativeSessionExportResult? Result { get; private set; }
    protected override string UnconfirmedFailure(Exception exception) => exception.Message;
    public Task<bool> ExportAsync(Func<string, Task<string?>> pick, Func<bool> isCurrent)
    {
        if (Closed || Busy || MutationUnconfirmed || Result != null) return Task.FromResult(false);
        return RunAsync(async () =>
        {
            var review = (await client.InvokeAsync("native_conversation_export_preview", new()
                { ["session_id"] = source.Id, ["include_artifacts"] = true }, source.ProjectId))
                ?.Deserialize<NativeSessionExportPreview>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing export preview");
            if (review.Schema != NativeSessionExportPreview.SchemaName || review.ProjectId != source.ProjectId || review.SessionId != source.Id
                || !review.IncludeArtifacts || !WorkspaceSessionTransfer.IsRevision(review.Revision) || review.MessageCount < 0
                || review.Artifacts == null || review.MissingArtifacts == null || string.IsNullOrWhiteSpace(review.DefaultFilename)
                || Path.GetFileName(review.DefaultFilename) != review.DefaultFilename || !review.DefaultFilename.EndsWith(".zip", StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("Export preview does not match the selected conversation");
            if (Closed || !isCurrent()) return null;
            var path = await pick(review.DefaultFilename);
            if (Closed || !isCurrent() || string.IsNullOrEmpty(path)) return null;
            if (!Path.IsPathFullyQualified(path) || !path.EndsWith(".zip", StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("Choose an absolute ZIP archive destination");
            try
            {
                var result = (await client.InvokeAsync("native_conversation_export", new()
                { ["session_id"] = source.Id, ["include_artifacts"] = true, ["revision"] = review.Revision, ["destination_path"] = path }, source.ProjectId))
                    ?.Deserialize<NativeSessionExportResult>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing export acknowledgement");
                if (result.Schema != review.Schema || result.ProjectId != source.ProjectId || result.SessionId != source.Id
                    || result.Revision != review.Revision || !result.IncludeArtifacts || result.DestinationPath != path
                    || !WorkspaceSessionTransfer.IsRevision(result.Checksum) || result.Bytes == 0)
                    throw new InvalidDataException("Export acknowledgement does not match the requested archive");
                return result;
            }
            catch { MutationUnconfirmed = true; throw; }
        }, value => Result = value);
    }
}
