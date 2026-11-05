using System.Text.Json;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record HomeActivityCount(string ProjectId, long Running, long NeedsYou);
public sealed record HomeActivitySnapshot(string Revision, PrivacyMode Privacy, HomeActivityCount[] Projects, BrowserSession[] Recent);

public static class NativeHomeActivityClient
{
    public const string Command = "native_project_home_activity";
    public static bool Supported(JsonNode? capabilities) => capabilities?["projects"] is JsonArray commands &&
        commands.Any(command => command?.ToString() == Command);

    public static async Task<HomeActivitySnapshot?> ReadAsync(INativeSettingsClient host, string? revision, CancellationToken token)
    {
        var args = new JsonObject();
        if (revision != null) args["known_revision"] = revision;
        var value = await host.InvokeAsync(Command, args, cancellationToken: token).ConfigureAwait(false);
        if (value == null && revision != null) return null; // host confirmed no transition
        if (value?.Deserialize<HomeActivitySnapshot>(ConversationSnapshot.JsonOptions) is not
            { Revision.Length: > 0, Privacy.ProjectIds: not null, Projects: not null, Recent: { Length: <= 5 } } snap
            || snap.Projects.Any(p => p == null || string.IsNullOrEmpty(p.ProjectId) || p.Running < 0 || p.NeedsYou < 0)
            || snap.Projects.DistinctBy(p => p.ProjectId).Count() != snap.Projects.Length
            || snap.Recent.Any(s => s == null || string.IsNullOrEmpty(s.Id) || string.IsNullOrEmpty(s.ProjectId) ||
                s.Title == null || s.Status is not ("running" or "needs_you" or "complete"))
            || snap.Recent.DistinctBy(s => (s.ProjectId,s.Id)).Count() != snap.Recent.Length)
            throw new InvalidDataException("Invalid home activity snapshot.");
        return snap;
    }
}
