using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativeSessionRulesStatus(
    [property: JsonPropertyName("project_id")] string ProjectId,
    [property: JsonPropertyName("session_id")] string SessionId,
    [property: JsonPropertyName("stale_prompt"), JsonRequired] bool StalePrompt);

public sealed record NativeSessionRulesResult(
    [property: JsonPropertyName("project_id")] string ProjectId,
    [property: JsonPropertyName("session_id")] string SessionId,
    [property: JsonPropertyName("changed"), JsonRequired] bool Changed);
