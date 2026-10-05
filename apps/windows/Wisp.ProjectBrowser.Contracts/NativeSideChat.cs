using System.Text.Json;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativeSideChatEvidence(string SourceId, long? EventSeq, long? MessageSeq, int Turn, string Role, string Excerpt, string Relevance);
public sealed record NativeSideChatResponse(string? SessionId, string Answer, long SnapshotVersion, NativeSideChatEvidence[] Evidence, bool NoEvidence);
public sealed record NativeSideChatOption(string Id, string Label, string Kind, bool Active)
{
    public string Key => Kind + ":" + Id;
}
public sealed record NativeSideChatQuote(string Text, string Source = "")
{
    public static string Question(string text, IEnumerable<NativeSideChatQuote> quotes)
    {
        var parts = quotes.Select(quote => {
            var source = quote.Source.Replace('\r', ' ').Replace('\n', ' ').Replace("`", "\\`").Trim();
            var block = source.Length == 0 ? "" : $"Selected excerpt from reference `{source}`:\n";
            return block + string.Join("\n", quote.Text.Trim().Split('\n').Select(line => "> " + line));
        }).ToList();
        parts.Add(text.Trim());
        return string.Join("\n\n", parts).Trim();
    }
}
public interface INativeSideChatClient
{
    Task<NativeSideChatOption[]> OptionsAsync(string project, string session, CancellationToken token = default);
    Task SelectHttpModelAsync(string project, string modelId, CancellationToken token = default);
    Task<NativeSideChatResponse> AskAsync(string project, string session, string question, string? acpAgentId = null, CancellationToken token = default);
}
public sealed class NativeSideChatClient(INativeSettingsClient transport) : INativeSideChatClient
{
    private static readonly JsonSerializerOptions Camel = new() { PropertyNamingPolicy = JsonNamingPolicy.CamelCase };
    public async Task<NativeSideChatOption[]> OptionsAsync(string project, string session, CancellationToken token = default) =>
        (await transport.InvokeAsync("native_conversation_panel_side_chat_options", new JsonObject { ["session_id"] = session }, project, token).ConfigureAwait(false))?.Deserialize<NativeSideChatOption[]>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing side-chat options");
    public async Task SelectHttpModelAsync(string project, string modelId, CancellationToken token = default) =>
        _ = await transport.InvokeAsync("set_active_model", new JsonObject { ["id"] = modelId }, project, token).ConfigureAwait(false);
    public async Task<NativeSideChatResponse> AskAsync(string project, string session, string question, string? acpAgentId = null, CancellationToken token = default)
    {
        var value = await transport.InvokeAsync("native_conversation_panel_side_chat", new JsonObject { ["session_id"] = session, ["question"] = question, ["acp_agent_id"] = acpAgentId }, project, token).ConfigureAwait(false);
        var response = value?.Deserialize<NativeSideChatResponse>(Camel) ?? throw new InvalidDataException("Missing side-chat reply");
        if (response.SessionId != session) throw new InvalidDataException("Side-chat response scope mismatch");
        return response;
    }
}

public enum NativeSideChatReturnAction { Send, Newline, Composition }
public static class NativeSideChatKeyboard
{
    /// Use the WinUI input method's composition state before deciding whether Return sends.
    public static NativeSideChatReturnAction ResolveReturn(bool shift, bool composing, bool control = false, bool sendWithModifier = false) =>
        composing ? NativeSideChatReturnAction.Composition : shift || sendWithModifier && !control
            ? NativeSideChatReturnAction.Newline : NativeSideChatReturnAction.Send;
}
