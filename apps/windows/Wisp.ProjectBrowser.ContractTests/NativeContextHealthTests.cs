using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeContextHealthTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private static NativeContextState State(params ulong[] after) => new((ulong)after.Length,
        after.Select((value, i) => new NativeContextEpoch((ulong)i + 1, (ulong)i, "manual", "regular", 1000, value, 0, null, null, null, false)).ToArray(), null, [], []);
    public static void Run()
    {
        var low = new NativeContextUsage(200, 1000, [new("conversation", "Conversation", 200)]);
        var danger = low with { Used = 950 };
        Check(NativeContextHealth.Read(State(), danger) is { Level: "ok", Compactions: 0 }, "Danger alone does not invent compaction history.");
        Check(NativeContextHealth.Read(State(), danger).ShowNudge(danger), "Danger still offers a new session before the first compaction.");
        Check(NativeContextHealth.Read(State(950), low).Level == "ok", "One low-yield compaction is not a trend.");
        Check(NativeContextHealth.Read(State(200), danger).Level == "split-now", "Danger after compaction warrants splitting.");
        Check(NativeContextHealth.Read(State(200, 800), low).Level == "split-soon", "Less than 25 percent freed.");
        Check(NativeContextHealth.Read(State(200, 650), low).Level == "split-soon", "Relative drop below half the previous yield.");
        Check(NativeContextHealth.Read(State(200, 900, 880), low).Level == "split-now", "Two consecutive yields under 15 percent.");
        Check(NativeContextHealth.Read(State(200, 800), low with { Used = 600, Rows = [new("conversation", "", 500)] }).Level == "split-now", "Diminishing yield with half the window occupied by observed conversation.");
        Check(NativeContextHealth.Read(State(200, 800), low with { Used = 600, Rows = [new("conversation", "", 600)], HasBreakdown = false }).Level == "split-soon", "Synthetic fallback is not measured conversation pressure.");
        Check(NativeContextHealth.Read(State(Enumerable.Repeat(280UL, 12).ToArray()), low).Level == "ok", "Count alone never warns.");
        Check(NativeContextHealth.Read(State(100, 450), low).Level == "ok", "A large release is healthy despite relative drop.");
        Check(NativeContextHealth.Read(State(200, 750), low).Level == "split-soon", "Exactly 25 percent still qualifies via relative drop.");
        Check(NativeContextHealth.Read(State(500, 750), low).Level == "ok", "Strict ratio and absolute boundaries.");
        var fork = State(200, 950, 300); fork = fork with { ContextEpochs = fork.ContextEpochs.Select(e => e.Epoch == 3 ? e with { ParentEpoch = 1 } : e).ToArray(), UndoneEpochs = [2] };
        Check(NativeContextHealth.Read(fork, low) is { Level: "ok", Compactions: 2 }, "Only the live head chain is counted.");
        var cycle = State(200, 300); cycle = cycle with { ContextEpochs = cycle.ContextEpochs.Select(e => e with { ParentEpoch = e.Epoch == 1 ? 2UL : 1UL }).ToArray() };
        Check(NativeContextHealth.Read(cycle, low).Compactions == 2, "Malformed cycles terminate.");
        var empty = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>());
        Check(NativeContextHealth.Read(State(), low).Line(empty) == null, "No history has no health line.");
        ConversationItem Usage(string json) => new("usage", json, null, null, null, null);
        Check(NativeContextUsage.Latest([Usage("{\"ctx_tokens\":600,\"max_context\":1000}")])?.HasBreakdown == false, "Missing breakdown remains explicit.");
        Check(NativeContextUsage.Latest([Usage("{\"ctx_tokens\":600,\"max_context\":1000,\"context_usage\":{}}")])?.HasBreakdown == false, "Empty bucket object also uses a synthetic fallback.");
    }
}
