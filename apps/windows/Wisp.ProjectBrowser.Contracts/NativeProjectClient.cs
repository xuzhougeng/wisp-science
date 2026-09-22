using System.Text.Json;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser.Contracts;

/// <summary>
/// Project creation for a native shell. The call carries no project id and is not
/// retried: a lost response may already have created the workspace.
/// </summary>
public interface INativeProjectClient
{
    Task<ProjectSummary> CreateAsync(string name, string workspaceDirectory, string description, string agentContext, bool standardLayout, CancellationToken cancellationToken = default);
    Task<ProjectSummary> ImportAsync(string archivePath, CancellationToken cancellationToken = default);
}

public sealed class NativeProjectClient(INativeSettingsClient transport) : INativeProjectClient
{
    public async Task<ProjectSummary> CreateAsync(string name, string workspaceDirectory, string description, string agentContext, bool standardLayout, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_project_create", new JsonObject
        {
            ["name"] = name,
            ["workspace_dir"] = workspaceDirectory,
            ["description"] = description,
            ["agent_context"] = agentContext,
            ["standard_layout"] = standardLayout,
        }, null, cancellationToken).ConfigureAwait(false) ?? throw new InvalidDataException("Missing created project");
        var project = node.Deserialize<ProjectSummary>() ?? throw new InvalidDataException("Missing created project");
        if (string.IsNullOrEmpty(project.Id)) throw new InvalidDataException("Missing created project id");
        return project;
    }

    public async Task<ProjectSummary> ImportAsync(string archivePath, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_project_import", new JsonObject
        {
            ["archive_path"] = archivePath,
        }, null, cancellationToken).ConfigureAwait(false) ?? throw new InvalidDataException("Missing imported project");
        var project = node.Deserialize<ProjectSummary>() ?? throw new InvalidDataException("Missing imported project");
        if (string.IsNullOrEmpty(project.Id)) throw new InvalidDataException("Missing imported project id");
        return project;
    }
}
