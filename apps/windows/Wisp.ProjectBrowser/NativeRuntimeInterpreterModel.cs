using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeRuntimeInterpreterValues(string Python, string Rscript)
{
    // Mirrors RuntimeInterpreterForm::from_context: explicit config, legacy path,
    // then the latest probe. An explicitly empty first key falls back to probing.
    public static NativeRuntimeInterpreterValues From(string configJson, string capabilitiesJson)
    {
        JsonObject Parse(string value) { try { return JsonNode.Parse(value) as JsonObject ?? new(); } catch (JsonException) { return new(); } }
        var config = Parse(configJson); var capabilities = Parse(capabilitiesJson);
        string? Read(JsonObject value, params string[] keys)
        {
            foreach (var key in keys)
                if (value[key] is JsonValue v && v.TryGetValue<string>(out var text)) return string.IsNullOrWhiteSpace(text) ? null : text.Trim();
            return null;
        }
        return new(Read(config, "python_executable", "python_path") ?? Read(capabilities, "python_executable") ?? "",
            Read(config, "rscript_executable", "rscript_path") ?? Read(capabilities, "rscript_executable") ?? "");
    }
    public JsonObject Arguments(string contextId) => new()
    {
        ["contextId"] = contextId, ["pythonExecutable"] = Python, ["rscriptExecutable"] = Rscript
    };
}

/// A context-bound draft. Closing discards text and invalidates late picker/save replies.
public sealed class NativeRuntimeInterpreterModel(INativeSettingsClient client, string projectId, NativePanelContext context,
    Action<NativePanelContext>? accepted = null) : WorkspaceActionModel
{
    public NativePanelContext Context { get; } = context;
    public NativeRuntimeInterpreterValues Values { get; private set; } = NativeRuntimeInterpreterValues.From(context.ConfigJson, context.CapabilitiesJson);
    public bool Browsing { get; private set; }
    public bool Saved { get; private set; }
    public bool CanSave => !Busy && !Browsing && !Closed;
    public bool CanBrowse => CanSave && Context.Kind == "local";
    public string ContextLabel => string.IsNullOrWhiteSpace(Context.Label) ? Context.Id : Context.Label;
    public void SetPath(bool python, string value)
    {
        if (!CanSave) return;
        Values = python ? Values with { Python = value } : Values with { Rscript = value };
        Saved = false;
    }
    public async Task BrowseAsync(bool python, Func<Task<string?>> picker)
    {
        if (!CanBrowse) return;
        Browsing = true; ClearFailure(); Notify();
        try
        {
            var path = await picker();
            if (!Closed && !string.IsNullOrEmpty(path))
            {
                Values = python ? Values with { Python = path } : Values with { Rscript = path };
                Saved = false;
            }
        }
        catch (Exception ex) { if (!Closed) Fail(ex.Message); }
        finally { Browsing = false; if (!Closed) Notify(); }
    }
    public Task<bool> SaveAsync()
    {
        if (!CanSave) return Task.FromResult(false);
        Saved = false;
        return RunAsync(() => client.InvokeAsync("update_execution_context_interpreters", Values.Arguments(Context.Id), projectId), reply =>
        {
            var updated = reply?.Deserialize<NativePanelContext>(ConversationSnapshot.JsonOptions);
            if (updated == null || updated.Id != Context.Id) throw new InvalidDataException("Missing interpreter configuration acknowledgement");
            accepted?.Invoke(updated);
            Saved = true;
        });
    }
}
