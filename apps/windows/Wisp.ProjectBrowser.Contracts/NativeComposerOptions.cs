using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser.Contracts;

// The session shape is defined in wisp-dto::native_conversations::ComposerOptions
// and checked against the shared composer-options.json fixture.
public sealed record ComposerCompletion(string Policy, bool AutoResume);
public sealed record ComposerSessionOptions(string SessionId, bool FullPermission, bool Delegation,
    ComposerCompletion Completion, bool AutoReview, JsonObject? Specialist, bool SpecialistLocked);
public sealed record ComposerFailureAnalysis(bool Enabled, int FailureRateThreshold, int MinimumFailures);
public sealed record ComposerOptionChoice(string Id, string Label);
