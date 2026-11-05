using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// The WebView welcome copy rotates independently; hiding its tip lasts until restart.
public sealed class NativeWelcomeModel
{
    public const int TitleCount = 7, SubtitleCount = 7, TipCount = 10;
    public event Action? Changed;
    public bool TipsEnabled { get; private set; } = true;
    public bool TipsDismissed { get; private set; }
    public bool ShowTips => TipsEnabled && !TipsDismissed;
    public int TitleIndex { get; private set; } = Random.Shared.Next(TitleCount);
    public int SubtitleIndex { get; private set; } = Random.Shared.Next(SubtitleCount);
    public int TipIndex { get; private set; } = Random.Shared.Next(TipCount);
    public string Title(NativeLocalization locale) => locale.Text($"empty.title.{TitleIndex}");
    public string Subtitle(NativeLocalization locale) => locale.Text($"empty.subtitle.{SubtitleIndex}");
    public string Tip(NativeLocalization locale) => locale.Format($"tip.{TipIndex}", ("modifier", "Ctrl"));
    public void ApplyCommittedSettings(JsonNode? settings)
    {
        if (settings is not JsonObject obj || obj["show_tips"] is not JsonValue value
            || !value.TryGetValue<bool>(out var enabled) || enabled == TipsEnabled) return;
        TipsEnabled = enabled; Changed?.Invoke();
    }
    public void DismissTips()
    {
        if (TipsDismissed) return;
        TipsDismissed = true; Changed?.Invoke();
    }
    public void Rotate()
    {
        TitleIndex = Random.Shared.Next(TitleCount);
        SubtitleIndex = Random.Shared.Next(SubtitleCount);
        TipIndex = Random.Shared.Next(TipCount);
    }
}
