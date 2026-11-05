using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;

internal static class NativeAcpQuestionTests
{
    public static void Run()
    {
        static NativeAcpQuestion Parse(string json) => NativeAcpQuestion.Parse(JsonNode.Parse(json)!.AsObject())!;
        static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
        var restricted = Parse("""{"request_id":"q","question":"Use **paired** samples?","allow_freeform":false,"options":[{"label":"Paired","description":"Keep donor identities."},{"label":"Independent"}]}""");
        Check(!restricted.AllowsFreeform && restricted.Choices.Length == 2, "explicit choices-only questions hide freeform input");
        Check(restricted.Choices[0] == new NativeAcpQuestion.Choice("Paired", "Keep donor identities.")
            && restricted.Choices[1].Description == "", "choice labels remain literal answers while descriptions are display-only and default empty");
        Check(restricted.Pending(["q"]) && !restricted.Pending(["other"]), "only authoritative live request IDs remain actionable");
        Check(Parse("""{"request_id":"q","allow_freeform":false,"options":[]} """).AllowsFreeform, "no-options question still permits an answer like WebView");
        Check(Parse("""{"request_id":"q","options":[{}]}""").AllowsFreeform, "missing freeform flag defaults on");
        Check(Parse("""{"request_id":"q","allow_freeform":false,"options":[{"label":42}]}""").Choices.Length == 0,
            "invalid foreign options degrade as the shared Rust parser does");
        var answered = Parse("""{"request_id":"q","status":"answered"}""");
        Check(!answered.Pending(["q"]) && answered.NoteKey == "plan.answer_sent", "answered snapshot cannot expose stale buttons while live IDs catch up");
        var expired = Parse("""{"request_id":"q","status":"expired"}""");
        Check(!expired.Pending(["q"]) && expired.NoteKey == "plan.question.expired", "expired copy uses shared locale key and cannot submit");
        Check(restricted.DraftKey("p", "s") != restricted.DraftKey("p", "other")
            && restricted.DraftKey("p", "s") != restricted.DraftKey("other", "s"), "reused protocol request IDs cannot leak drafts across projects or sessions");
        Check(NativeAcpQuestion.Parse(JsonNode.Parse("""{"request_id":""}""")!.AsObject()) == null,
            "empty request IDs remain ordinary questions");
        Console.WriteLine("Native ACP question presentation: freeform policy, literal options, lifecycle and draft scope passed.");
    }
}
