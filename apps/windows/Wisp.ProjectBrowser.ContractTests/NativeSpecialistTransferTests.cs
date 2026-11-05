using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSpecialistTransferTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var fake = new Fake(); using var model = new NativeSpecialistTransferModel(fake, "owned-project");
        fake.Path = null;
        Check(await model.PickAsync() && !model.ModalOpen && fake.Calls.Count == 1, "picker cancellation never previews or writes");
        fake.Path = "C:/fixtures/review.wisp-specialist.json";
        Check(await model.PickAsync() && model.ModalOpen && !model.CanImport && !await model.ImportAsync(), "preview is not import authorization");
        Check(fake.Calls[^1].Command == "preview_specialist_import" && fake.Calls.All(c => c.Project == "owned-project")
            && model.ModelId == "same-provider", "preview is project scoped and prefers the same provider among exact model matches");
        model.SetModel("unrelated"); Check(model.ModelId == "same-provider", "unlisted local profiles cannot be selected");
        model.SetModel(""); model.SetReviewed(true);
        fake.Pending = new(); var importing = model.ImportAsync(); var count = fake.Calls.Count;
        Check(model.Busy && !model.Cancel() && !await model.ImportAsync() && fake.Calls.Count == count, "pending import excludes dismiss and duplicate writes");
        Check(fake.Calls[^1].Args["modelId"]!.GetValue<string>() == ""
            && fake.Calls[^1].Args["package"]?["skills"] == null
            && fake.Calls[^1].Args["package"]?["connectors"] is JsonArray { Count: 0 }, "follow-active and inherited/empty whitelists keep their distinct wire meanings");
        model.Preview!["package"]!["instructions"] = "changed after click";
        Check(fake.Calls[^1].Args["package"]?["instructions"]?.GetValue<string>() == "Read the evidence.", "confirmation submits the reviewed snapshot");
        fake.Pending.SetException(new IOException("lost response"));
        Check(!await importing && model.Preview != null && model.Error != null && model.Imported == null && fake.Calls.Count == count, "ambiguous import retains preview and never retries");
        Check(model.Cancel() && !model.ModalOpen && model.Error == null, "Escape-equivalent dismiss preserves parent without writing");
        fake.Pending = null; fake.Preview["name_conflict"] = new JsonObject { ["id"] = "existing", ["name"] = "Example", ["builtin"] = true };
        Check(await model.PickAsync(), "conflict remains reviewable"); model.SetReviewed(true);
        Check(model.HasConflict && !model.Reviewed && !await model.ImportAsync(), "a conflicting name cannot overwrite a saved specialist");
        count = fake.Calls.Count;
        Check(model.Cancel() && !model.ModalOpen && !model.CanImport && fake.Calls.Count == count,
            "disabled conflict confirmation must still dismiss without an import or another host request");
        fake.Preview = Preview(); fake.Preview["model_matches"]![1]!["same_provider"] = false;
        Check(await model.PickAsync() && model.ModelId == "", "cross-provider match is never silently preselected"); model.SetModel("other-provider"); model.SetReviewed(true);
        Check(await model.ImportAsync() && !model.ModalOpen && model.Imported is { Count: 1 } && model.NoticeKey == "specialists.import.done", "confirmed import returns the authoritative list and closes only the preview");
        fake.Preview = new JsonObject { ["package"] = new JsonObject { ["name"] = "Incomplete" } };
        Check(!await model.PickAsync() && !model.ModalOpen, "malformed preview is never reviewable or importable");

        var saved = new JsonObject { ["id"] = "custom", ["name"] = "Saved name", ["builtin"] = false };
        Check(!await model.ExportAnywayAsync() && !await model.ExportAsync(new() { ["id"] = "reviewer", ["builtin"] = true })
            && !await model.ExportAsync(new() { ["builtin"] = false }), "builtins, unsaved rows and unsolicited acknowledgements cannot export");
        fake.Export = new JsonObject { ["status"] = "secret_warning", ["findings"] = new JsonArray(new JsonObject { ["kind"] = "assignment", ["line"] = 2, ["excerpt"] = "token=••••" }) };
        Check(await model.ExportAsync(saved) && model.Findings is { Count: 1 } && model.ExportName == "Saved name"
            && fake.Calls[^1].Args["acknowledgeSecrets"]?.GetValue<bool>() == false, "initial saved-id export always asks the shared backend to inspect secrets");
        saved["id"] = "later edit"; fake.Export = new JsonObject { ["status"] = "cancelled" };
        Check(await model.ExportAnywayAsync() && fake.Calls[^1].Args["id"]?.GetValue<string>() == "custom"
            && fake.Calls[^1].Args["acknowledgeSecrets"]?.GetValue<bool>() == true && !model.ModalOpen && model.NoticeKey == null, "explicit acknowledgement remains bound to reviewed identity; file dialog cancellation is not success");
        fake.Export = new JsonObject { ["status"] = "exported", ["path"] = "C:/export/example.wisp-specialist.json" };
        Check(await model.ExportAsync(saved) && model.NoticeKey == "specialists.export.done", "export success requires a path");
        fake.Export = new JsonObject { ["status"] = "exported" };
        Check(!await model.ExportAsync(saved) && model.NoticeKey == null, "malformed export is not success");

        fake.Pending = new(); var chat = model.CreateChatAsync("Shared localized prompt"); count = fake.Calls.Count;
        Check(!await model.CreateChatAsync("Duplicate") && fake.Calls.Count == count, "chat entry creates only one owned conversation at a time");
        fake.Pending.SetResult(JsonValue.Create("new-session"));
        Check(await chat && model.Chat == new NativeSpecialistChat("owned-project", "new-session", "Shared localized prompt")
            && fake.Calls[^1].Command == "native_conversation_create", "chat result binds the shared prompt to the exact created session");
        using var home = new NativeSpecialistTransferModel(fake, null);
        Check(!await home.CreateChatAsync("prompt"), "projectless settings cannot create an unscoped conversation");
        fake.Pending = new(); var late = model.CreateChatAsync("Late"); model.Dispose(); fake.Pending.SetResult(JsonValue.Create("late-session"));
        Check(!await late && model.Chat == null, "closed settings cannot navigate on a late session reply");
        var delayed = new Fake { Path = "C:/late.json", PendingPreview = new() }; using var closing = new NativeSpecialistTransferModel(delayed, "p");
        var previewing = closing.PickAsync(); closing.Dispose(); delayed.PendingPreview.SetResult(Preview());
        Check(!await previewing && !closing.ModalOpen, "late package previews cannot reopen a closed settings overlay");
    }
    private static JsonObject Preview() => new()
    {
        ["package"] = new JsonObject { ["format"] = "wisp-specialist", ["version"] = 1, ["name"] = "Example", ["instructions"] = "Read the evidence.", ["skills"] = null, ["connectors"] = new JsonArray() },
        ["file_name"] = "review.wisp-specialist.json", ["bytes"] = 42L, ["skills_checked"] = true, ["name_conflict"] = null,
        ["missing_skills"] = new JsonArray(), ["missing_connectors"] = new JsonArray(),
        ["model_matches"] = new JsonArray(new JsonObject { ["id"] = "other-provider", ["same_provider"] = false }, new JsonObject { ["id"] = "same-provider", ["same_provider"] = true })
    };
    private sealed class Fake : INativeSettingsClient
    {
        public string? Path;
        public JsonObject Preview = NativeSpecialistTransferTests.Preview(), Export = new() { ["status"] = "cancelled" };
        public TaskCompletionSource<JsonNode?>? Pending, PendingPreview;
        public List<(string Command, JsonObject Args, string? Project)> Calls = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            if (PendingPreview != null && command == "preview_specialist_import") return PendingPreview.Task;
            if (Pending != null && command is "import_specialist" or "native_conversation_create") return Pending.Task;
            return Task.FromResult<JsonNode?>(command switch
            {
                "pick_specialist_package" => JsonValue.Create(Path),
                "preview_specialist_import" => Preview.DeepClone(),
                "import_specialist" => new JsonArray(new JsonObject { ["id"] = "imported" }),
                "export_specialist" => Export.DeepClone(),
                "native_conversation_create" => JsonValue.Create("created"),
                _ => throw new InvalidOperationException(command)
            });
        }
    }
}
