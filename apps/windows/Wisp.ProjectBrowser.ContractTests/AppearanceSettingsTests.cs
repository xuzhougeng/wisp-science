using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class AppearanceSettingsTests
{
    public static async Task RunAsync()
    {
        await CachePrecedenceAsync();
        Check(NativeTypography.From(new()) == new NativeTypography(), "older preferences use native font defaults");
        var documentTypography = new NativeTypography(18, 12, "User UI font");
        Check(documentTypography.MarkdownFontSource("en").StartsWith("ms-appx:///Assets/Fonts/wisp-serif.ttf#Wisp Serif")
            && documentTypography.MarkdownFontSource("zh-CN") == documentTypography.FontSource()
            && documentTypography.MarkdownSize == 19,
            "skill Markdown uses the shared English serif/Chinese UI font rule and UI size plus one");
        Check(documentTypography.ResponseSize("en") == 19 && documentTypography.ResponseSize("zh-CN") == 19.5
            && documentTypography.ResponseSize("zh", commentary: true) == 18
            && documentTypography.ResponseLeading("en") == 1.62 && documentTypography.ResponseLeading("zh") == 1.78
            && documentTypography.ResponseLeading("zh", commentary: true) == 1.5 && documentTypography.ResponseTableSize == 17,
            "response text retains locale-specific CSS sizing and leading while commentary and tables use their own sizes");
        Check(new NativeTypography(UiSize: 0).ResponseTableSize > 0 && new NativeTypography(UiSize: 0).ResponseSize("zh") == 1.5
            && new NativeTypography(UiSize: 30).ResponseSize("zh") == 31.5,
            "response metrics stay renderable at zero and preserve large appearance values");
        var paragraphs = NativeSkillMarkdown.Parse("**Lead:** body\n\n  **Also lead**\n\nPrefix **bold**\n\n「**inside punctuation**」\n\n~~removed~~\n\n[**linked**](https://example.test)\n\n*italic*")
            .OfType<Markdig.Syntax.ParagraphBlock>().ToArray();
        Check(paragraphs.Select(NativeResponseMarkdown.HasStrongLead).SequenceEqual(new[] { true, true, false, false, false, false, false }),
            "lead bars follow the first strong inline rather than any bold phrase or unrelated emphasis");
        var codeText = NativeResponseMarkdown.CodeText("# Windows line\r\nvalue = 3\rprint(value)\n");
        var comment = NativeCodeHighlight.Tokenize(codeText, "python").First(token => token.Kind == NativeCodeHighlight.Comment);
        Check(codeText == "# Windows line\nvalue = 3\nprint(value)\n" && !codeText.Substring(comment.Start, comment.Length).Contains('\r'),
            "copied Markdown code uses HTML line endings and syntax runs cannot split a CRLF into two visual line breaks");
        Check(NativeResponseMarkdown.MinimumWidthWords("alpha 中文 beta_identifier").SequenceEqual(new[] { "alpha", "中", "文", "beta_identifier" }),
            "table minimum widths preserve Latin words while allowing normal CJK glyph wrapping");
        var links = Markdig.Markdown.Parse("[results/a.tsv](results/a.tsv) [**report**](file:///C:/work/report.md) [web](https://example.test) [mail](mailto:a@example.test) [anchor](#section) ![image](results/plot.png)")
            .OfType<Markdig.Syntax.ParagraphBlock>().Single().Inline!.OfType<Markdig.Syntax.Inlines.LinkInline>().ToArray();
        Check(links.Select(NativeResponseMarkdown.LocalLinkCode).SequenceEqual(new string?[] { "results/a.tsv", "report", null, null, null, null }),
            "unverified local links become inert code while external links, anchors and images retain their rendering");
        Check(new NativeTypography().FontSource().StartsWith("ms-appx:///Assets/Fonts/inter-latin.ttf#Inter, Microsoft YaHei UI"),
            "default UI uses the bundled WebView font and an explicit Chinese sans-serif fallback");
        Check(new NativeTypography().FontSource(true).StartsWith("ms-appx:///Assets/Fonts/jetbrains-mono-400.ttf#JetBrains Mono, Cascadia Mono"),
            "default code font resolves without a machine-wide font installation");
        var customFallback = new NativeTypography(UiFamily: "  'Missing Font', \"Inter\", inter, Arial ").FontSource();
        Check(customFallback.StartsWith("Missing Font, ms-appx:///Assets/Fonts/inter-latin.ttf#Inter, Arial, Microsoft YaHei UI")
            && customFallback.Split("#Inter").Length == 2,
            "custom font lists keep priority, remove CSS quotes, and retain deduplicated bundled fallbacks");
        var fonts = NativeTypography.From(new() { ["ui_font_size"] = 20, ["code_font_size"] = 10,
            ["ui_font_family"] = "  Arial  ", ["code_font_family"] = "Cascadia Code" });
        Check(Math.Abs(fonts.Scale(18) - 180d / 7) < 0.0001 && fonts.Scale(12, true) == 10,
            "heading proportions and independent code size survive appearance changes");
        Check(fonts.UiFamily == "Arial" && fonts.CodeFamily == "Cascadia Code", "custom native font families are retained");
        var invalid = NativeTypography.From(new() { ["ui_font_size"] = -5, ["code_font_size"] = "bad",
            ["ui_font_family"] = "  ", ["code_font_family"] = new JsonObject() });
        Check(invalid == new NativeTypography(), "malformed or empty native preferences fall back without breaking rendering");
        var bounded = NativeTypography.From(new() { ["ui_font_size"] = 999, ["code_font_size"] = 1 });
        Check(bounded.UiSize == 30 && bounded.CodeSize == 1, "native font preferences retain the WebView 0 to 30 range");
        var zero = NativeTypography.From(new() { ["ui_font_size"] = 0, ["code_font_size"] = 0 });
        Check(zero.UiSize == 0 && zero.CodeSize == 0 && zero.Scale(14) == .01 && zero.Scale(12, true) == .01,
            "zero wire sizes are retained while XAML receives a positive rendering size");
        Check(NativeTypography.From(new() { ["ui_font_size"] = 24, ["code_font_size"] = 28 }) is { UiSize: 24, CodeSize: 28 },
            "WebView large font values are not silently reduced when opened natively");
        Check(NativeTypography.From(JsonNode.Parse("{\"ui_font_size\":16.5,\"code_font_size\":13}")!.AsObject()).UiSize == 16.5,
            "wire numeric representation preserves fractional font sizes");
        var client = new Fake();
        var model = new AppearanceSettingsModel(client, "project-a");
        await model.LoadAsync();
        model.Draft!["theme"] = "dark";
        await model.LoadAsync();
        Check(model.Draft["theme"]!.GetValue<string>() == "dark", "refresh preserves unsaved draft");
        client.Fail = true;
        try { await model.SaveAsync(); throw new Exception("Expected failure"); } catch (IOException) { }
        Check(client.Writes == 1 && model.HasChanges && !model.Busy, "failed mutation retains draft without retry");
        client.Fail = false;
        client.Pending = new();
        var save = model.SaveAsync();
        await model.SaveAsync();
        Check(client.Writes == 2, "duplicate save blocked while busy");
        client.Pending.SetResult(client.Sent!.DeepClone());
        await save;
        Check(!model.HasChanges && client.Sent!["future_field"]!["nested"]!.GetValue<int>() == 7,
            "successful roundtrip preserves unknown fields");
        Check(client.Project == "project-a", "explicit project identity forwarded");
        model.Draft!["theme"] = "light"; model.Discard();
        Check(model.Draft["theme"]!.GetValue<string>() == "dark", "discard restores confirmed save");
        client.Pending = null;
        model.Draft["ui_font_size"] = 0; model.Draft["code_font_size"] = 30;
        var resized = await model.SaveAsync();
        Check(resized!["ui_font_size"]!.GetValue<int>() == 0 && resized["code_font_size"]!.GetValue<int>() == 30,
            "appearance save roundtrips both WebView font endpoints without replacing them with render floors");
        using var cancelled = new CancellationTokenSource(); cancelled.Cancel();
        try { await model.LoadAsync(cancelled.Token); throw new Exception("Expected cancellation"); } catch (OperationCanceledException) { }
        Check(!model.Busy, "cancellation releases busy state");
        var shared = new Fake { Stored = JsonNode.Parse("{\"theme\":\"light\",\"send_with_modifier\":true,\"selection_popup_enabled\":false,\"ui_font_size\":18,\"future_field\":{\"nested\":7}}")! };
        var changed = await AppearanceSettingsModel.ChangeThemeAsync(shared, "dark");
        var reopened = new AppearanceSettingsModel(shared, "another-project");
        await reopened.LoadAsync();
        Check(reopened.Draft!["theme"]!.GetValue<string>() == "dark", "footer theme persists when Settings reopens in another project");
        Check(NativeInputPreferences.From(changed) == new NativeInputPreferences(true, false)
            && changed["ui_font_size"]!.GetValue<int>() == 18 && changed["future_field"]!["nested"]!.GetValue<int>() == 7,
            "footer theme preserves input, fonts and unknown preferences");
        var writesBeforeInvalid = shared.Writes;
        try { await AppearanceSettingsModel.ChangeThemeAsync(shared, "invalid"); throw new Exception("Expected invalid theme"); }
        catch (ArgumentException) { }
        Check(shared.Writes == writesBeforeInvalid, "invalid theme does not write");
        shared.Fail = true;
        try { await AppearanceSettingsModel.ChangeThemeAsync(shared, "light"); throw new Exception("Expected lost acknowledgement"); }
        catch (IOException) { }
        Check(shared.Writes == writesBeforeInvalid + 1, "footer failed save is never replayed");
        var fontClient = new Fake { Stored = JsonNode.Parse("""{"ui_font_size":29,"code_font_size":1,"unknown":22}""")! };
        await AppearanceSettingsModel.StepFontAsync(fontClient, false, 1);
        await AppearanceSettingsModel.StepFontAsync(fontClient, false, 1);
        await AppearanceSettingsModel.StepFontAsync(fontClient, true, -1);
        var atLimit = await AppearanceSettingsModel.StepFontAsync(fontClient, true, -1);
        Check(fontClient.Writes == 2 && atLimit["code_font_size"]!.GetValue<int>() == 0 && atLimit["ui_font_size"]!.GetValue<int>() == 30
            && atLimit["unknown"]!.GetValue<int>() == 22, "font commands change only their own field and do not write beyond either endpoint");
        await reopened.LoadAsync();
        Check(reopened.Draft!["theme"]!.GetValue<string>() == "dark", "failed save is not presented as the saved appearance");
        var missingDatabase = Path.Combine(Path.GetTempPath(), Guid.NewGuid().ToString("N"), "wisp.sqlite");
        using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(10));
        try
        {
            using var unexpected = await NativeSettingsClient.ConnectAsync(missingDatabase, Environment.ProcessPath, deadline.Token);
            throw new Exception("Expected exited host failure");
        }
        catch (InvalidOperationException ex) when (ex.Message.Contains("设置宿主已退出"))
        { Check(true, "incompatible exiting host fails promptly instead of waiting indefinitely"); }
        var previousExit = Environment.GetEnvironmentVariable("WISP_TEST_HOST_EXIT_CODE");
        try
        {
            Environment.SetEnvironmentVariable("WISP_TEST_HOST_EXIT_CODE", "17");
            using var crashDeadline = new CancellationTokenSource(TimeSpan.FromSeconds(5));
            try
            {
                using var unexpected = await NativeSettingsClient.ConnectAsync(missingDatabase, Environment.ProcessPath, crashDeadline.Token);
                throw new Exception("Expected crashed host failure");
            }
            catch (InvalidOperationException ex) when (ex.Message.Contains("退出码 17"))
            { Check(true, "crashed host reports its exit code before the discovery deadline"); }
        }
        finally { Environment.SetEnvironmentVariable("WISP_TEST_HOST_EXIT_CODE", previousExit); }
    }

    private static void Check(bool passed, string message)
    {
        if (!passed) throw new Exception(message);
        Console.WriteLine("PASS appearance: " + message);
    }

    private static async Task CachePrecedenceAsync()
    {
        var oldProfile = new Wisp.Science.Preview.PreviewSettings { Typography = new(16, 15, CodeFamily: "Consolas") };
        var migrated = oldProfile.ReadAppearanceCache();
        Check((string?)migrated["ui_font_family"] == "" && (string?)migrated["code_font_family"] == "Consolas",
            "legacy rendering defaults stay placeholders, while a chosen font remains editable");
        oldProfile.CachedAppearance = new() { ["ui_font_family"] = "Inter", ["code_font_family"] = "" };
        migrated = oldProfile.ReadAppearanceCache();
        Check((string?)migrated["ui_font_family"] == "Inter" && (string?)migrated["code_font_family"] == "",
            "exact cached font choices override the lossy legacy rendering values");
        Check(Math.Abs(NativeSettingsLayout.AppearanceHeadingMargin(24 * 16d / 14) - 22.7657) < .01,
            "enlarging text retains proportional space above and below the Appearance heading");
        var host = new Fake { Stored = JsonNode.Parse("""{"saved":false,"theme":"system","dark_palette":"charcoal","ui_font_size":14,"future":3}""")! };
        JsonObject Local() => new() { ["theme"]="dark", ["dark_palette"]="github", ["ui_font_size"]=16, ["custom_css"]="/* local */", ["future"]=8 };
        var editor = new AppearanceSettingsModel(host, null, Local);
        await editor.LoadAsync();
        Check(editor.Draft?["theme"]?.ToString() == "dark" && editor.Draft["future"]?.ToString() == "3" && !editor.HasChanges && host.Writes == 0,
            "unsaved host defaults preserve local appearance without rewriting the database or its unknown fields");
        var live = new NativeLiveAppearanceModel(host, null, Local);
        await live.LoadAsync(); live.Set("selection_popup_enabled", false); await live.WaitForPersistenceAsync();
        Check(host.Writes == 1 && host.Sent?["dark_palette"]?.ToString() == "github" && host.Sent["custom_css"]?.ToString() == "/* local */"
            && host.Sent["saved"] == null && host.Sent["future"]?.ToString() == "3", "first settings edit keeps the other cached preferences");
        host.Stored = JsonNode.Parse("""{"saved":true,"theme":"light","ui_font_size":14}""")!;
        await editor.LoadAsync(); Check(editor.Draft?["theme"]?.ToString() == "light" && editor.Draft["custom_css"] == null,
            "a saved database wins over the cached theme and CSS");
        host.Stored = JsonNode.Parse("""{"theme":"system","ui_font_size":12}""")!;
        await editor.LoadAsync(); Check(editor.Draft?["theme"]?.ToString() == "system", "a legacy host without the marker retains its previous behavior");
        host.Stored = JsonNode.Parse("""{"saved":false,"ui_font_size":14}""")!;
        var stepped = await AppearanceSettingsModel.StepFontAsync(host, false, 1, local: Local);
        Check(stepped["ui_font_size"]?.GetValue<int>() == 17 && stepped["theme"]?.ToString() == "dark",
            "the font shortcut increments the displayed size, not the unsaved database default");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public bool Fail;
        public JsonNode Stored = JsonNode.Parse("{\"theme\":\"light\",\"future_field\":{\"nested\":7}}")!;
        public int Writes;
        public string? Project;
        public JsonNode? Sent;
        public TaskCompletionSource<JsonNode?>? Pending;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            cancellationToken.ThrowIfCancellationRequested(); Project = projectId;
            if (command == "get_appearance_prefs") return Task.FromResult<JsonNode?>(Stored.DeepClone());
            if (command != "set_appearance_prefs") throw new Exception("Unexpected command");
            Writes++; Sent = arguments["prefs"]!.DeepClone();
            if (Fail) return Task.FromException<JsonNode?>(new IOException("lost response"));
            if (Pending != null) return Pending.Task;
            Stored = Sent.DeepClone();
            return Task.FromResult<JsonNode?>(Stored.DeepClone());
        }
    }
}
