using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;

internal static class NativeLocalizationTests
{
    public static void Run()
    {
        var strings = new Dictionary<string, Dictionary<string, string>>
        {
            ["en"] = new() { ["title"] = "Settings", ["message"] = "Open {name}: {other}", ["fallback"] = "English fallback" },
            ["zh"] = new() { ["title"] = "设置", ["message"] = "打开 {name}：{other}" }
        };
        var locale = new NativeLocalization(strings);
        Check(locale.Text("title") == "Settings", "default matches WebView English locale");
        var committed = new JsonObject { ["locale"] = "en", ["unrelated"] = "unchanged" };
        var draft = (JsonObject)committed.DeepClone(); draft["locale"] = "zh";
        Check(locale.Text("title") == "Settings", "editing a preference draft alone does not change mounted UI language");
        Check(locale.ApplyCommittedSettings(draft) && locale.Text("title") == "设置", "confirmed settings change selects Chinese");
        Check(!locale.ApplyCommittedSettings(new JsonObject()) && !locale.ApplyCommittedSettings(new JsonObject { ["locale"] = 7 })
            && locale.Locale == "zh", "missing or malformed optional read preserves current language");
        Check(locale.Text("fallback") == "English fallback" && locale.Text("missing") == "missing", "missing translations have deterministic fallbacks");
        Check(locale.Format("message", ("name", "{other}"), ("other", "Settings")) == "打开 {other}：Settings",
            "user values are inserted verbatim and never retranslated or recursively substituted");
        Check(NativeLocalization.Normalize(" ZH-cn ") == "zh" && NativeLocalization.Normalize("zh_TW") == "zh"
            && NativeLocalization.Normalize("fr") == "en", "locale tags use the WebView-supported language set");
        Check(committed["locale"]!.GetValue<string>() == "en" && committed["unrelated"]!.GetValue<string>() == "unchanged",
            "localization never changes settings or persists drafts");
        Check(locale.ApplyCommittedSettings(committed) && locale.Preview("zh") && locale.Text("title") == "设置",
            "explicit language selection previews immediately without writing settings");
        Check(locale.ApplyCommittedSettings(committed) && locale.Text("title") == "Settings" && draft["locale"]!.GetValue<string>() == "zh",
            "cancelling restores the committed language without mutating unrelated draft data");
        var sounds = JsonNode.Parse("[{\"id\":\"Notification.Mail\",\"label\":\"Mail\"},{\"id\":\"custom\",\"label\":\"用户命名\"}]");
        var englishSounds = NativeNotificationSounds.Choices(sounds, "retired", "en");
        Check(englishSounds[0].Label == "System default" && englishSounds[1].Label == "Mail"
            && englishSounds[2].Label == "用户命名" && englishSounds[3].Id == "retired",
            "known sound labels follow locale while custom labels and saved IDs stay unchanged");
        strings["en"]["selection.research_literature"] = "Research literature";
        strings["zh"]["selection.research_literature"] = "文献调研";
        var action = new JsonObject { ["id"] = "literature_research", ["name"] = "Research literature" };
        locale.Preview("zh");
        Check(NativeQuickActions.Label(action, locale) == "文献调研", "the exact stock literature label follows the shared locale");
        action["name"] = "My renamed action";
        Check(NativeQuickActions.Label(action, locale) == "My renamed action", "renamed built-in action labels remain user text");
        action["id"] = "custom"; action["name"] = "Research literature";
        Check(NativeQuickActions.Label(action, locale) == "Research literature", "custom action names are not translated by coincidence");
        strings["en"]["composer.reviewer.default_http"] = "Default HTTP model";
        strings["en"]["composer.reviewer.follow_session"] = "Follow session backend";
        strings["en"]["specialists.model.follow"] = "Follow active model";
        locale.Preview("en");
        var reviewer = new JsonObject { ["id"] = "reviewer", ["model_id"] = "retired" };
        var options = NativeSpecialists.BackendOptions(reviewer,
            [new JsonObject { ["id"] = "chat", ["model"] = "chat", ["label"] = "用户命名" }], [], locale);
        Check(options.Any(o => o.Id == "http:" && o.Label == "Default HTTP model")
            && options.Any(o => o.Id == "follow_session" && o.Label == "Follow session backend")
            && options.Any(o => o.Id == "http:chat" && o.Label == "用户命名")
            && options.Any(o => o.Id == "http:retired" && !o.Available && o.Label.EndsWith("http:retired")),
            "localized backend controls preserve model labels, stable keys and unavailable references");
        Check(NativeSpecialists.BackendOptions(new JsonObject { ["id"] = "custom" }, [], [], locale)[0].Label == "Follow active model",
            "ordinary specialists use the same follow-active-model caption as WebView");
        strings["en"]["sidebar.untitled"] = "Untitled session";
        strings["en"]["time.just_now"] = "just now";
        strings["en"]["time.minutes"] = "{n}m";
        strings["en"]["time.hours"] = "{n}h";
        strings["en"]["time.days"] = "{n}d";
        var now = DateTimeOffset.FromUnixTimeSeconds(1793280000);
        Check(NativeBrowserPresentation.RelativeTime(now.AddSeconds(-30).ToUnixTimeSeconds(), now, locale: locale) == "just now"
            && NativeBrowserPresentation.RelativeTime(now.AddMinutes(-8).ToUnixTimeSeconds(), now, locale: locale) == "8m"
            && NativeBrowserPresentation.RelativeTime(now.AddHours(-3).ToUnixTimeSeconds(), now, locale: locale) == "3h"
            && NativeBrowserPresentation.RelativeTime(now.AddDays(-2).ToUnixTimeSeconds(), now, locale: locale) == "2d",
            "project recency uses shared locale keys at each time boundary");
        Check(NativeBrowserPresentation.SessionTitle("用户命名 · Settings", locale) == "用户命名 · Settings"
            && NativeBrowserPresentation.SessionTitle(" Selected skills: data ", locale) == "Untitled session",
            "only generated empty-title fallbacks are localized, not user titles");
        strings["en"]["composer.hint"] = "Enter to send · Shift+Enter for newline · Drag top edge to resize · Drop files to attach";
        strings["en"]["composer.hint_modifier"] = "{modifier}+Enter to send · Enter for newline · Drag top edge to resize · Drop files to attach";
        strings["zh"]["composer.hint_modifier"] = "{modifier}+Enter 发送 · Enter 换行 · 拖拽上边缘调整高度 · 拖放文件可附加";
        Check(new NativeInputPreferences().LocalizedHint(locale) == strings["en"]["composer.hint"]
            && new NativeInputPreferences(true).LocalizedHint(locale) == "Ctrl+Enter to send · Enter for newline · Drag top edge to resize · Drop files to attach",
            "both Windows send modes include the shared resize and attachment hints");
        locale.Preview("zh");
        Check(new NativeInputPreferences(true).LocalizedHint(locale).StartsWith("Ctrl+Enter 发送") &&
            new NativeInputPreferences(true).LocalizedHint(locale).Contains("拖拽上边缘"), "modifier and resize hints follow a locale change");
        locale.Preview("en");
        var welcome = new NativeWelcomeModel();
        for (var i = 0; i < NativeWelcomeModel.TitleCount; i++) strings["en"][$"empty.title.{i}"] = "Title " + i;
        for (var i = 0; i < NativeWelcomeModel.SubtitleCount; i++) strings["en"][$"empty.subtitle.{i}"] = "Subtitle " + i;
        for (var i = 0; i < NativeWelcomeModel.TipCount; i++) strings["en"][$"tip.{i}"] = "Tip {modifier}";
        welcome.Rotate();
        Check(welcome.Title(locale).StartsWith("Title ") && welcome.Subtitle(locale).StartsWith("Subtitle ")
            && welcome.Tip(locale) == "Tip Ctrl", "rotating welcome copy stays within the shared catalog and uses the Windows modifier");
        var visibilityChanges = 0; welcome.Changed += () => visibilityChanges++;
        welcome.ApplyCommittedSettings(new JsonObject { ["show_tips"] = false });
        Check(!welcome.ShowTips && visibilityChanges == 1, "saved disabled tips are hidden before display");
        welcome.ApplyCommittedSettings(new JsonObject { ["show_tips"] = "invalid" });
        Check(!welcome.ShowTips && visibilityChanges == 1, "malformed optional preferences retain the last committed setting");
        welcome.ApplyCommittedSettings(new JsonObject { ["show_tips"] = true });
        welcome.DismissTips(); welcome.DismissTips(); welcome.Rotate();
        Check(!welcome.ShowTips && welcome.TipsEnabled && visibilityChanges == 3 && new NativeWelcomeModel().ShowTips,
            "tip dismissal survives new welcome screens until restart, without changing saved preferences");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
}
