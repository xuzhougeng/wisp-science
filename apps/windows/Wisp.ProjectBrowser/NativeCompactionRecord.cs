using System.Globalization;
using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Presentation of the existing compaction UiItem, enriched by native_transcript.
public sealed record NativeCompactionRecord(ulong Before, ulong After, string Strategy, ulong? Epoch,
    string? Checkpoint, int? KeptFromUserIndex, bool Undone, bool CanUndo, string? UndoReason)
{
    public bool AutomaticContinue => Strategy == "auto_continue";
    public string TitleKey => Undone ? "chat.compaction_undone" : Strategy == "auto" ? "chat.context_auto_compacted" : "chat.context_compacted";
    public string Counts => Tokens(Before) + " → " + Tokens(After) + " tokens";
    public string? Reduction => Before > After
        ? Math.Round((Before - After) * 100m / Before, 0, MidpointRounding.AwayFromZero).ToString(CultureInfo.InvariantCulture) : null;
    private static string Tokens(ulong value) => value < 1000 ? value.ToString(CultureInfo.InvariantCulture)
        : (value / 1000m).ToString("0.0", CultureInfo.InvariantCulture) + "k";
    public string? UndoReasonKey => UndoReason switch {
        null => null, "undone" => "chat.compaction_undo_reason_undone", "not_head" => "chat.compaction_undo_reason_not_head",
        _ => "chat.compaction_undo_reason_has_new_turns"
    };
    public static NativeCompactionRecord? Parse(ConversationItem row)
    {
        if (row.Role != "compaction") return null;
        try
        {
            using var json = JsonDocument.Parse(row.Text); var root = json.RootElement;
            if (root.ValueKind != JsonValueKind.Object || !root.TryGetProperty("before", out var before) || !before.TryGetUInt64(out var b)
                || !root.TryGetProperty("after", out var after) || !after.TryGetUInt64(out var a)) return null;
            string? Text(string key) => root.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.String ? value.GetString() : null;
            bool Flag(string key) => root.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.True;
            ulong? epoch = root.TryGetProperty("epoch", out var e) && e.ValueKind == JsonValueKind.Number && e.TryGetUInt64(out var number) && number > 0 ? number : null;
            int? kept = root.TryGetProperty("kept_from_user_index", out var k) && k.ValueKind == JsonValueKind.Number && k.TryGetInt32(out var n) && n >= 0 ? n : null;
            return new(b, a, Text("strategy") ?? "manual", epoch, Text("checkpoint"), kept, Flag("undone"), Flag("can_undo"), Text("undo_reason"));
        }
        catch (Exception ex) when (ex is JsonException or InvalidOperationException or FormatException) { return null; }
    }
    public static int? RewindRow(ConversationSnapshot page, int keptFrom)
    {
        if (page.UserOffset is not { } offset || keptFrom < offset) return null;
        for (var i = 0; i < page.Items.Length; i++)
            if (page.Items[i].Role == "user" && page.Items[i].Kind != "checkpoint" && offset++ == keptFrom) return i;
        return null;
    }
}

public sealed record NativeCompactionTarget(string ProjectId, string SessionId, string HostEpoch, ulong Epoch);
public sealed record NativeCompactionUndoOutcome(bool Confirmed = false, string? Error = null);
