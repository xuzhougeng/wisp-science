using System.Text.Json;
using System.Text.Json.Nodes;
namespace Wisp.ProjectBrowser.Contracts;
public sealed record NativeShareRow(string Role, string Text);
public sealed record NativeShareDraftRow(int Id, NativeShareRow Row, bool Selected);
public static class NativeShareDraft
{
    public static NativeShareDraftRow[] From(IEnumerable<NativeShareRow> rows) => rows.Select((row, index) => (row, index))
        .Where(pair => pair.row.Role is "user" or "assistant" or "reasoning" && !string.IsNullOrWhiteSpace(pair.row.Text))
        .Select(pair => new NativeShareDraftRow(pair.index, pair.row, pair.row.Role != "reasoning")).ToArray();
    public static string[] Keywords(string raw) => raw.Split(',', '，', '\n')
        .Select(word => word.Trim()).Where(word => word.Length > 0).Distinct().OrderByDescending(word => word.Length).ThenBy(word => word).ToArray();
    public static string Redact(string text, IEnumerable<string> keywords) =>
        keywords.Aggregate(text, (current, word) => current.Replace(word, "xxx", StringComparison.OrdinalIgnoreCase));
    public static int Width(string raw) => uint.TryParse(raw.Trim(), out var value) ? (int)Math.Clamp(value, 320u, 2400u) : 840;
    public static NativeShareRow[] Selected(IEnumerable<NativeShareDraftRow> draft, string keywords)
    {
        var words = Keywords(keywords);
        return draft.Where(row => row.Selected).Select(row => row.Row with { Text = Redact(row.Row.Text, words) }).ToArray();
    }
}
/// <summary>Send only the user's selected, edited and redacted rows to HtmlAsync.
/// PNG rendering and file dialogs belong to the native frontend.</summary>
public interface INativeShareClient
{
    Task<NativeShareRow[]> ReadAsync(string projectId, string sessionId, CancellationToken cancellationToken = default);
    Task<string> HtmlAsync(string projectId, string sessionId, NativeShareRow[] rows, bool dark = false, CancellationToken cancellationToken = default);
}
public sealed class NativeShareClient(INativeSettingsClient transport) : INativeShareClient
{
    public async Task<NativeShareRow[]> ReadAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) =>
        (await transport.InvokeAsync("native_conversation_share", new() { ["session_id"] = sessionId }, projectId, cancellationToken).ConfigureAwait(false))?.Deserialize<NativeShareRow[]>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing share rows");
    public async Task<string> HtmlAsync(string projectId, string sessionId, NativeShareRow[] rows, bool dark = false, CancellationToken cancellationToken = default) =>
        (await transport.InvokeAsync("native_conversation_share_html", new() { ["session_id"] = sessionId, ["rows"] = JsonSerializer.SerializeToNode(rows, ConversationSnapshot.JsonOptions), ["dark"] = dark }, projectId, cancellationToken).ConfigureAwait(false))?.GetValue<string>()
            ?? throw new InvalidDataException("Missing share document");
}
