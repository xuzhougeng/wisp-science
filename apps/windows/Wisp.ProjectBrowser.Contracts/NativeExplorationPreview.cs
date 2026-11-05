using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativeExplorationFile(string Path, string Kind, JsonElement? Before, JsonElement? After);
public sealed record NativeExplorationArtifact(
    [property: JsonPropertyName("logicalKey")] string LogicalKey,
    [property: JsonPropertyName("beforeArtifactId")] string? BeforeArtifactId,
    [property: JsonPropertyName("beforeVersionId")] string? BeforeVersionId,
    [property: JsonPropertyName("afterArtifactId")] string AfterArtifactId,
    [property: JsonPropertyName("afterVersionId")] string AfterVersionId);
public sealed record NativeExplorationDecision(string Id, string Kind, string Title, string? RefId, string MetadataJson);
public sealed record NativeExplorationEdge(string SourceId, string TargetId, string Relation, string MetadataJson);
public sealed record NativeExplorationResource(string Id, string Kind, string Uri, string? Version, string? Checksum);
public sealed record NativeExplorationEffect(string Id, string EffectKind, string Recoverability, string TargetSummary, string MetadataJson, long CreatedAt);
public sealed record NativeExplorationDiff(
    [property: JsonPropertyName("explorationId")] string ExplorationId,
    NativeExplorationFile[] Files, NativeExplorationArtifact[] Artifacts, NativeRun[] Runs, NativeExplorationDecision[] Decisions,
    [property: JsonPropertyName("researchEdges")] NativeExplorationEdge[] ResearchEdges,
    [property: JsonPropertyName("externalResources")] NativeExplorationResource[] ExternalResources,
    [property: JsonPropertyName("externalEffects")] NativeExplorationEffect[] ExternalEffects);
public sealed record NativeExplorationMainlineChanges(NativeExplorationFile[] Files,
    [property: JsonPropertyName("artifactKeys")] string[] ArtifactKeys,
    [property: JsonPropertyName("entityKeys")] string[] EntityKeys,
    [property: JsonPropertyName("sourceMessageHead")] long SourceMessageHead,
    [property: JsonPropertyName("sourceUiEventHead")] long SourceUiEventHead,
    [property: JsonPropertyName("stateGeneration")] long StateGeneration);
public sealed record NativePromotionBlocker(string Code, string Message);
public sealed record NativePromotionEligibility([property: JsonRequired] bool Eligible, string? Code, NativePromotionBlocker[] Reasons,
    [property: JsonPropertyName("expectedGuardHash")] string ExpectedGuardHash,
    [property: JsonPropertyName("manualResolutionAvailable"), JsonRequired] bool ManualResolutionAvailable);
public sealed record NativeExplorationPreviewData(NativeExploration Exploration, NativeExplorationDiff Diff,
    [property: JsonPropertyName("mainlineChanges")] NativeExplorationMainlineChanges MainlineChanges, NativePromotionEligibility Eligibility);

public sealed record NativeExplorationPreview(string ProjectId, string SessionId, string SourceSessionId, NativeExplorationPreviewData Preview)
{
    public static NativeExplorationPreview Decode(JsonNode? data, string project, NativeExplorationSummary owner)
    {
        var result = data?.Deserialize<NativeExplorationPreview>(ConversationSnapshot.JsonOptions);
        var value = result?.Preview;
        if (result?.ProjectId != project || result.SessionId != owner.Exploration.FrameId || result.SourceSessionId != owner.SourceFrameId
            || value?.Exploration == null || value.Exploration.Id != owner.Exploration.Id || value.Exploration.FrameId != result.SessionId
            || value.Exploration.Name == null || value.Exploration.Status is not ("creating" or "active" or "promoting" or "failed")
            || value.Diff == null || value.Diff.ExplorationId != owner.Exploration.Id || value.MainlineChanges?.Files == null
            || value.Eligibility == null || value.Eligibility.Reasons == null || value.Eligibility.Reasons.Any(row => row == null || row.Code == null || row.Message == null)
            || value.Eligibility.Eligible && (value.Exploration.Status != "active" || string.IsNullOrWhiteSpace(value.Eligibility.ExpectedGuardHash))
            || value.Eligibility.ManualResolutionAvailable && (value.Eligibility.Eligible || value.Exploration.Status != "active")
            || value.Diff.Files == null || value.Diff.Artifacts == null || value.Diff.Runs == null || value.Diff.Decisions == null
            || value.Diff.ResearchEdges == null || value.Diff.ExternalResources == null || value.Diff.ExternalEffects == null
            || value.Diff.Files.Concat(value.MainlineChanges.Files).Any(row => row == null || row.Path == null || row.Kind == null)
            || value.Diff.Artifacts.Any(row => row == null || row.LogicalKey == null || row.AfterVersionId == null)
            || value.Diff.Runs.Any(row => row == null || row.Title == null || row.Status == null)
            || value.Diff.Decisions.Any(row => row == null || row.Title == null || row.Kind == null)
            || value.Diff.ResearchEdges.Any(row => row == null || row.Relation == null || row.SourceId == null || row.TargetId == null)
            || value.Diff.ExternalResources.Any(row => row == null || row.Kind == null || row.Uri == null)
            || value.Diff.ExternalEffects.Any(row => row == null || row.TargetSummary == null || row.EffectKind == null || row.Recoverability == null))
            throw new InvalidDataException("Exploration preview did not match the reviewed project and candidate");
        return result;
    }
}

public sealed record NativeExplorationActionResult(string ProjectId, string SessionId, string SourceSessionId, string ExplorationId,
    string Action, [property: JsonRequired] bool Completed)
{
    public static NativeExplorationActionResult Decode(JsonNode? data, string project, NativeExplorationSummary owner, string action)
    {
        var result = data?.Deserialize<NativeExplorationActionResult>(ConversationSnapshot.JsonOptions);
        if (result?.ProjectId != project || result.SessionId != owner.Exploration.FrameId || result.SourceSessionId != owner.SourceFrameId
            || result.ExplorationId != owner.Exploration.Id || result.Action != action || !result.Completed)
            throw new InvalidDataException("Exploration acknowledgement did not match the reviewed action");
        return result;
    }
}
