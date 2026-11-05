using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeQueueDisplayRow(string Text, string State, NativeQueueItem? Item);

public static class NativeQueuePresentation
{
    public static (byte A, byte R, byte G, byte B) Mix((byte A, byte R, byte G, byte B) first, (byte A, byte R, byte G, byte B) second, double weight)
    {
        var front = first.A * weight; var back = second.A * (1 - weight); var alpha = front + back;
        byte Channel(byte a, byte b) => alpha == 0 ? (byte)0 : (byte)Math.Round((a * front + b * back) / alpha);
        return ((byte)Math.Round(alpha), Channel(first.R, second.R), Channel(first.G, second.G), Channel(first.B, second.B));
    }
    // Old hosts expose queued rows without actionable queue identities. Keep
    // those messages in the composer region, without inventing mutation targets.
    public static NativeQueueDisplayRow[] Rows(ConversationSnapshot? snapshot, string? legacyDraft)
    {
        if (snapshot?.Queue is { } queue) return queue.Items.Select(item => new NativeQueueDisplayRow(item.Message, item.State, item)).ToArray();
        var rows = snapshot?.Items.Where(item => item.Role == "queued_user")
            .Select(item => new NativeQueueDisplayRow(item.Text, "queued", null)).ToArray() ?? [];
        return rows.Length > 0 || string.IsNullOrEmpty(legacyDraft) ? rows : [new(legacyDraft, "queued", null)];
    }

    public static string RecoveryReason(string reason, NativeLocalization locale) => reason switch {
        "failed" => locale.Choose("排队执行失败，请检查会话后恢复草稿。", "The queued message failed. Check the conversation before restoring its draft."),
        "restarted" => locale.Choose("宿主已重启，排队结果未确认。请核对会话后恢复草稿。", "The host restarted before the queue result was confirmed. Check the conversation before restoring its draft."),
        "returned" => locale.Choose("已从队列取回；现有草稿保留，可在清空输入框后恢复。", "Returned from the queue. Your current draft is preserved; restore this message when the composer is empty."),
        _ => reason
    };
}
