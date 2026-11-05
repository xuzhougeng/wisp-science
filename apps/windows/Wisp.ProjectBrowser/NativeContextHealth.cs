using System.Globalization;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Same head-chain and freed-window thresholds as app_support/context_usage.rs.
public sealed record NativeContextHealth(string Level, int Compactions, double? LatestFreed)
{
    public string Tone => Level == "split-now" ? "err" : Level == "split-soon" ? "warn" : "text-muted";
    public string? Line(NativeLocalization locale) => Compactions == 0 ? null : locale.Format(
        LatestFreed == null ? "context_usage.health.line_no_ratio" : "context_usage.health.line",
        ("level", locale.Text("context_usage.health." + Level.Replace('-', '_'))),
        ("count", Compactions.ToString(CultureInfo.InvariantCulture)),
        ("freed", LatestFreed is { } freed ? Math.Round(freed * 100, MidpointRounding.AwayFromZero).ToString(CultureInfo.InvariantCulture) : ""));
    public bool ShowNudge(NativeContextUsage usage) => usage.Tone == "err" || Level != "ok";
    public string NudgeKey(NativeContextUsage usage) => usage.Tone == "err" ? "context_usage.nudge" : "context_usage.health.nudge";
    public static NativeContextHealth Read(NativeContextState? state, NativeContextUsage usage)
    {
        var chain = new List<NativeContextEpoch>(); var seen = new HashSet<ulong>();
        var cursor = state?.HeadEpoch ?? 0;
        while (cursor != 0 && seen.Add(cursor) && state?.ContextEpochs.FirstOrDefault(row => row.Epoch == cursor) is { } row)
        {
            if (state.UndoneEpochs.Contains(cursor)) break;
            chain.Add(row); cursor = row.ParentEpoch;
        }
        static double? Freed(NativeContextEpoch? epoch) => epoch is { BeforeTokens: > 0 }
            ? (double)(epoch.BeforeTokens - Math.Min(epoch.BeforeTokens, epoch.AfterTokens)) / epoch.BeforeTokens : null;
        var latest = Freed(chain.FirstOrDefault()); var previous = Freed(chain.Skip(1).FirstOrDefault());
        NativeContextHealth Report(string level) => new(level, chain.Count, latest);
        if (chain.Count == 0) return Report("ok");
        if (usage.Tone == "err") return Report("split-now");
        if (chain.Count < 2 || latest == null) return Report("ok");
        if (latest < .15 && previous < .15) return Report("split-now");
        var diminishing = latest < .25 || latest < .5 && latest < previous * .5;
        if (!diminishing) return Report("ok");
        var conversation = usage.Rows.FirstOrDefault(row => row.Key == "conversation")?.Tokens ?? 0;
        // A synthetic conversation-only fallback is not an observed breakdown.
        return Report(usage.HasBreakdown && usage.Max > 0 && (double)conversation / usage.Max >= .5 ? "split-now" : "split-soon");
    }
}
