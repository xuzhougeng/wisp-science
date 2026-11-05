using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspaceConversationModel
{
    public bool ExampleMode { get; private set; }
    public static WorkspaceConversationModel CreateExampleViewer()
    {
        var transport = new ExampleTransport();
        return new(new NativeConversationClient(transport), transport) { ExampleMode = true };
    }
    public void ShowExample(ConversationSnapshot snapshot)
    {
        if (!ExampleMode || !snapshot.ReadOnly || snapshot.ProjectId != "example" || snapshot.Running || snapshot.Approvals.Length != 0)
            throw new ArgumentException("Expected a read-only bundled example", nameof(snapshot));
        Reset();
        Snapshot = snapshot; projectId = snapshot.ProjectId; sessionId = snapshot.SessionId;
        // Remain inactive: no polling, mark-seen, runtime probe or composer lookup.
        Notify();
    }
    private sealed class ExampleTransport : INativeSettingsClient
    {
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
            => throw new InvalidOperationException("Example recordings are read-only");
    }
}
