using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// One confirmation owns one explicit project. No mutation is replayed after a lost response.
public sealed class NativeProjectDeleteModel(INativeSettingsClient client, string projectId, TimeProvider? time = null) : WorkspaceActionModel
{
    private readonly TimeProvider clock = time ?? TimeProvider.System;
    private long confirmationStarted;
    public string ProjectId { get; } = projectId;
    public bool ConfirmingData { get; private set; }
    public bool Attempted { get; private set; }
    public bool Deleted { get; private set; }
    public int RemainingSeconds => ConfirmingData ? Math.Max(0, (int)Math.Ceiling(5 - clock.GetElapsedTime(confirmationStarted).TotalSeconds)) : 0;
    public bool CanDelete => !Closed && !Busy && !Attempted && (!ConfirmingData || RemainingSeconds == 0);
    public void ConfirmData()
    {
        if (Closed || Busy || Attempted || ConfirmingData) return;
        ConfirmingData = true; confirmationStarted = clock.GetTimestamp(); Notify();
    }
    public void Back()
    {
        if (Closed || Busy || Attempted) return;
        ConfirmingData = false; Notify();
    }
    protected override string UnconfirmedFailure(Exception exception) => exception.Message;
    public Task<bool> DeleteAsync()
    {
        if (!CanDelete || string.IsNullOrWhiteSpace(ProjectId)) return Task.FromResult(false);
        var deleteData = ConfirmingData;
        Attempted = true;
        return RunAsync(() => client.InvokeAsync("native_project_delete", new JsonObject { ["delete_data"] = deleteData }, ProjectId), result =>
        {
            if (result is not JsonObject data || data["project_id"]?.GetValue<string>() != ProjectId
                || data["delete_data"]?.GetValue<bool>() != deleteData || data["deleted"]?.GetValue<bool>() != true)
                throw new InvalidDataException("Project deletion acknowledgement did not match the selected project and choice.");
            Deleted = true;
        });
    }
}
