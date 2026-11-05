using System.Text.Json.Nodes;
using System.Text.Json;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeBranchResultsTests
{
    public static void Run(string fixtures)
    {
        var json = JsonNode.Parse(File.ReadAllText(Path.Combine(fixtures, "../../native-conversations/v1/branch-results.json")))!;
        var page = ConversationSnapshot.Decode(json, "research-1", "main-1");
        Check(NativeBranchResults.At(page, 0).Single().Id == "before-1", "absolute before-user checkpoint");
        var merged = NativeBranchResults.At(page, 1).Single();
        Check(merged.Id == "branch-1" && merged.Merged && merged.MergeSummary!.EndsWith("cohort.\n"), "exact summary remains attached to its source response");
        Check(page.Items[1].Text == "The original answer remains visible.", "original answer is not replaced by merge text");
        Check(NativeBranchResults.At(page with { UserOffset = 3 }, 1).Length == 0, "other history page never acquires this result");
        Check(NativeBranchResults.At(page with { SessionId = "other" }, 1).Length == 0, "source session isolation");
        Check(NativeBranchResults.At(page, -1).Length == 0 && NativeBranchResults.At(page, 2).Length == 0, "invalid indexes");
        var assistant = page.Items[1];
        Check(NativeBranchResults.At(page with { Items = [assistant] }, 0).Length == 0, "no fabricated owner for leading assistant");
        Check(NativeBranchResults.At(page with { Items = [page.Items[0], assistant with { Role = "tool" }] }, 1).Length == 0, "tools do not acquire response links");
        Check(NativeBranchResults.At(page with { Items = [page.Items[0] with { Role = "queued_user" }, assistant] }, 1).Length == 1, "queued user owning-turn count matches WebView");
        Check(NativeBranchResults.IsStandalone(assistant with { Role = "branch_merge", ToolName = "Branch title" })
            && !NativeBranchResults.IsStandalone(assistant), "legacy card requires its explicit role, never guessed from content");
        Check(NativeBranchResults.Title(" ", "Fallback") == "Fallback" && NativeBranchResults.Title("  Literal  ", "Fallback") == "  Literal  ", "fallback only for blank titles");
        var legacy = json.DeepClone(); legacy.AsObject().Remove("branches");
        Check(NativeBranchResults.At(ConversationSnapshot.Decode(legacy, "research-1", "main-1"), 1).Length == 0, "older host remains compatible");
        var wrong = json.DeepClone(); wrong["branches"]![0]!["source_session_id"] = "foreign";
        Reject(wrong, "cross-session link rejected");
        wrong = json.DeepClone(); wrong["branches"]![1]!["id"] = "before-1"; Reject(wrong, "duplicate IDs rejected");
        wrong = json.DeepClone(); wrong["branches"]![0]!["checkpoint_user_index"] = -1; Reject(wrong, "negative checkpoint rejected");
        var catalog = JsonSerializer.Deserialize<Dictionary<string, Dictionary<string, string>>>(File.ReadAllText(
            Path.Combine(fixtures, "../../../apps/macos/Sources/WispProjectBrowserUI/Resources/native-locales.json")))!;
        var locale = new NativeLocalization(catalog, "en");
        var labels = NativeTranscriptLabels.Actions(locale);
        Check(labels.Copy == "Copy" && labels.AddToChat == "Add to chat" && labels.SideChat == "Quote in side chat" && labels.Highlight == "Save highlight", "message and selection actions share the exported English catalog");
        Check(NativeTranscriptLabels.Role(page.Items[0], locale) == "You", "user role follows locale");
        Check(NativeTranscriptLabels.Role(assistant with { Role = "tool", ToolName = "用户定义工具" }, locale) == "用户定义工具", "tool names stay verbatim");
        Check(NativeTranscriptLabels.Role(assistant with { Role = "tool", ToolName = "attempt_completion", Ok = true }, locale) == "Wisp Science", "completion remains an assistant answer");
        locale.Preview("zh");
        Check(NativeTranscriptLabels.Actions(locale).Copy == "复制" && NativeTranscriptLabels.Role(page.Items[0], locale) == "你", "fresh labels reflect an immediate locale change");
    }
    private static void Reject(JsonNode data, string reason)
    {
        try { ConversationSnapshot.Decode(data, "research-1", "main-1"); }
        catch (InvalidDataException) { return; }
        throw new InvalidOperationException(reason);
    }
    private static void Check(bool valid, string message) { if (!valid) throw new InvalidOperationException(message); }
}
