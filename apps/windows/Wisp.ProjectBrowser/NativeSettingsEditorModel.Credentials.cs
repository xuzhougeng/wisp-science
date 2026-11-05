using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public sealed partial class NativeSettingsEditorModel
{
    private readonly Dictionary<string, string> credentialInputs = [];
    public string CustomCredentialName { get; set; } = "";
    public string CustomCredentialEnvironment { get; set; } = "";
    public string CustomCredentialValue { get; set; } = "";
    public string? CredentialFeedbackKey { get; private set; }
    public bool CredentialsLoaded => Values["credential_status"] is JsonArray && Values["list_custom_credentials"] is JsonArray;
    public JsonObject[] CustomCredentials => (Values["list_custom_credentials"] as JsonArray)?.OfType<JsonObject>().ToArray() ?? [];
    public bool HasCredentialChanges => credentialInputs.Values.Any(v => v.Length > 0) || CustomCredentialName.Length > 0 || CustomCredentialEnvironment.Length > 0 || CustomCredentialValue.Length > 0;
    public bool CanAddCredential => !Busy && !Closed && CredentialsLoaded && !string.IsNullOrWhiteSpace(CustomCredentialName)
        && !string.IsNullOrWhiteSpace(CustomCredentialEnvironment) && !string.IsNullOrWhiteSpace(CustomCredentialValue);
    private JsonArray? CredentialStatusRow(string id) => (Values["credential_status"] as JsonArray)?.OfType<JsonArray>()
        .FirstOrDefault(row => row.Count == 2 && row[0]?.GetValue<string>() == id);
    private bool KnownCredential(string id) => CredentialStatusRow(id) != null || CustomCredentials.Any(row => row["id"]?.GetValue<string>() == id);
    public bool CredentialPresent(string id) => CredentialStatusRow(id)?[1]?.GetValue<bool>()
        ?? CustomCredentials.FirstOrDefault(row => row["id"]?.GetValue<string>() == id)?["present"]?.GetValue<bool>() ?? false;
    public string CredentialInput(string id) => credentialInputs.GetValueOrDefault(id, "");
    public void SetCredentialInput(string id, string value)
    {
        if (Busy || Closed || !KnownCredential(id)) return;
        credentialInputs[id] = value; CredentialFeedbackKey = null;
    }
    public void DiscardCredentialInputs(IEnumerable<string>? ids = null)
    {
        if (Busy) return;
        if (ids == null) { credentialInputs.Clear(); CustomCredentialName = CustomCredentialEnvironment = CustomCredentialValue = ""; }
        else foreach (var id in ids) credentialInputs.Remove(id);
        CredentialFeedbackKey = null; ClearFailure();
    }
    private void AcceptCredential(string id, bool present)
    {
        if (CredentialStatusRow(id) is { } row) row[1] = present;
        else (Values["credential_status"] as JsonArray)?.Add(new JsonArray(id, present));
        foreach (var item in CustomCredentials.Where(item => item["id"]?.GetValue<string>() == id)) item["present"] = present;
        credentialInputs.Remove(id);
    }
    public Task<bool> SaveCredentialsAsync(IEnumerable<string> ids)
    {
        if (Busy || Closed || !CredentialsLoaded) return Task.FromResult(false);
        var edits = ids.Distinct().Where(KnownCredential).Select(id => (Id: id, Value: CredentialInput(id)))
            .Where(edit => !string.IsNullOrWhiteSpace(edit.Value)).ToArray();
        if (edits.Length == 0) return Task.FromResult(false);
        CredentialFeedbackKey = null;
        return RunAsync(async () =>
        {
            foreach (var (id, value) in edits)
            {
                if (Closed) return false;
                await client.InvokeAsync("set_credential", new() { ["id"] = id, ["value"] = value }, projectId);
                // Retain only unacknowledged edits if a later field fails. A retry
                // must not silently replay an already committed secret.
                if (Closed) return false;
                AcceptCredential(id, true);
            }
            return true;
        }, _ => CredentialFeedbackKey = "cred.saved");
    }
    public Task<bool> ClearCredentialAsync(string id)
    {
        if (Busy || Closed || !CredentialsLoaded || !KnownCredential(id) || !CredentialPresent(id)) return Task.FromResult(false);
        CredentialFeedbackKey = null;
        return RunAsync(() => client.InvokeAsync("set_credential", new() { ["id"] = id, ["value"] = "" }, projectId), _ =>
        { AcceptCredential(id, false); CredentialFeedbackKey = "cred.cleared"; });
    }
    public Task<bool> AddCustomCredentialAsync()
    {
        if (!CanAddCredential) return Task.FromResult(false);
        CredentialFeedbackKey = null;
        var args = new JsonObject { ["name"] = CustomCredentialName, ["envVar"] = CustomCredentialEnvironment, ["value"] = CustomCredentialValue };
        return RunAsync(() => client.InvokeAsync("add_custom_credential", args, projectId), result =>
        {
            if (result is not JsonObject row || string.IsNullOrEmpty(row["id"]?.GetValue<string>()) || row["present"] is not JsonValue)
                throw new InvalidDataException("Missing credential status acknowledgement");
            var id = row["id"]!.GetValue<string>(); var rows = (JsonArray)Values["list_custom_credentials"]!;
            var existing = CustomCredentials.FirstOrDefault(item => item["id"]?.GetValue<string>() == id);
            if (existing == null) rows.Add(row.DeepClone()); else rows[rows.IndexOf(existing)] = row.DeepClone();
            AcceptCredential(id, row["present"]!.GetValue<bool>());
            CustomCredentialName = CustomCredentialEnvironment = CustomCredentialValue = "";
            CredentialFeedbackKey = "cred.custom.added";
        });
    }
    public Task<bool> RemoveCustomCredentialAsync(string id)
    {
        if (Busy || Closed || !CredentialsLoaded || !CustomCredentials.Any(row => row["id"]?.GetValue<string>() == id)) return Task.FromResult(false);
        CredentialFeedbackKey = null;
        return RunAsync(() => client.InvokeAsync("remove_custom_credential", new() { ["id"] = id }, projectId), _ =>
        {
            foreach (var row in CustomCredentials.Where(row => row["id"]?.GetValue<string>() == id)) ((JsonArray)Values["list_custom_credentials"]!).Remove(row);
            if (CredentialStatusRow(id) is { } status) ((JsonArray)Values["credential_status"]!).Remove(status);
            credentialInputs.Remove(id); CredentialFeedbackKey = "cred.custom.removed";
        });
    }
}
