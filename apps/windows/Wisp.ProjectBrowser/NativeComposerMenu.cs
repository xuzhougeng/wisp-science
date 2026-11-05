using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public static class NativeComposerMenu
{
    public sealed record Item(string Id, string LabelKey, bool Indent = false);
    public sealed record Group(string LabelKey, Item[] Items);
    public static double Width(double viewportWidth) => double.IsFinite(viewportWidth) ? Math.Clamp(viewportWidth - 32, 0, 320) : 320;
    public static IReadOnlyList<Group> Groups { get; } = [
        new("composer.group.mode", [new("plan", "composer.plan_first"), new("full_permission", "composer.full_permission")]),
        new("composer.group.agent", [new("specialist", "composer.specialist"), new("auto_review", "composer.auto_review"), new("reviewer", "composer.reviewer_model", true)]),
        new("composer.group.resources", [new("compute", "composer.compute")])
    ];

    public static string ReviewerLabel(ComposerOptionChoice choice, NativeLocalization locale) => choice.Id switch
    {
        "http:" => locale.Text("composer.reviewer.default_http"),
        "follow_session" => locale.Text("composer.reviewer.follow_session"),
        _ => choice.Label
    };
    public static string SpecialistLabel(ComposerOptionChoice choice, NativeLocalization locale) =>
        choice.Id.Length == 0 ? locale.Text("composer.specialist.none") : choice.Label;

    public static string ComputeSummary(string? defaultId, string? label, int remoteCount, NativeLocalization locale)
    {
        var id = defaultId?.Trim();
        return !string.IsNullOrEmpty(id) && id != "local"
            ? locale.Format("compute.default_named", ("name", string.IsNullOrWhiteSpace(label) ? id : label.Trim()))
            : remoteCount == 0 ? locale.Text("compute.default_local") : locale.Format("composer.compute_count", ("n", remoteCount.ToString()));
    }

    // Match the WebView's ACP plan-mode adapter. Protocol IDs, not display names,
    // determine both the plan target and the default/first non-plan return mode.
    public static bool IsPlan(string id) => id.Contains("plan", StringComparison.OrdinalIgnoreCase);
    public static (string Plan, string Exit)? PlanPair(NativeAcpSessionState? state)
    {
        var ids = state?.ModeChoices.Select(choice => choice.Id).ToArray() ?? [];
        var plan = ids.FirstOrDefault(IsPlan);
        var exit = ids.FirstOrDefault(id => id == "default") ?? ids.FirstOrDefault(id => !IsPlan(id));
        return plan != null && exit != null ? (plan, exit) : null;
    }
}
