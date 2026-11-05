using System.Text.Json;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser.Contracts;

// Projection of wisp_dto::ProjectMemo; private notes never enter agent context.
public sealed record NativeMemo(string Id, string ProjectId, string Title, string Body, bool Pinned, long CreatedAt, long UpdatedAt);
public sealed record NativeMemoInput(string Title, string Body);
public sealed record NativeMemoPatch(string Id, string? Title = null, string? Body = null, bool? Pinned = null);
public sealed record NativeMemoList(string Schema, string ProjectId, NativeMemo[] Memos);
public sealed record NativeMemoResult(string Schema, string ProjectId, NativeMemo Memo);
public sealed record NativeDeletedMemo(string Schema, string ProjectId, string MemoId);
public interface INativeMemosClient
{
    Task<NativeMemo[]> ListAsync(string project, CancellationToken token = default);
    Task<NativeMemo> AddAsync(string project, NativeMemoInput input, CancellationToken token = default);
    Task<NativeMemo> UpdateAsync(string project, NativeMemoPatch patch, CancellationToken token = default);
    Task DeleteAsync(string project, string id, CancellationToken token = default);
}
public sealed class NativeMemosClient(INativeSettingsClient transport) : INativeMemosClient
{
    public const string Schema = "wisp.native-memos.v1";
    private async Task<T> Call<T>(string command, string project, JsonObject args, CancellationToken token) where T : class
        => (await transport.InvokeAsync(command, args, project, token))?.Deserialize<T>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing memo response");
    private static void Scope(string schema, string owner, string project)
    { if (schema != Schema || owner != project) throw new InvalidDataException("Memo project scope mismatch"); }
    private static void MemoScope(NativeMemo memo, string project, string? id = null)
    {
        if (memo == null || memo.ProjectId != project || string.IsNullOrWhiteSpace(memo.Id) || id != null && memo.Id != id
            || memo.Title == null || memo.Body == null) throw new InvalidDataException("Memo identity mismatch");
    }
    public async Task<NativeMemo[]> ListAsync(string project, CancellationToken token = default)
    {
        var result = await Call<NativeMemoList>("native_project_memos", project, new(), token);
        Scope(result.Schema, result.ProjectId, project);
        if (result.Memos == null) throw new InvalidDataException("Missing memo list");
        foreach (var memo in result.Memos) MemoScope(memo, project);
        if (result.Memos.Select(m => m.Id).Distinct().Count() != result.Memos.Length) throw new InvalidDataException("Duplicate memo identity");
        return result.Memos;
    }
    public async Task<NativeMemo> AddAsync(string project, NativeMemoInput input, CancellationToken token = default)
    {
        var result = await Call<NativeMemoResult>("native_project_memo_add", project,
            new() { ["input"] = JsonSerializer.SerializeToNode(input, ConversationSnapshot.JsonOptions) }, token);
        Scope(result.Schema, result.ProjectId, project); MemoScope(result.Memo, project); return result.Memo;
    }
    public async Task<NativeMemo> UpdateAsync(string project, NativeMemoPatch patch, CancellationToken token = default)
    {
        var value = new JsonObject { ["id"] = patch.Id };
        if (patch.Title != null) value["title"] = patch.Title;
        if (patch.Body != null) value["body"] = patch.Body;
        if (patch.Pinned is { } pinned) value["pinned"] = pinned;
        var result = await Call<NativeMemoResult>("native_project_memo_update", project, new() { ["patch"] = value }, token);
        Scope(result.Schema, result.ProjectId, project); MemoScope(result.Memo, project, patch.Id); return result.Memo;
    }
    public async Task DeleteAsync(string project, string id, CancellationToken token = default)
    {
        var result = await Call<NativeDeletedMemo>("native_project_memo_delete", project, new() { ["memo_id"] = id }, token);
        Scope(result.Schema, result.ProjectId, project);
        if (result.MemoId != id) throw new InvalidDataException("Deleted memo identity mismatch");
    }
}
