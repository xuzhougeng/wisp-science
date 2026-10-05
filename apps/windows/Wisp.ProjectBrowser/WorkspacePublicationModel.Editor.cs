using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record PublicationItemDraft(string? Id = null, string Kind = "figure", string Title = "", string Content = "", string? ParentId = null, long Ordinal = 0);

public sealed partial class WorkspacePublicationModel
{
    public string? PublicationId => Workspace?.Publication?.Id;
    public string? RevisionId => Workspace?.Revision?.Id;
    public bool Editable => Workspace?.Revision?.State == "draft" && !Busy && !Closed && !MutationUncertain;
    public bool CanBuildCapsule => Workspace?.Revision is { State: "frozen" or "published", ManifestSha256.Length: > 0 }
        && !Busy && !Closed && !MutationUncertain;
    public bool MutationUncertain { get; private set; }
    public bool Reconciled { get; private set; }
    public PublicationItemDraft ItemDraft { get; set; } = new();
    public string? SelectedItem { get; private set; }
    public JsonObject? Readiness { get; private set; }
    public NativePublicationSources Sources { get; private set; } = new([], false);
    public NativePublicationSource? Source { get; private set; }
    public bool SourcesLoading { get; private set; }
    public string? SourcesError { get; private set; }
    private int sourceGeneration;
    private bool preciseSource;
    private readonly Dictionary<string, PublicationItemDraft> drafts = [];
    private readonly Dictionary<(string Revision, string Item), PublicationItemDraft> itemDrafts = [];
    private readonly Dictionary<string, string> pendingNewItems = [];

    private void RetainItemDraft()
    {
        if (RevisionId is { } revision) {
            var pendingNew = pendingNewItems.TryGetValue(revision, out var pending) && pending == ItemDraft.Id;
            itemDrafts[(revision, pendingNew ? "" : ItemDraft.Id ?? SelectedItem ?? "")] = ItemDraft;
        }
    }

    private void Apply(NativePublicationWorkspace value)
    {
        var previous = RevisionId;
        if (previous != value.Revision?.Id)
        {
            if (previous != null) { RetainItemDraft(); drafts[previous] = ItemDraft; }
            ItemDraft = value.Revision == null ? new() : drafts.GetValueOrDefault(value.Revision.Id) ?? new();
            SelectedItem = ItemDraft.Id; Source = null; preciseSource = false; Sources = new([], false); sourceGeneration++; SourcesLoading = false; SourcesError = null;
        }
        Workspace = value; Readiness = value.Readiness;
        if (RevisionId is { } current && pendingNewItems.TryGetValue(current, out var pendingId)
            && value.Items.Any(i => i.Id == pendingId)) {
            // A read can confirm that a lost creation reply did reach the store.
            // Preserve local edits under that item, but stop offering it as new.
            if (itemDrafts.Remove((current, ""), out var pendingDraft)) itemDrafts[(current, pendingId)] = pendingDraft;
            pendingNewItems.Remove(current);
        }
        if (SelectedItem != null && SelectedItem != ItemDraft.Id && !value.Items.Any(i => i.Id == SelectedItem)) SelectedItem = null;
    }
    private async Task<bool> CreateConfirmedAsync()
    {
        var ok = await RunAsync(() => client.CreateAsync(projectId, Title.Trim(), Description, RevisionLabel.Trim()), value =>
        { Apply(value); Title = Description = RevisionLabel = ""; });
        if (!ok && !Closed) { MutationUncertain = true; Reconciled = false; Notify(); }
        return ok;
    }
    public Task<bool> SelectWorkspaceAsync(string? publication, string? revision)
    {
        if (Busy || Closed) return Task.FromResult(false);
        return RunAsync(() => client.SelectAsync(projectId, publication, revision), Apply);
    }
    public void AcknowledgeUncertain()
    {
        if (!Busy && Reconciled) { MutationUncertain = false; Notify(); }
    }
    public void SelectItem(string? id)
    {
        if (Busy || Closed) return;
        RetainItemDraft();
        SelectedItem = id;
        ItemDraft = RevisionId is { } revision && itemDrafts.TryGetValue((revision, id ?? ""), out var retained) ? retained
            : Workspace?.Items.FirstOrDefault(i => i.Id == id) is { } item
            ? new(item.Id, item.Kind, item.Title, item.Content, item.ParentItemId, item.Ordinal)
            : new(Ordinal: (Workspace?.Items.Count ?? 0));
        SelectedItem = ItemDraft.Id;
        Notify();
    }
    public async Task<bool> SaveItemAsync()
    {
        if (!Editable) return false;
        if (string.IsNullOrWhiteSpace(ItemDraft.Title)) { Fail("请填写条目标题。"); return false; }
        if (ItemDraft.Ordinal < 0) { Fail("排序必须是非负整数。"); return false; }
        var newItem = ItemDraft.Id == null;
        var d = ItemDraft = ItemDraft with { Id = ItemDraft.Id ?? Guid.NewGuid().ToString() };
        SelectedItem = d.Id;
        // Keep a not-yet-confirmed creation reachable through New item, with the
        // same identity even when the reply is lost and the editor is reopened.
        if (newItem && RevisionId is { } currentRevision) pendingNewItems[currentRevision] = d.Id;
        RetainItemDraft();
        var ok = await MutateAsync(new() { ["action"] = "save_item", ["id"] = d.Id, ["kind"] = d.Kind, ["title"] = d.Title,
            ["content"] = d.Content, ["parent_item_id"] = d.ParentId, ["ordinal"] = d.Ordinal });
        if (ok && RevisionId is { } revision) {
            itemDrafts.Remove((revision, d.Id));
            if (pendingNewItems.TryGetValue(revision, out var pending) && pending == d.Id) {
                pendingNewItems.Remove(revision); itemDrafts.Remove((revision, ""));
            }
        }
        return ok;
    }
    public async Task<bool> MutateAsync(JsonObject operation)
    {
        if (Busy || Closed || MutationUncertain || RevisionId is not { } revision) return false;
        var action = operation["action"]?.GetValue<string>();
        if (action is "save_item" or "bind_evidence" or "update_binding" or "save_waiver" or "check" or "freeze" && !Editable) return false;
        if (action is "verify" or "build_capsule" && !CanBuildCapsule) return false;
        var publication = PublicationId;
        var payload = (JsonObject)operation.DeepClone();
        var ok = await RunAsync(() => client.MutateAsync(projectId, revision, payload), result =>
        {
            if (result.Workspace.Publication?.Id != publication) throw new InvalidDataException("Publication mutation returned a different paper");
            Apply(result.Workspace); Readiness = result.Readiness ?? result.Workspace.Readiness;
        });
        if (!ok && !Closed) { MutationUncertain = true; Reconciled = false; Notify(); }
        return ok;
    }
    public async Task LoadSourcesAsync(string kind, string query, uint offset)
    {
        var current = ++sourceGeneration; var revision = RevisionId;
        if (Closed) return;
        SourcesLoading = true; SourcesError = null; Source = null; preciseSource = false; Sources = new([], false); Notify();
        try
        {
            var page = await client.SourcesAsync(projectId, kind, query, offset);
            if (!Closed && current == sourceGeneration && RevisionId == revision) Sources = page;
        }
        catch (Exception e) { if (!Closed && current == sourceGeneration) SourcesError = e.Message; }
        finally { if (!Closed && current == sourceGeneration) { SourcesLoading = false; Notify(); } }
    }
    public void SelectSource(NativePublicationSource source)
    {
        if (!Closed && !SourcesLoading && Sources.Sources.Contains(source)) { Source = source; preciseSource = false; Notify(); }
    }
    public void ClearSource()
    {
        if (Closed || Busy) return;
        Source = null; preciseSource = false; Notify();
    }
    public void SelectPreciseSource(string kind, string frame, string sequence, string start, string end, string call, string exact)
    {
        if (!Editable) return;
        var source = NativePublicationEvidence.Precise(kind, frame, sequence, start, end, call, exact);
        sourceGeneration++; SourcesLoading = false; SourcesError = null;
        Source = source; preciseSource = true; Notify();
    }
    public static string MessageLocator(NativePublicationSource source, int start, int length)
    {
        var text = source.Text ?? throw new InvalidDataException("消息内容不可用。");
        var end = checked(start + length);
        bool Boundary(int i) => i >= 0 && i <= text.Length && !(i > 0 && i < text.Length && char.IsHighSurrogate(text[i - 1]) && char.IsLowSurrogate(text[i]));
        if (source.Kind != "message_span" || string.IsNullOrEmpty(source.FrameId) || source.MessageSeq == null
            || length <= 0 || !Boundary(start) || !Boundary(end)) throw new InvalidDataException("请选择完整的消息片段。");
        var fields = new SortedDictionary<string, object?> {
            ["byte_start"] = Encoding.UTF8.GetByteCount(text.AsSpan(0, start)), ["byte_end"] = Encoding.UTF8.GetByteCount(text.AsSpan(0, end)),
            ["frame_id"] = source.FrameId, ["message_seq"] = source.MessageSeq
        };
        if (source.TextSha256 != null) fields["message_content_sha256"] = source.TextSha256;
        return JsonSerializer.Serialize(fields);
    }
    public Task<bool> BindSourceAsync(string? item, string purpose, string? claim, string selection, string visibility, int start = 0, int length = 0)
    {
        if (!Editable || Source is not { } source || !preciseSource && !Sources.Sources.Contains(source)) return Task.FromResult(false);
        var id = source.Kind == "message_span" && !preciseSource ? MessageLocator(source, length == 0 ? 0 : start, length == 0 ? source.Text?.Length ?? 0 : length) : source.Id;
        return MutateAsync(new() { ["action"] = "bind_evidence", ["item_id"] = item, ["purpose"] = purpose,
            ["supported_claim_item_id"] = claim, ["selection_state"] = selection, ["visibility"] = visibility,
            ["source_kind"] = source.Kind, ["source_id"] = id });
    }
    public override void Dispose() { sourceGeneration++; base.Dispose(); }
}
