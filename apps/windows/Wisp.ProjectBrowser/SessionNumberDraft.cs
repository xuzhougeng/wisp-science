using System.Globalization;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// <summary>Keeps unfinished editor text out of the settings sent to the host.</summary>
public sealed class SessionNumberDraft
{
    private readonly JsonObject settings;
    private readonly string key;
    public ulong Minimum { get; }
    public ulong Maximum { get; }
    public bool Enabled { get; private set; } = true;
    public bool Invalid { get; private set; }
    public string Text { get; private set; }
    private string ValidText => settings[key]?.ToString() ?? Minimum.ToString(CultureInfo.InvariantCulture);

    public SessionNumberDraft(JsonObject settings, string key)
    {
        this.settings = settings;
        this.key = key;
        (Minimum, Maximum) = key switch
        {
            "max_iter" => (0UL, (ulong)long.MaxValue),
            "auto_continue_limit" => (1UL, ulong.MaxValue),
            "auto_compact_percent" => (10UL, 95UL),
            "semantic_compact_idle_hours" => (0UL, ulong.MaxValue),
            _ => throw new ArgumentOutOfRangeException(nameof(key))
        };
        Text = ValidText;
    }

    public void Edit(string text)
    {
        if (!Enabled) return;
        Text = text;
        Invalid = !ulong.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture, out var n) || n < Minimum || n > Maximum;
        if (!Invalid) settings[key] = key == "max_iter" ? JsonValue.Create((long)n) : JsonValue.Create(n);
    }

    public void SetEnabled(bool enabled)
    {
        Enabled = enabled;
        if (!enabled) { Text = ValidText; Invalid = false; }
    }
}
