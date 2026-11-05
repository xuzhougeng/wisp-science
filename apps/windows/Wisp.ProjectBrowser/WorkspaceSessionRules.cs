using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Explicitly scoped, one-shot rules reload; a lost acknowledgement is never replayed.</summary>
public sealed class WorkspaceSessionRules(INativeSettingsClient client, string projectId, string sessionId) : WorkspaceActionModel
{
    public bool MutationUnconfirmed { get; private set; }
    public NativeSessionRulesResult? Result { get; private set; }
    public bool CanReload => !Closed && !Busy && !MutationUnconfirmed && Result == null;
    protected override string UnconfirmedFailure(Exception exception) => exception.Message;

    public static async Task<bool> ReadStaleAsync(INativeSettingsClient client, string projectId, string sessionId)
    {
        var reply = (await client.InvokeAsync("native_conversation_rules_status", new() { ["session_id"] = sessionId }, projectId))?.Deserialize<NativeSessionRulesStatus>();
        if (reply == null || reply.ProjectId != projectId || reply.SessionId != sessionId)
            throw new InvalidDataException("Project rules status did not match the conversation");
        return reply.StalePrompt;
    }

    public Task<bool> ReloadAsync()
    {
        if (!CanReload) return Task.FromResult(false);
        return RunAsync(async () =>
        {
            try
            {
                var reply = (await client.InvokeAsync("native_conversation_rules_reload", new() { ["session_id"] = sessionId }, projectId))?.Deserialize<NativeSessionRulesResult>();
                if (reply == null || reply.ProjectId != projectId || reply.SessionId != sessionId)
                    throw new InvalidDataException("Rules reload acknowledgement did not match the conversation");
                return reply;
            }
            catch { MutationUnconfirmed = true; throw; }
        }, result => Result = result);
    }
}
