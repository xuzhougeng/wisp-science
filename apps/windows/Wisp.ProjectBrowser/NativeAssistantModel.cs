using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed class NativeAssistantModel
{
    private readonly NativeAssistantClient client;
    private readonly INativeCalendarClient calendar;
    public WorkspaceConversationModel Conversation { get; }
    public NativeAssistantLayout Layout { get; }
    public event Action? Changed;
    public AssistantWorkspace? Workspace { get; private set; }
    public IReadOnlyList<CalendarProject> Activity { get; private set; } = [];
    public DateTime Day { get; private set; } = DateTime.Today;
    public DateTime Month => new(Day.Year, Day.Month, 1);
    public string? Error { get; private set; }
    public string? CalendarError { get; private set; }
    public bool Busy { get; private set; }
    public bool Active { get; private set; }
    public string? SelectedProjectId => Conversation.ContextReference?.Reference.Id;
    private int generation;
    private readonly INativeSettingsClient transport;
    public NativeAssistantRemoteModel CreateRemote() => new(transport);
    public NativeAutomationModel CreateAutomation() => new(new NativeAssistantClient(transport));
    public WorkspacePanelModel CreateFiles() => new(new NativePanelClient(transport), NativeAssistantClient.ProjectId,
        NativeAssistantClient.SessionId, new NativePanelTabs("[\"files\"]", "files", ["files"]), files: new NativeFilesClient(transport));
    public WorkspaceShareModel CreateShare() => new(new NativeShareClient(transport), NativeAssistantClient.ProjectId, NativeAssistantClient.SessionId);
    public NativeAssistantModel(INativeSettingsClient transport, NativeAssistantLayout? layout = null)
    {
        Layout = layout ?? new();
        this.transport = transport;
        client = new(transport); calendar = new NativeCalendarClient(transport);
        Conversation = new(new NativeConversationClient(transport), transport) { AssistantMode = true };
        Conversation.SetAssistantContext(null, false);
    }
    public async Task OpenAsync()
    {
        Active = true; var current = ++generation;
        Conversation.Reset(); Conversation.SetAssistantContext(null, false);
        Busy = true; Error = null; Workspace = null; Activity = []; Changed?.Invoke();
        try
        {
            await client.OpenAsync();
            if (!Active || current != generation) return;
            await ReadAsync(current);
            if (!Active || current != generation || Workspace == null) return;
            await Conversation.OpenAsync(NativeAssistantClient.ProjectId, NativeAssistantClient.SessionId);
        }
        catch (Exception ex) { if (Active && current == generation) Error = ex.Message; }
        finally { if (Active && current == generation) { Busy = false; Changed?.Invoke(); } }
    }
    public Task ReloadAsync() => SelectDayAsync(Day);
    public async Task SelectDayAsync(DateTime day)
    {
        if (!Active) return;
        Day = day.Date; var current = ++generation;
        await ReadAsync(current);
    }
    private async Task ReadAsync(int current)
    {
        var selected = SelectedProjectId;
        Busy = true; Error = CalendarError = null; Workspace = null; Activity = [];
        Conversation.SetAssistantContext(null, false); Changed?.Invoke();
        try
        {
            var workspace = await client.ReadAsync(Day);
            if (!Active || current != generation) return;
            Workspace = workspace;
            SelectProject(selected);
            var month = Month;
            try
            {
                var rows = workspace.Projects.Length == 0 ? [] : await calendar.ReadAsync(workspace.Projects.Select(p => p.Id).ToArray(),
                    WorkspaceCalendarModel.Unix(month), WorkspaceCalendarModel.Unix(month.AddMonths(1)));
                if (!Active || current != generation) return;
                // The server repeats the privacy check. Never display unexpected projects.
                if (rows.Any(row => !workspace.Projects.Any(p => p.Id == row.ProjectId))) throw new InvalidDataException("Calendar project visibility mismatch");
                Activity = rows;
            }
            catch (Exception ex) { if (Active && current == generation) CalendarError = ex.Message; }
        }
        catch (Exception ex) { if (Active && current == generation) Error = ex.Message; }
        finally { if (Active && current == generation) { Busy = false; Changed?.Invoke(); } }
    }
    public void SelectProject(string? id)
    {
        var project = Workspace?.Projects.FirstOrDefault(p => p.Id == id);
        Conversation.SetAssistantContext(project == null ? null : new(new("project", project.Id), project.Name, project.Description), Workspace != null);
        Changed?.Invoke();
    }
    public void DraftDay(bool chinese)
    {
        if (Workspace == null) return;
        var day = Day.ToString("yyyy-MM-dd", System.Globalization.CultureInfo.InvariantCulture);
        Conversation.Prefill(chinese ? $"请帮我安排 {day} 的研究计划。" : $"Help me plan my research for {day}.");
    }
    public void Close()
    {
        Active = false; generation++; Busy = false; Workspace = null; Activity = [];
        Conversation.Pause(); Conversation.SetAssistantContext(null, false);
    }
}

/// Desktop rail choices persist; temporary narrow drawers never overwrite them.
public sealed class NativeAssistantLayout
{
    public bool Narrow { get; private set; }
    public bool Left { get; private set; }
    public bool Right { get; private set; }
    public bool LastLeft { get; private set; } = true;
    private bool desktopLeft, desktopRight;
    private readonly Action<bool, bool>? persist;
    public NativeAssistantLayout(bool projects = true, bool calendar = true, Action<bool, bool>? persist = null)
    { Left = desktopLeft = projects; Right = desktopRight = calendar; this.persist = persist; }
    public void Resize(double width)
    {
        var narrow = width < 960;
        if (narrow == Narrow) return;
        Narrow = narrow; Left = !narrow && desktopLeft; Right = !narrow && desktopRight;
    }
    public void Toggle(bool left)
    {
        if (left) { Left = !Left; if (Left) LastLeft = true; if (!Narrow) desktopLeft = Left; }
        else { Right = !Right; if (Right) LastLeft = false; if (!Narrow) desktopRight = Right; }
        if (!Narrow) persist?.Invoke(desktopLeft, desktopRight);
    }
    public bool CloseDrawer()
    {
        if (!Narrow || !Left && !Right) return false;
        if (Left && (LastLeft || !Right)) Left = false; else Right = false;
        return true;
    }
    public static double LeftWidth(double width) => Math.Clamp(width * .17, 184, 256);
    public static double RightWidth(double width) => Math.Clamp(width * .24, 264, 350);
}
