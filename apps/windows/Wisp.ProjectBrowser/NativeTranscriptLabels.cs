using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeTranscriptActionLabels(string Copy, string AddToChat, string SideChat, string Highlight);

public static class NativeTranscriptLabels
{
    public static NativeTranscriptActionLabels Actions(NativeLocalization locale) => new(
        locale.Text("msg.copy"), locale.Text("selection.add_to_chat"),
        locale.Text("selection.quote_side_chat"), locale.Text("selection.highlight"));

    public static string Role(ConversationItem item, NativeLocalization locale) => item.Role switch
    {
        "user" => locale.Text("branch.role_user"),
        "reasoning" => locale.Text("share.role_thinking"),
        "tool" when NativeTranscriptActivity.Completion(item) => "Wisp Science",
        "tool" or "acp_tool" => item.ToolName ?? locale.Text("branch.role_tool"),
        _ => "Wisp Science"
    };
}
