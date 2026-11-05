using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public enum NativeProjectImportKind { Directory, Archive, Recovery }
public enum NativeProjectImportStage { Options, Selecting, Reading, Importing, Preview, Complete, Failed, Cancelled }

/// One import owns its picker results and confirmation. It never uses an active project.
public sealed class NativeProjectImportModel(INativeSettingsClient client) : WorkspaceActionModel
{
    public NativeProjectImportStage Stage { get; private set; } = NativeProjectImportStage.Options;
    public NativeProjectImportKind Kind { get; private set; }
    public string ProgressKey { get; private set; } = "projects.transfer.reading";
    public string SourcePath { get; private set; } = "";
    public string Name { get; set; } = "";
    public JsonObject? Preview { get; private set; }
    public string? ImportedProjectId { get; private set; }
    public bool IsModal => Stage is NativeProjectImportStage.Options or NativeProjectImportStage.Preview;
    public bool CanRecover => !Closed && !Busy && Stage == NativeProjectImportStage.Preview && Count("recoverable_session_count") > 0 && !string.IsNullOrWhiteSpace(Name);
    public long Count(string key) => Preview?[key]?.GetValue<long>() ?? 0;
    public long? Timestamp(string key) => Preview?[key]?.GetValue<long>();

    public async Task<bool> ChooseAsync(NativeProjectImportKind kind, Func<Task<string?>> pickSource, Func<Task<string?>> pickDestination)
    {
        if (Closed || Busy || Stage != NativeProjectImportStage.Options) return false;
        Kind = kind;
        var ok = await RunAsync(async () =>
        {
            Move(NativeProjectImportStage.Selecting, kind == NativeProjectImportKind.Archive ? "projects.transfer.selecting_archive" : "projects.transfer.selecting_project_folder");
            var source = await pickSource();
            if (Closed) return (JsonNode?)null;
            if (string.IsNullOrWhiteSpace(source)) { Stage = NativeProjectImportStage.Cancelled; return null; }
            SourcePath = source;
            if (kind == NativeProjectImportKind.Recovery)
            {
                Move(NativeProjectImportStage.Reading, "projects.transfer.reading");
                return await client.InvokeAsync("native_project_recovery_preview", new() { ["workspace_dir"] = source });
            }
            var args = new JsonObject();
            if (kind == NativeProjectImportKind.Archive)
            {
                Move(NativeProjectImportStage.Selecting, "projects.transfer.selecting_import_destination");
                var destination = await pickDestination();
                if (Closed) return null;
                if (string.IsNullOrWhiteSpace(destination)) { Stage = NativeProjectImportStage.Cancelled; return null; }
                args["archive_path"] = source; args["destination_parent"] = destination;
            }
            else args["directory_path"] = source;
            Move(NativeProjectImportStage.Importing, "projects.transfer.reading");
            return await client.InvokeAsync(kind == NativeProjectImportKind.Archive ? "native_project_import" : "native_project_import_directory", args);
        }, result =>
        {
            if (Stage == NativeProjectImportStage.Cancelled) return;
            if (kind == NativeProjectImportKind.Recovery)
            {
                Preview = ReadPreview(result);
                SourcePath = Preview["workspace_dir"]!.GetValue<string>();
                Name = Preview["suggested_name"]!.GetValue<string>();
                Stage = NativeProjectImportStage.Preview;
            }
            else { ImportedProjectId = ReadId(result, "id"); Stage = NativeProjectImportStage.Complete; }
        });
        if (!Closed && !ok) { Stage = NativeProjectImportStage.Failed; Notify(); }
        return ok;
    }
    public Task<bool> RecoverAsync()
    {
        if (!CanRecover) return Task.FromResult(false);
        var args = new JsonObject { ["workspace_dir"] = SourcePath, ["name"] = Name };
        return RunAsync(() => client.InvokeAsync("native_project_recover_workspace", args), result =>
        { ImportedProjectId = ReadId(result, "project_id"); Stage = NativeProjectImportStage.Complete; });
    }
    private void Move(NativeProjectImportStage stage, string key) { Stage = stage; ProgressKey = key; Notify(); }
    private static string ReadId(JsonNode? value, string key)
    {
        if (value is JsonObject obj && obj[key] is JsonValue id && id.TryGetValue<string>(out var text) && !string.IsNullOrWhiteSpace(text)) return text;
        throw new InvalidDataException("No valid project was returned. Check the project list before retrying.");
    }
    private static JsonObject ReadPreview(JsonNode? value)
    {
        if (value is not JsonObject data || data["workspace_dir"] is not JsonValue path || !path.TryGetValue<string>(out var directory) || string.IsNullOrWhiteSpace(directory)
            || data["suggested_name"] is not JsonValue name || !name.TryGetValue<string>(out _)) throw new InvalidDataException("Missing recovery preview.");
        foreach (var key in new[] { "archive_count", "valid_archive_count", "recoverable_session_count", "message_count", "invalid_archive_count", "duplicate_archive_count" })
            if (data[key] is not JsonValue count || !count.TryGetValue<long>(out var number) || number < 0) throw new InvalidDataException("Invalid recovery count: " + key);
        foreach (var key in new[] { "earliest_message_at", "latest_message_at" })
            if (data[key] != null && (data[key] is not JsonValue timestamp || !timestamp.TryGetValue<long>(out var seconds) || seconds is < -62135596800 or > 253402300799)) throw new InvalidDataException("Invalid recovery timestamp.");
        return (JsonObject)data.DeepClone();
    }
}
