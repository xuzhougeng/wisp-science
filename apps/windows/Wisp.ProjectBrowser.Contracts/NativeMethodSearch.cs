using System.Text.Json;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

// Wire names follow the shared MethodSearchRunDetails DTO: run records and
// contract fields are snake_case; the outer detail and the audit use camelCase.
public sealed record NativeMethodDetail(NativeRun Run, NativeMethodState State, NativeMethodSpec Spec,
    NativeMethodAudit Audit, [property: JsonPropertyName("auditArtifactVersionId")] string AuditVersion,
    NativeMethodCandidate[] Candidates, NativeMethodStrategy[] Strategies, NativeMethodOutput[] Outputs)
{
    public NativeMethodDetail Verify(string runId, string context)
    {
        if (Run?.Id != runId || Run.ContextId != context || Run.Kind != "method_search" || State?.RunId != runId ||
            string.IsNullOrEmpty(State.SpecArtifactVersionId) || string.IsNullOrEmpty(AuditVersion) ||
            Spec?.Target == null || Spec.Evaluator == null || Spec.Metrics?.Guardrails == null || Spec.Budget == null ||
            Spec.ProtectedPaths == null || Spec.Constraints == null || Audit?.Baseline == null || Audit.ProtectedFiles == null ||
            Audit.Findings == null || Candidates == null || Strategies == null || Outputs == null ||
            Candidates.Any(c => c?.RunId != runId) || Strategies.Any(s => s?.RunId != runId) ||
            Outputs.Any(o => o?.RunId != runId || string.IsNullOrWhiteSpace(o.ArtifactVersionId)))
            throw new InvalidDataException("Method-search review does not match the requested run");
        return this;
    }
}
public sealed record NativeMethodState(string RunId, string SpecArtifactVersionId, string SpecSha256, string ControlState, string? ResultStatus);
public sealed record NativeMethodSpec(string Objective, NativeMethodTarget Target, NativeMethodEvaluator Evaluator,
    NativeMethodMetrics Metrics, string[] ProtectedPaths, string[] Constraints, NativeMethodBudget Budget, JsonElement? FinalVerification);
public sealed record NativeMethodTarget(string SourceArtifactVersionId, string SourcePath, string Symbol);
public sealed record NativeMethodEvaluator(string ArtifactVersionId, string EntryPath, long Repetitions, long TimeoutSeconds);
public sealed record NativeMethodMetrics(string Primary, string Direction, NativeMethodGuardrail[] Guardrails);
public sealed record NativeMethodGuardrail(string Metric, string Op, double Value);
public sealed record NativeMethodBudget(long MaxCandidates, long MaxCostMicrounits, long MaxWallSeconds, long MaxEvaluatorSeconds);
public sealed record NativeMethodAudit(NativeMethodBaseline Baseline, [property: JsonPropertyName("sentinelReachable")] bool SentinelReachable,
    [property: JsonPropertyName("protectedFiles")] JsonElement[] ProtectedFiles,
    [property: JsonPropertyName("targetSourceSha256")] string TargetSourceSha256, string[] Findings);
public sealed record NativeMethodBaseline(long Repetitions, long SuccessfulRepetitions, double MedianPrimary, double Spread, double NoiseFloor, double FailureRate);
public sealed record NativeMethodCandidate(string Id, string RunId, string? ParentCandidateId, long Sequence, string Family, string StrategyKey,
    string Status, double? PrimaryScore, long? ChangedLines, long? RuntimeMs, string? Rationale, string? Error);
public sealed record NativeMethodStrategy(string RunId, string StrategyKey, string Category, double Weight, long Attempts, long Improvements);
public sealed record NativeMethodOutput(string RunId, string ArtifactVersionId, string Role, string LogicalOutputKey, string SourcePath);
public sealed record NativeMethodProgressDetail(string? Phase = null, double? BaselinePrimary = null, double? BestPrimary = null,
    long CandidateCount = 0, long SuccessfulCount = 0, long FailedCount = 0, long CostMicrounits = 0, string? CurrentStrategy = null,
    long? LastCheckpointAt = null, string? BestCandidateId = null)
{
    public static NativeMethodProgressDetail Parse(string? json)
    {
        try { return JsonSerializer.Deserialize<NativeMethodProgressDetail>(json ?? "{}", ConversationSnapshot.JsonOptions) ?? new(); }
        catch (JsonException) { return new(); }
    }
}
