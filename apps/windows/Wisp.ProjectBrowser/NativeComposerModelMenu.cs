using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record ComposerModelMenuChoice(string Id, string Label, string Detail, string Tooltip, bool Acp, bool Selected, bool SupportsVision = false);

public static class NativeComposerModelMenu
{
    public static ComposerModelMenuChoice[] Choices(JsonObject[] profiles, ConversationModelOption[] agents, string? selected)
    {
        static string Text(JsonObject row, string key) => row[key]?.GetValue<string>() ?? "";
        var chat = profiles.Where(NativeModelPresentation.IsChat).ToArray();
        var http = chat.Select(row =>
        {
            var label = NativeModelPresentation.Label(row);
            var detail = Text(row, "provider") + " · " + Text(row, "model");
            var duplicate = chat.Count(other => NativeModelPresentation.Label(other).Trim().Equals(label.Trim(), StringComparison.OrdinalIgnoreCase)) > 1;
            var id = Text(row, "id");
            return new ComposerModelMenuChoice(id, label, duplicate ? detail : "", label + "\n" + detail, false, id == selected,
                row["supports_vision"]?.GetValue<bool>() == true);
        });
        return http.Concat(agents.Select(agent => new ComposerModelMenuChoice("acp:" + agent.Id, agent.Label,
            "ACP · local stdio", agent.Label, true, "acp:" + agent.Id == selected))).ToArray();
    }

    public static bool ShowMode(NativeAcpSessionState? state) => state is { HasModeConfig: false, CurrentMode: not null };
    public static string ModeLabel(NativeAcpSessionState state) => state.ModeChoices.FirstOrDefault(c => c.Id == state.CurrentMode)?.Label ?? state.CurrentMode ?? "";
}
