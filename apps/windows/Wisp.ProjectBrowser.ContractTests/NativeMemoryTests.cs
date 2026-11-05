using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeMemoryTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var fake = new Fake(); using var model = new NativeMemoryModel(fake, "b");
        Check(await model.LoadAsync() && model.ProjectId == "b" && model.Files.Count == 0 && model.TodayFile == "2026-10-29.md", "memory opens the requested project using the host's daily note name");
        Check(fake.Calls[^1].Project == "b" && fake.Calls[^1].Args["projectId"]?.GetValue<string>() == "b", "memory reads carry the same explicit project in the context and arguments");
        Check(await model.SelectProjectAsync("a") && model.ProjectId == "a", "memory can browse another project without an activation command");
        fake.Fail = "get_memory_view";
        Check(!await model.SelectProjectAsync("b") && model.ProjectId == "a" && model.Error != null, "failed selection preserves the last confirmed project and exposes the error");
        fake.Fail = null;
        fake.WrongProject = true;
        Check(!await model.SelectProjectAsync("b") && model.ProjectId == "a", "a response for the wrong project never replaces the confirmed memory view");
        fake.WrongProject = false; fake.Fail = "read_memory_file";
        Check(!await model.OpenNoteAsync(model.TodayFile) && !model.HasDraft, "failed note reads do not replace content with a writable empty draft");
        fake.Fail = null;
        Check(await model.OpenNoteAsync(model.TodayFile) && !model.HasChanges && model.Content.Contains("\nsecond"), "opening a note preserves multiline content without dirtying it");
        model.EditContent("temporary edit"); model.EditContent("first\nsecond");
        Check(!model.HasChanges && model.HasDraft, "restoring the loaded text clears unsaved changes without closing the note");
        var calls = fake.Calls.Count;
        Check(!await model.SelectProjectAsync("b") && !await model.LoadAsync() && fake.Calls.Count == calls, "mounted drafts cannot be replaced by project changes or reloads");
        model.EditContent("edited\nsecond\n"); fake.Fail = "write_memory_file";
        Check(!await model.SaveAsync() && model.HasChanges && model.NoteName == model.TodayFile && model.Content == "edited\nsecond\n", "ambiguous note writes keep the exact multiline draft without replaying");
        fake.Fail = null; fake.PendingCommand = "write_memory_file"; fake.Pending = new();
        var save = model.SaveAsync(); calls = fake.Calls.Count;
        Check(model.Busy && !await model.SaveAsync() && fake.Calls.Count == calls, "a pending note save cannot be submitted twice");
        fake.Pending.SetResult(new JsonArray(new JsonObject { ["name"] = model.TodayFile, ["bytes"] = 14 })); await save; fake.Pending = null;
        Check(!model.HasChanges && model.HasDraft && model.NoteName == "2026-10-29.md" && model.Files.Count == 1 && model.Feedback == "已保存。", "confirmed note saves update the file list and keep the editor open");
        Check(fake.Calls[^1].Project == "a" && fake.Calls[^1].Args["projectId"]?.GetValue<string>() == "a" && fake.Calls[^1].Args["content"]?.GetValue<string>() == "edited\nsecond\n", "note saves retain whitespace and their original project");
        model.Discard(); Check(!model.HasDraft && model.EditGlobal(), "global habits start as independent inline drafts"); calls = fake.Calls.Count;
        model.EditContent(" \n "); Check(!model.CanSave && !await model.SaveAsync() && fake.Calls.Count == calls, "blank global habits are not sent to the host");
        model.EditContent("  Use evidence\nKeep citations  "); await model.SaveAsync();
        Check(!model.HasDraft && model.GlobalMemories.Count == 2 && model.GlobalMemories[0]["content"]?.GetValue<string>() == "Use evidence\nKeep citations", "new habits trim outer whitespace and use authoritative returned identities");
        Check(fake.Calls[^1].Command == "create_global_memory" && !fake.Calls[^1].Args.ContainsKey("id"), "new global habits let the host assign their identity");
        Check(model.EditGlobal("existing"), "existing habits open in place"); model.EditContent("Revised habit");
        Check(await model.SaveAsync() && !model.HasDraft && model.GlobalMemories.Single(row => row["id"]?.GetValue<string>() == "existing")["content"]?.GetValue<string>() == "Revised habit", "void update acknowledgements confirm habit edits");
        Check(model.GlobalMemories.Single(row => row["id"]?.GetValue<string>() == "existing")["source_turn_index"]?.GetValue<int>() == 3, "habit updates preserve provenance fields");
        Check(model.EditGlobal("existing"), "populated row opens its own habit draft");
        var multilineHabit = "Line one\n\n  Keep this indentation\n证据先于结论";
        model.EditContent(multilineHabit); fake.Fail = "update_global_memory";
        var updatesBeforeFailure = fake.Calls.Count(call => call.Command == "update_global_memory");
        Check(!await model.SaveAsync() && model.GlobalId == "existing" && model.Content == multilineHabit && model.HasChanges
            && model.GlobalMemories.Single(row => row["id"]?.GetValue<string>() == "existing")["content"]!.GetValue<string>() == "Revised habit"
            && fake.Calls.Count(call => call.Command == "update_global_memory") == updatesBeforeFailure + 1,
            "failed habit edits retain exact multiline drafts and the prior visible row without replay");
        fake.Fail = null;
        Check(await model.SaveAsync() && !model.HasDraft
            && model.GlobalMemories.Single(row => row["id"]?.GetValue<string>() == "existing")["content"]!.GetValue<string>() == multilineHabit
            && model.GlobalMemories.Single(row => row["id"]?.GetValue<string>() == "existing")["source_turn_index"]!.GetValue<int>() == 3,
            "explicit retry updates only that row and preserves whitespace, identity and provenance");
        Check(await model.DeleteGlobalAsync("existing") && model.GlobalMemories.Count == 1, "confirmed deletion removes only its matching global habit");
        Check(await model.SetEnabledAsync(false) && !model.Enabled, "memory enable state follows the returned host view");
        await model.OpenNoteAsync(model.TodayFile); model.EditContent("unsaved");
        Check(await model.DeleteNoteAsync() && !model.HasDraft && model.Files.Count == 0, "confirmed note deletion returns to the collection");
        Check(await model.ClearAsync() && model.Files.Count == 0, "clear memory uses its authoritative returned list");
        Check(fake.Calls.All(call => call.Command is not ("open_project" or "activate_project")), "browsing memory never changes the active conversation project");
        var delayed = new Fake { PendingCommand = "get_memory_view", Pending = new() }; var closed = new NativeMemoryModel(delayed, "a");
        var load = closed.LoadAsync(); closed.Dispose(); delayed.Pending.SetResult(Fake.View("a")); await load;
        Check(closed.View == null && closed.Projects.Count == 0, "late memory responses cannot repopulate a disposed page");
        using var empty = new NativeMemoryModel(new Fake { EmptyProjects = true }, null);
        Check(await empty.LoadAsync() && empty.View == null && !empty.EditGlobal() && !empty.CanSave, "an empty project catalog never invents a writable target");
        var restored = new Fake(); using var home = new NativeMemoryModel(restored, null);
        Check(await home.LoadAsync() && home.ProjectId == "b" && restored.Calls.Any(c => c.Command == "get_project_info")
            && restored.Calls[^1].Project == "b" && restored.Calls[^1].Args["projectId"]!.GetValue<string>() == "b",
            "Home memory opens the host-restored workspace, not the first catalog entry");
        restored.RestoredProject = "a";
        Check(await home.LoadAsync() && home.ProjectId == "b" && restored.Calls.Count(c => c.Command == "get_project_info") == 1,
            "reloading retains the page's browsing project even when the host's workspace changes");
        Check(await home.SelectProjectAsync("a") && home.ProjectId == "a" && restored.Calls.All(c => c.Command != "activate_project"),
            "the project picker remains a read-only browsing scope after restoration");
        foreach (var identity in new JsonNode?[] { null, JsonValue.Create(""), JsonValue.Create("  "), JsonValue.Create(42) })
        {
            var missing = new Fake { RestoredProject = identity }; using var unavailable = new NativeMemoryModel(missing, null);
            Check(!await unavailable.LoadAsync() && unavailable.View == null && !unavailable.EditGlobal() && !unavailable.CanSave
                && missing.Calls.All(c => c.Command != "get_memory_view"), "invalid restored identity never falls back to another project's memories");
        }
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>
        {
            ["en"] = new() { ["memory.save"] = "Save note", ["memory.global_created"] = "Created next turn", ["memory.global_updated"] = "Updated next turn", ["memory.global_deleted"] = "Deleted next turn" },
            ["zh"] = new() { ["memory.global_created"] = "全局习惯已新增" }
        });
        using var localized = new NativeMemoryModel(new Fake(), "b", locale);
        Check(await localized.LoadAsync() && await localized.OpenNoteAsync(localized.TodayFile) && await localized.SaveAsync() && localized.Feedback == "Save note", "note acknowledgement uses shared locale copy");
        localized.Discard(); Check(localized.EditGlobal(), "localized global draft opens"); localized.EditContent("User text stays unchanged");
        Check(await localized.SaveAsync() && localized.Feedback == "Created next turn" && localized.GlobalMemories[0]["content"]!.GetValue<string>() == "User text stays unchanged", "new habit feedback is translated without translating memory content");
        Check(localized.EditGlobal("existing"), "localized existing habit opens"); localized.EditContent("Revised text");
        Check(await localized.SaveAsync() && localized.Feedback == "Updated next turn" && await localized.DeleteGlobalAsync("existing") && localized.Feedback == "Deleted next turn", "create, update and delete retain distinct timing feedback");
        locale.Preview("zh"); localized.EditGlobal(); localized.EditContent("中文内容");
        Check(await localized.SaveAsync() && localized.Feedback == "全局习惯已新增", "subsequent feedback follows the current interface locale");
        var failed = new Fake(); using var failedEditor = new NativeMemoryModel(failed, "b", locale);
        await failedEditor.LoadAsync(); failedEditor.EditGlobal("existing"); failedEditor.EditContent("Unconfirmed draft"); failed.Fail = "update_global_memory";
        Check(!await failedEditor.SaveAsync() && failedEditor.Error == "操作结果未能确认；不会自动重试。\nlost acknowledgement", "Chinese failure notice retains the original host detail");
        locale.Preview("en");
        Check(!await failedEditor.SaveAsync() && failedEditor.Error == "The operation could not be confirmed; it will not be retried automatically.\nlost acknowledgement"
            && failedEditor.Content == "Unconfirmed draft" && failedEditor.GlobalId == "existing", "English failure notice follows the current language without translating details or discarding edits");
        Check(!NativeMemoryLayout.UseTwoColumns(1100) && NativeMemoryLayout.UseTwoColumns(1101) && !NativeMemoryLayout.UseTwoColumns(double.NaN), "memory collection columns follow the WebView breakpoint");
        Check(NativeMemoryLayout.EditorHeight(500) == 460 && NativeMemoryLayout.EditorHeight(200) == 240 && NativeMemoryLayout.EditorHeight(double.NaN) == 240, "note editor reserves top padding and the footer gap without subtracting the separately reserved footer twice");
        Check(NativeMemoryLayout.FormatBytes(512) == "512 B" && NativeMemoryLayout.FormatBytes(1536) == "1.5 KB", "note rows expose readable sizes");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public string? Fail, PendingCommand; public TaskCompletionSource<JsonNode?>? Pending; public bool EmptyProjects, WrongProject;
        public JsonNode? RestoredProject = JsonValue.Create("b");
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public static JsonObject View(string project, bool enabled = true) => new()
        {
            ["project_id"] = project, ["project_name"] = project.ToUpperInvariant(), ["today_file"] = "2026-10-29.md", ["enabled"] = enabled,
            ["files"] = new JsonArray(), ["global_memories"] = new JsonArray(new JsonObject { ["id"] = "existing", ["content"] = "Old habit", ["source_turn_index"] = 3 })
        };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            if (command == Fail) return Task.FromException<JsonNode?>(new IOException("lost acknowledgement"));
            if (command == PendingCommand && Pending != null) return Pending.Task;
            JsonNode? result = command switch
            {
                "list_projects" => EmptyProjects ? new JsonArray() : new JsonArray(new JsonObject { ["id"] = "a", ["name"] = "A" }, new JsonObject { ["id"] = "b", ["name"] = "B" }),
                "get_project_info" => new JsonObject { ["id"] = RestoredProject?.DeepClone() },
                "get_memory_view" => View(WrongProject ? "wrong-project" : projectId!),
                "set_memory_enabled" => View(projectId!, arguments["enabled"]!.GetValue<bool>()),
                "read_memory_file" => JsonValue.Create("first\nsecond"),
                "write_memory_file" or "delete_memory_file" or "clear_memory" => new JsonArray(),
                "create_global_memory" => new JsonObject { ["id"] = "created", ["content"] = arguments["content"]!.DeepClone() },
                "update_global_memory" or "delete_global_memory" => null,
                _ => throw new InvalidOperationException(command)
            };
            return Task.FromResult(result);
        }
    }
}
