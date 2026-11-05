using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed class WorkspaceExplorationStart(INativeSettingsClient client, NativeHistoryTarget target, string name) : WorkspaceActionModel
{
    public string Name { get; set; } = name;
    public NativeExplorationSummary? Result { get; private set; }
    public bool Unconfirmed { get; private set; }
    public bool CanSubmit => !Closed && !Busy && !Unconfirmed && Result == null && !string.IsNullOrWhiteSpace(Name);
    public bool CanDismiss => !Busy;
    protected override string UnconfirmedFailure(Exception exception) => exception.Message;
    public Task<bool> CreateAsync()
    {
        if (!CanSubmit) return Task.FromResult(false);
        var submittedName = Name.Trim();
        return RunAsync(async () =>
        {
            try
            {
                var data = await client.InvokeAsync("native_conversation_exploration_start", new JsonObject
                {
                    ["session_id"] = target.SessionId,
                    ["target"] = JsonSerializer.SerializeToNode(target.Turn, ConversationSnapshot.JsonOptions),
                    ["name"] = submittedName,
                }, target.ProjectId);
                return NativeExplorationStarted.Decode(data, target);
            }
            catch { Unconfirmed = true; throw; }
        }, result => Result = result);
    }
    public static string DefaultName(NativeLocalization locale, int candidates) =>
        locale.Text("exploration.default_name").Replace("{n}", (Math.Max(0, candidates) + 1).ToString());
}
