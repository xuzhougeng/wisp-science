using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>View identity comes from the persisted user-turn offset, never the
/// snapshot's polling sequence or message text (repeated prompts are valid).</summary>
public static class NativeTranscriptRows
{
    public static IReadOnlyList<string> Keys(ConversationSnapshot snapshot)
    {
        var turn = (snapshot.UserOffset ?? 0) - 1;
        var ordinal = 0;
        var scope = $"{snapshot.ProjectId}/{snapshot.SessionId}/{snapshot.Epoch}/";
        // Older hosts do not expose turn offsets. Do not reuse rows across
        // different history pages when their identities cannot be established.
        if (snapshot.UserOffset is null) scope += $"page:{snapshot.NextBeforeSeq}/";
        var keys = new List<string>(snapshot.Items.Length);
        var occurrences = new Dictionary<string, int>(StringComparer.Ordinal);
        foreach (var item in snapshot.Items)
        {
            if (item.Role == "user") { turn++; ordinal = 0; }
            var slot = ordinal++;
            var key = item.Role == "acp_tool" && !string.IsNullOrEmpty(item.CallId)
                ? $"{scope}{turn}/acp:{item.CallId.Length}:{item.CallId}" : $"{scope}{turn}/{slot}/{item.Role}";
            var occurrence = occurrences.GetValueOrDefault(key);
            occurrences[key] = occurrence + 1;
            keys.Add($"{key}/occurrence:{occurrence}");
        }
        return keys;
    }
}
