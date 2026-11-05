using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public sealed partial class NativeSettingsEditorModel
{
    public bool ReviewerTesting { get; private set; }
    public bool ReviewerTestAttempted { get; private set; }
    public JsonObject? ReviewerReport { get; private set; }
    public bool CanTestReviewer => !Busy && !Closed && !string.IsNullOrEmpty(projectId)
        && command == "save_specialist_cmd" && Draft?["id"]?.GetValue<string>() == "reviewer";

    private void ResetReviewerTest()
    {
        ReviewerTesting = false;
        ReviewerTestAttempted = false;
        ReviewerReport = null;
    }

    // Uses the current form without persisting it or changing the active conversation.
    public async Task<bool> TestReviewerAsync()
    {
        if (!CanTestReviewer) return false;
        var args = new JsonObject { ["reviewer"] = NativeSpecialists.SaveDraft(Draft!, original!), ["sessionId"] = sessionId };
        ReviewerReport = null; ReviewerTestAttempted = true; ReviewerTesting = true; SaveConfirmed = false;
        try
        {
            return await RunAsync(async () =>
            {
                using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(120));
                var result = await client.InvokeAsync("test_reviewer_backend", args, projectId, timeout.Token);
                // A transport acknowledgement alone is not a successful Reviewer report.
                if (result is not JsonObject report || new[] { "backend", "model", "status", "summary" }
                    .Any(key => report[key] is not JsonValue value || !value.TryGetValue<string>(out _)))
                    throw new InvalidDataException("Invalid Reviewer backend test response.");
                return (JsonObject)report.DeepClone();
            }, report => ReviewerReport = report);
        }
        finally { ReviewerTesting = false; if (!Closed) Notify(); }
    }
}
