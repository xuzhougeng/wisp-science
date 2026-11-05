using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// The environment card owns a draft independently of the General preferences.
public sealed class NativeLocalEnvironmentModel(INativeSettingsClient client, string? projectId) : WorkspaceActionModel
{
    public static (string Label, string Key)[] Fields { get; } =
    [
        ("Python", "python_executable"), ("Rscript", "rscript_executable"), ("uv", "uv_executable"),
        ("Node", "node_executable"), ("npm", "npm_executable"), ("sci", "sci_executable"),
        ("pixi", "pixi_executable"), ("Cua Driver", "cua_driver_executable")
    ];
    public JsonObject? Snapshot { get; private set; }
    public JsonObject? Draft { get; private set; }
    private JsonObject? original;
    public bool Editing => Draft != null;
    // Only the path editor can start a save; detection runs outside edit mode.
    public bool Saving => Busy && Editing;
    public bool CanEdit => !Closed && !Busy && !Editing && Paths != null;
    public bool HasChanges => Editing && !JsonNode.DeepEquals(original, Draft);
    public bool Saved { get; private set; }
    public JsonObject? Paths => Snapshot?["local_environment"]?["paths"] as JsonObject;
    public string? Warning => Snapshot?["local_environment"]?["warning"]?.GetValue<string>();
    public void AcceptSnapshot(JsonNode? value)
    {
        if (!Busy && !Closed && !Editing && value is JsonObject snapshot) Snapshot = (JsonObject)snapshot.DeepClone();
    }
    public void Edit()
    {
        if (!CanEdit) return;
        original = (JsonObject)Paths!.DeepClone();
        Draft = (JsonObject)original.DeepClone(); Saved = false; ClearFailure(); Notify();
    }
    public void Cancel()
    {
        if (Busy || Closed) return;
        Draft = original = null; Saved = false; ClearFailure(); Notify();
    }
    public Task<bool> SaveAsync() => !Editing ? Task.FromResult(false) : RefreshAsync(true);
    public Task<bool> DetectAsync() => Editing ? Task.FromResult(false) : RefreshAsync(false);
    private Task<bool> RefreshAsync(bool save)
    {
        var args = save ? new JsonObject { ["paths"] = Draft!.DeepClone() } : new JsonObject();
        return RunAsync(async () =>
        {
            var result = await client.InvokeAsync(save ? "save_local_environment_paths" : "detect_local_environment", args, projectId);
            if (result is not JsonObject snapshot || snapshot["local_environment"]?["paths"] is not JsonObject)
                throw new InvalidDataException("The host did not return the updated local environment paths.");
            return snapshot;
        }, snapshot =>
        {
            Snapshot = (JsonObject)snapshot.DeepClone(); Draft = original = null; Saved = save;
        });
    }
    public override void Dispose() { base.Dispose(); Draft = original = Snapshot = null; }
}
