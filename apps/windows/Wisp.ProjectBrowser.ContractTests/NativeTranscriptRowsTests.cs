using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeTranscriptRowsTests
{
    public static void Run()
    {
        ConversationItem Row(string role, string text) => new(role, text, null, null, null, null);
        var snapshot = new ConversationSnapshot(ConversationSnapshot.SchemaId, "epoch", 1, "project", "session",
            [Row("user", "重复问题"), Row("tool", "结果"), Row("assistant", "回答")], null, false, false, false, "model", null, null, [], 2);
        var first = NativeTranscriptRows.Keys(snapshot);
        var polling = NativeTranscriptRows.Keys(snapshot with { Sequence = 20 });
        Check(first.SequenceEqual(polling), "polling must not change row identity");
        var streaming = NativeTranscriptRows.Keys(snapshot with { Items = [snapshot.Items[0], snapshot.Items[1], Row("assistant", "回答继续")] });
        Check(first.SequenceEqual(streaming), "streaming changes content, not identity");
        var prepend = NativeTranscriptRows.Keys(snapshot with { UserOffset = 1,
            Items = [Row("user", "重复问题"), Row("assistant", "先前回答"), .. snapshot.Items] });
        Check(first.SequenceEqual(prepend.Skip(2)), "history prepending preserves canonical turn identity");
        Check(prepend.Distinct().Count() == prepend.Count, "identical prompts in different turns remain distinct");
        Check(!first.Intersect(NativeTranscriptRows.Keys(snapshot with { SessionId = "other" })).Any(), "session isolation");
        Check(!first.Intersect(NativeTranscriptRows.Keys(snapshot with { Epoch = "restarted" })).Any(), "host restart invalidates view identity");
        Check(!NativeTranscriptRows.Keys(snapshot with { UserOffset = null, NextBeforeSeq = 10 }).Intersect(
            NativeTranscriptRows.Keys(snapshot with { UserOffset = null, NextBeforeSeq = 20 })).Any(), "legacy history pages cannot share guessed identities");
        Console.WriteLine("Native transcript row identity across polling, streaming, repeated prompts, history and host restart passed.");
    }

    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
}
