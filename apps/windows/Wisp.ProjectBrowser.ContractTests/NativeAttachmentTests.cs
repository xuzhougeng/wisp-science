using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeAttachmentTests
{
    public static async Task RunAsync()
    {
        foreach (var path in new[] { "uploads/../secret.png", "uploads/a\\b", "uploads//x", "uploads/C:/x", "uploads/a\nb" })
            Check(!NativeAttachmentPresentation.ValidPath(path), "unsafe staged attachment rejected");
        Check(NativeAttachmentPresentation.Image("uploads/图.JPG") && !NativeAttachmentPresentation.Image("uploads/data.csv"), "WebView image extensions");
        var host = new Host(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "s");
        host.Upload = new();
        var adding = model.AttachAsync("C:/sample/figure.png");
        Check(model.Busy && model.AttachmentUploads.Single() is { Name: "figure.png", Uploading: true }
            && !model.CanSend && !model.CanQueue, "visible pending upload blocks sending");
        host.Upload.SetResult(JsonSerializer.SerializeToNode(new ComposerAttachment("uploads/figure.png", "figure.png"), ConversationSnapshot.JsonOptions));
        Check(await adding && model.AttachmentUploads.Length == 0 && model.Attachments.Length == 1, "confirmed upload replaces pending state");
        host.Upload = new(); adding = model.AttachAsync("C:/sample/large.csv");
        host.Upload.SetException(new IOException("exceeds byte limit"));
        Check(!await adding && model.AttachmentUploads.Single().MetaKey == "composer.upload_too_large"
            && model.Attachments.Length == 1 && !model.Busy && model.HasInlineAttachmentError && model.ShowWelcome, "failure preserves accepted files and identifies size limit without replacing welcome");
        model.DismissAttachmentUpload(model.AttachmentUploads.Single().Id);
        Check(model.AttachmentUploads.Length == 0 && model.OperationError == null && host.UploadCalls == 2, "dismissal clears its inline error and never retries copying");
        Check(await model.ReadAttachmentAsync("uploads/foreign.png") == null && host.ReadCalls == 0, "unreferenced attachments cannot be previewed");
        host.Read = new();
        var first = model.ReadAttachmentAsync("uploads/figure.png"); var second = model.ReadAttachmentAsync("uploads/figure.png");
        host.Read.SetResult(Preview("p", "s"));
        Check(await first != null && await second != null && host.ReadCalls == 1 && host.UploadCalls == 2, "thumbnail and preview share one scoped read without copying");
        model.RemoveAttachment("uploads/figure.png");
        Check(!model.CanPreviewAttachment("uploads/figure.png"), "removed attachment cannot be opened");
        host.Upload = null; await model.AttachAsync("figure.png"); host.Read = new();
        var late = model.ReadAttachmentAsync("uploads/figure.png");
        await model.OpenAsync("other", "other-session"); host.Read.SetResult(Preview("p", "s"));
        Check(await late == null && model.AttachmentUploads.Length == 0, "late preview cannot cross navigation");
        host.Upload = new(); var staleUpload = model.AttachAsync("old.png");
        await model.OpenAsync("p", "s"); host.Upload.SetException(new IOException("late failure"));
        Check(!await staleUpload && model.AttachmentUploads.Length == 0 && model.OperationError == null, "late upload errors cannot alter the reopened scope");
        host.Items = [new("user", "Image\n\nUploaded files: uploads/figure.png", null, null, null, null),
            new("assistant", "Uploaded files: uploads/untrusted.png", null, null, null, null)];
        await model.OpenAsync("p", "recorded"); host.Read = new();
        Check(model.Attachments.Length == 0 && model.CanPreviewAttachment("uploads/figure.png")
            && !model.CanPreviewAttachment("uploads/untrusted.png"), "recorded user attachments can open without staging or accepting assistant text as uploads");
        var historical = model.ReadAttachmentAsync("uploads/figure.png");
        host.Read.SetResult(Preview("p", "recorded"));
        Check(await historical != null && host.UploadCalls == 4, "recorded image preview reads without uploading or modifying the draft");
        host.Items = []; await model.OpenAsync("p", "different");
        Check(!model.CanPreviewAttachment("uploads/figure.png"), "recorded attachment permission does not leak to another conversation");
        model.Pause();
    }
    private static JsonNode? Preview(string project, string session) => JsonSerializer.SerializeToNode(new NativeFilePreview(
        NativeFilesClient.SchemaId, project, session, "local", "uploads/figure.png", new("uploads/figure.png", "image/png", null, "AA==", false, 1)), ConversationSnapshot.JsonOptions);
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Host : INativeSettingsClient
    {
        public TaskCompletionSource<JsonNode?>? Upload, Read;
        public int UploadCalls, ReadCalls;
        public ConversationItem[] Items = [];
        private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "native_conversation_attach")
            { UploadCalls++; return Upload?.Task ?? Task.FromResult(JsonSerializer.SerializeToNode(new ComposerAttachment("uploads/figure.png", "figure.png"), ConversationSnapshot.JsonOptions)); }
            if (command == "native_conversation_panel_file_read") { ReadCalls++; return Read!.Task; }
            object? result = command switch {
                "native_conversation_snapshot" => new ConversationSnapshot(ConversationSnapshot.SchemaId, "host", ++sequence, projectId!, args["session_id"]!.GetValue<string>(), Items, null, false, false, false, "model", null, null, [], ComposerReferences: true),
                "list_models" => new JsonArray(), _ => null };
            return Task.FromResult(result is JsonNode node ? node : JsonSerializer.SerializeToNode(result, ConversationSnapshot.JsonOptions));
        }
    }
}
