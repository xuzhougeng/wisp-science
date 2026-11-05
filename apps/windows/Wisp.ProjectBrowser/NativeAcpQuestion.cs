using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// Presentation of the existing question payload, with WebView's permissive defaults.
public sealed record NativeAcpQuestion(string RequestId, string Text, NativeAcpQuestion.Choice[] Choices,
    bool AllowsFreeform, string Status)
{
    public sealed record Choice(string Label, string Description);
    private static string? TextAt(JsonNode? value) => value is JsonValue scalar && scalar.TryGetValue<string>(out var text) ? text : null;

    public static NativeAcpQuestion? Parse(JsonObject payload)
    {
        if (TextAt(payload["request_id"]) is not { Length: > 0 } request) return null;
        var choices = payload["options"] is JsonArray options
            && options.All(option => option is JsonObject obj
                && (!obj.ContainsKey("label") || TextAt(obj["label"]) != null)
                && (!obj.ContainsKey("description") || TextAt(obj["description"]) != null))
            ? options.Select(option => new Choice(TextAt(option!["label"]) ?? "", TextAt(option["description"]) ?? "")).ToArray() : [];
        var freeform = payload["allow_freeform"] is not JsonValue value || !value.TryGetValue<bool>(out var allowed) || allowed;
        return new(request, TextAt(payload["question"]) ?? "", choices, freeform || choices.Length == 0,
            TextAt(payload["status"]) is "answered" or "expired" ? TextAt(payload["status"])! : "pending");
    }

    public bool Pending(IEnumerable<string>? liveRequests) => Status == "pending" && liveRequests?.Contains(RequestId) == true;
    public string NoteKey => Status == "answered" ? "plan.answer_sent" : "plan.question.expired";
    public (string Project, string Session, string Request) DraftKey(string project, string session) => (project, session, RequestId);
}
