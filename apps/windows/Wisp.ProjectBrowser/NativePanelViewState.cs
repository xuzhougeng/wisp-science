namespace Wisp.ProjectBrowser;

/// <summary>Reading state owned by one mounted session panel, never shared across sessions.</summary>
public sealed class NativePanelViewState
{
    private readonly Dictionary<(string Tab, string Directory), ReadingState> states = [];
    public ReadingState For(string tab, string directory = ".")
    {
        var key = (tab, tab == "files" ? directory : "");
        if (!states.TryGetValue(key, out var state)) states[key] = state = new();
        return state;
    }

    public sealed class ReadingState
    {
        public string Filter { get; private set; } = "";
        public double Offset { get; private set; }
        public void SetFilter(string value)
        {
            if (Filter == value) return;
            Filter = value;
            Offset = 0;
        }
        public void SetOffset(double value) => Offset = double.IsFinite(value) ? Math.Max(0, value) : 0;
    }
}
