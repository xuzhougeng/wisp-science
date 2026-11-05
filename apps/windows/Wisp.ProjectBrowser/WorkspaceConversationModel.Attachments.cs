using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspaceConversationModel
{
    public NativeAttachmentUpload[] AttachmentUploads { get; private set; } = [];
    private string? attachmentOperationError;
    public bool HasInlineAttachmentError => attachmentOperationError != null && OperationError == attachmentOperationError
        && AttachmentUploads.Any(item => !item.Uploading);
    private readonly Dictionary<string, Task<NativePanelFileContent>> attachmentReads = [];
    public void DismissAttachmentUpload(Guid id)
    {
        AttachmentUploads = AttachmentUploads.Where(item => item.Id != id).ToArray();
        if (!AttachmentUploads.Any(item => !item.Uploading) && OperationError == attachmentOperationError)
            OperationError = attachmentOperationError = null;
        Notify();
    }
    public bool CanPreviewAttachment(string path) => settings != null && active && sessionId != null
        && NativeAttachmentPresentation.ValidPath(path) && (Attachments.Any(file => file.Path == path)
            || (ShowingHistory ? History : Snapshot)?.Items.Any(item => item.Role == "user"
                && NativeMessagePresentation.User(item.Text, item.Attachments).Attachments.Contains(path, StringComparer.Ordinal)) == true);
    public async Task<NativePanelFileContent?> ReadAttachmentAsync(string path, CancellationToken token = default)
    {
        if (!CanPreviewAttachment(path) || token.IsCancellationRequested) return null;
        var revision = generation; var project = projectId!; var session = sessionId!;
        if (!attachmentReads.TryGetValue(path, out var reading))
        {
            // Cache only a small number of thumbnail/preview reads in this scope.
            if (attachmentReads.Count >= 4) attachmentReads.Remove(attachmentReads.Keys.First());
            attachmentReads[path] = reading = new NativeFilesClient(settings!).ReadAsync(project, session, "local", path, token);
        }
        try
        {
            var content = await reading;
            return revision == generation && CanPreviewAttachment(path) && !token.IsCancellationRequested ? content : null;
        }
        catch
        {
            if (revision == generation && attachmentReads.GetValueOrDefault(path) == reading) attachmentReads.Remove(path);
            throw;
        }
    }
}
