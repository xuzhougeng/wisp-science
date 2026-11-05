using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public sealed partial class NativeSettingsEditorModel
{
    private readonly Dictionary<string, JsonObject> confirmedProviderRows = [];
    public bool AddingProvider => command == "save_provider" && Draft != null;
    public string? ProviderErrorKey(bool save = true) => AddingProvider ? NativeProviderDrafts.ErrorKey(Draft!,
        (Values["list_models"] as JsonArray)?.OfType<JsonObject>() ?? [], save) : null;
    private void ResetProviderBatch() => confirmedProviderRows.Clear();
    private async Task<bool> SaveProviderAsync()
    {
        if (!AddingProvider || Busy || Closed) return false;
        if (ProviderErrorKey() is { } error) { Fail(error); return false; }
        var draft = (JsonObject)Draft!.DeepClone();
        Saving = true; SaveConfirmed = false;
        try
        {
            return await RunAsync(async () =>
            {
                JsonNode? result = null;
                foreach (var entry in NativeProviderDrafts.SaveOrder(draft))
                {
                    if (Closed) return null;
                    var profile = NativeProviderDrafts.Profile(draft, entry);
                    JsonObject? catalog;
                    try { catalog = await client.InvokeAsync("model_catalog_lookup", NativeModelDrafts.CatalogArguments(profile), projectId) as JsonObject; }
                    catch { catalog = null; }
                    if (Closed) return null;
                    profile = NativeProviderDrafts.Profile(draft, entry, catalog);
                    var key = NativeProviderDrafts.Text(draft, "key"); profile["key"] = key.Trim().Length == 0 ? null : JsonValue.Create(key);
                    var args = NativeModelDrafts.SaveArguments(profile); var row = NativeProviderDrafts.Text(entry, "row_id");
                    if (confirmedProviderRows.TryGetValue(row, out var confirmed) && JsonNode.DeepEquals(confirmed, args)) continue;
                    result = await client.InvokeAsync("save_model", args, projectId);
                    if (Closed) return null;
                    confirmedProviderRows[row] = (JsonObject)args.DeepClone();
                    Values["list_models"] = result?.DeepClone();
                }
                return result;
            }, _ => { Draft = null; original = null; command = null; SaveConfirmed = true; ResetProviderBatch(); });
        }
        finally { Saving = false; if (!Closed) Notify(); }
    }
    private Task<bool> TestProviderAsync(Action<JsonNode?> apply)
    {
        if (!AddingProvider || Busy || Closed) return Task.FromResult(false);
        if (ProviderErrorKey(save: false) is { } error) { Fail(error); return Task.FromResult(false); }
        var draft = (JsonObject)Draft!.DeepClone();
        var entry = NativeProviderDrafts.Entries(draft).First(row => NativeProviderDrafts.Text(row, "model").Trim().Length > 0);
        var profile = NativeProviderDrafts.Profile(draft, entry);
        var stored = NativeProviderDrafts.StoredKeyProfile(draft, (Values["list_models"] as JsonArray)?.OfType<JsonObject>() ?? []);
        profile["id"] = stored?["id"]?.DeepClone(); profile["key"] = draft["key"]?.DeepClone(); profile["has_api_key"] = stored != null;
        return RunAsync(async () =>
        {
            var settings = await client.InvokeAsync("get_settings", new(), projectId) as JsonObject ?? throw new InvalidDataException("Unable to read settings.");
            if (Closed) return null;
            return await client.InvokeAsync("validate_settings", NativeModelDrafts.TestArguments(settings, profile,
                NativeProxyRoutes.ModelValue(draft, (Values["list_models"] as JsonArray)?.OfType<JsonObject>() ?? [])), projectId);
        }, apply);
    }
}
