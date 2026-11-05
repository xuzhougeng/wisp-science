using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeSpecialistChat(string ProjectId, string SessionId, string Prompt);

/// Uses the same portable-package commands as WebView. A preview never imports;
/// confirmation submits exactly the package the user reviewed, not a reread file.
public sealed class NativeSpecialistTransferModel(INativeSettingsClient client, string? projectId) : WorkspaceActionModel
{
    public JsonObject? Preview { get; private set; }
    public JsonArray? Findings { get; private set; }
    public JsonArray? Imported { get; private set; }
    public NativeSpecialistChat? Chat { get; private set; }
    public string ExportName { get; private set; } = "";
    private string exportId = "";
    public string ModelId { get; private set; } = "";
    public bool Reviewed { get; private set; }
    public string Operation { get; private set; } = "import";
    public string? NoticeKey { get; private set; }
    public string NoticeValue { get; private set; } = "";
    public bool ModalOpen => Preview != null || Findings != null;
    public bool HasConflict => Preview?["name_conflict"] != null;
    public bool CanImport => !Busy && !Closed && Preview != null && !HasConflict && Reviewed
        && (ModelId.Length == 0 || Matches.Any(row => Text(row, "id") == ModelId));
    public IEnumerable<JsonObject> Matches => (Preview?["model_matches"] as JsonArray)?.OfType<JsonObject>() ?? [];
    private static string Text(JsonNode? row, string key) => row?[key]?.GetValue<string>() ?? "";
    private static JsonArray Objects(JsonNode? value) => value is JsonArray rows && rows.All(row => row is JsonObject)
        ? (JsonArray)rows.DeepClone() : throw new InvalidDataException("Invalid specialist response.");
    private static JsonObject ValidatePreview(JsonNode? value)
    {
        if (value is not JsonObject preview || preview["package"] is not JsonObject package
            || Text(package, "format") != "wisp-specialist" || package["version"]?.GetValue<int>() != 1
            || string.IsNullOrWhiteSpace(Text(package, "name")) || package["instructions"] is not JsonValue
            || string.IsNullOrWhiteSpace(Text(preview, "file_name")) || preview["bytes"]?.GetValue<long>() is not >= 0
            || preview["skills_checked"] is not JsonValue)
            throw new InvalidDataException("Invalid specialist preview.");
        _ = preview["skills_checked"]!.GetValue<bool>();
        _ = package["instructions"]!.GetValue<string>();
        foreach (var key in new[] { "missing_skills", "missing_connectors" })
            if (preview[key] is not JsonArray entries || entries.Any(item => item is not JsonValue || item.GetValueKind() != System.Text.Json.JsonValueKind.String))
                throw new InvalidDataException("Invalid specialist whitelist preview.");
        foreach (var key in new[] { "skills", "connectors" })
            if (package[key] != null && (package[key] is not JsonArray entries || entries.Any(item => item is not JsonValue || item.GetValueKind() != System.Text.Json.JsonValueKind.String)))
                throw new InvalidDataException("Invalid specialist whitelist.");
        var matches = Objects(preview["model_matches"]);
        foreach (var match in matches)
        {
            if (Text(match, "id").Length == 0 || match?["same_provider"] is not JsonValue) throw new InvalidDataException("Invalid model match.");
            _ = match["same_provider"]!.GetValue<bool>();
        }
        return (JsonObject)preview.DeepClone();
    }
    public Task<bool> PickAsync()
    {
        if (ModalOpen || Busy || Closed) return Task.FromResult(false);
        Operation = "import"; NoticeKey = null; Imported = null;
        return RunAsync(async () =>
        {
            var selected = await client.InvokeAsync("pick_specialist_package", new(), projectId);
            if (Closed || selected == null) return null;
            var path = selected.GetValue<string>();
            if (string.IsNullOrWhiteSpace(path)) return null;
            return ValidatePreview(await client.InvokeAsync("preview_specialist_import", new() { ["path"] = path }, projectId));
        }, preview =>
        {
            Preview = preview; Reviewed = false;
            ModelId = Matches.FirstOrDefault(row => row["same_provider"]?.GetValue<bool>() == true)?["id"]?.GetValue<string>() ?? "";
        });
    }
    public void SetReviewed(bool value) { if (!Busy && !Closed && Preview != null && !HasConflict) { Reviewed = value; Notify(); } }
    public void SetModel(string value)
    {
        if (!Busy && !Closed && Preview != null && (value.Length == 0 || Matches.Any(row => Text(row, "id") == value))) { ModelId = value; Notify(); }
    }
    public Task<bool> ImportAsync()
    {
        if (!CanImport) return Task.FromResult(false);
        var package = (JsonObject)Preview!["package"]!.DeepClone();
        var args = new JsonObject { ["package"] = package, ["modelId"] = ModelId };
        return RunAsync(async () => Objects(await client.InvokeAsync("import_specialist", args, projectId)), rows =>
        {
            Imported = rows; Preview = null; Reviewed = false; ModelId = "";
            NoticeKey = "specialists.import.done"; NoticeValue = Text(package, "name");
        });
    }
    public Task<bool> ExportAsync(JsonObject saved)
    {
        if (Busy || Closed || ModalOpen || saved["builtin"]?.GetValue<bool>() != false || Text(saved, "id").Length == 0) return Task.FromResult(false);
        exportId = Text(saved, "id"); ExportName = Text(saved, "name"); NoticeKey = null; Operation = "export";
        return Export(false);
    }
    public Task<bool> ExportAnywayAsync() => !Busy && !Closed && Findings != null && exportId.Length > 0 ? Export(true) : Task.FromResult(false);
    private Task<bool> Export(bool acknowledged) => RunAsync(async () =>
    {
        var result = await client.InvokeAsync("export_specialist", new() { ["id"] = exportId, ["acknowledgeSecrets"] = acknowledged }, projectId) as JsonObject
            ?? throw new InvalidDataException("Invalid export response.");
        switch (Text(result, "status"))
        {
            case "secret_warning":
                var findings = Objects(result["findings"]);
                if (findings.Count == 0) throw new InvalidDataException("Missing export findings.");
                foreach (var finding in findings)
                    if (finding?["line"]?.GetValue<int>() is not > 0 || Text(finding, "kind").Length == 0 || Text(finding, "excerpt").Length == 0)
                        throw new InvalidDataException("Invalid export finding.");
                break;
            case "exported":
                if (Text(result, "path").Length == 0) throw new InvalidDataException("Missing export path.");
                break;
            case "cancelled": break;
            default: throw new InvalidDataException("Unknown export outcome.");
        }
        return result;
    }, result =>
    {
        Findings = Text(result, "status") == "secret_warning" ? (JsonArray)result["findings"]!.DeepClone() : null;
        if (Text(result, "status") == "exported") { NoticeKey = "specialists.export.done"; NoticeValue = Text(result, "path"); }
        if (Findings == null) exportId = "";
    });
    public bool Cancel()
    {
        if (Busy || Closed || !ModalOpen) return false;
        Preview = null; Findings = null; Reviewed = false; ModelId = exportId = ""; ClearFailure(); Notify(); return true;
    }
    public Task<bool> CreateChatAsync(string prompt)
    {
        if (Busy || Closed || ModalOpen || projectId == null || string.IsNullOrWhiteSpace(prompt)) return Task.FromResult(false);
        Chat = null; NoticeKey = null; Operation = "chat";
        return RunAsync(async () =>
        {
            var id = await new NativeConversationClient(client).CreateAsync(projectId);
            if (string.IsNullOrWhiteSpace(id)) throw new InvalidDataException("Missing conversation identity.");
            return new NativeSpecialistChat(projectId, id, prompt);
        }, launch => Chat = launch);
    }
    public override void Dispose() { base.Dispose(); Preview = null; Findings = null; Chat = null; }
}
