using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public enum NativeProjectExportStage { Options, Selecting, Exporting, Complete, Failed, Cancelled }

/// One explicit project owns the export; a lost response is never replayed.
public sealed class NativeProjectExportModel(INativeSettingsClient client, string projectId) : WorkspaceActionModel
{
    public string ProjectId { get; } = projectId;
    public NativeProjectExportStage Stage { get; private set; } = NativeProjectExportStage.Options;
    public string Destination { get; private set; } = "";
    public bool IsModal => Stage == NativeProjectExportStage.Options;
    public bool LocksProject => !Closed && Busy;

    public async Task<bool> ExportAsync(bool directory, Func<Task<string?>> pickDestination)
    {
        if (Closed || Busy || Stage != NativeProjectExportStage.Options) return false;
        var format = directory ? "directory" : "zip";
        var ok = await RunAsync(async () =>
        {
            Stage = NativeProjectExportStage.Selecting; Notify();
            var destination = await pickDestination();
            if (Closed) return (JsonNode?)null;
            if (string.IsNullOrWhiteSpace(destination)) { Stage = NativeProjectExportStage.Cancelled; return null; }
            if (!Path.IsPathFullyQualified(destination)) throw new InvalidDataException("Choose an absolute export destination.");
            Destination = destination;
            Stage = NativeProjectExportStage.Exporting; Notify();
            return await client.InvokeAsync("native_project_export", new() { ["destination_path"] = destination, ["format"] = format }, ProjectId);
        }, result =>
        {
            if (Stage == NativeProjectExportStage.Cancelled) return;
            if (result is not JsonObject data || data["project_id"]?.GetValue<string>() != ProjectId
                || data["format"]?.GetValue<string>() != format || data["destination_path"]?.GetValue<string>() != Destination)
                throw new InvalidDataException("Export result did not match the selected project and destination. Check the destination before retrying.");
            Stage = NativeProjectExportStage.Complete;
        });
        if (!Closed && !ok) { Stage = NativeProjectExportStage.Failed; Notify(); }
        return ok;
    }

    public static string DirectoryName(string name)
    {
        var result = new string(name.Trim().Select(c => char.IsControl(c) || "<>:\"/\\|?*".Contains(c) ? '-' : c).ToArray()).TrimEnd(' ', '.');
        if (result.Length == 0) result = "wisp-project";
        // Win32 device names remain reserved even with a filename extension.
        var stem = result.Split('.')[0].ToUpperInvariant();
        if (stem is "CON" or "PRN" or "AUX" or "NUL" || stem.Length == 4 && (stem.StartsWith("COM") || stem.StartsWith("LPT")) && stem[3] is >= '1' and <= '9') result = "wisp-" + result;
        return result;
    }
    public static string DirectoryDestination(string parent, string name, Func<string, bool>? exists = null)
    {
        if (!Path.IsPathFullyQualified(parent)) throw new InvalidDataException("Choose an absolute export location.");
        exists ??= path => Directory.Exists(path) || File.Exists(path);
        var leaf = DirectoryName(name);
        for (var i = 0; i < 1000; i++)
        {
            var candidate = Path.Combine(parent, i == 0 ? leaf : $"{leaf}-{i + 1}");
            if (!exists(candidate)) return candidate;
        }
        throw new IOException("Could not choose an unused project directory.");
    }
    public static string ArchiveName(string name)
    {
        var leaf = string.Concat(name.EnumerateRunes().Select(r => System.Text.Rune.IsLetterOrDigit(r) || r.Value is '.' or '-' or '_' ? r.ToString() : "_")).Trim('.', '_', '-');
        return "wisp-project-" + (leaf.Length == 0 ? "project" : leaf) + ".zip";
    }
}
