using Wisp.ProjectBrowser;

internal static class NativeMessagePresentationTests
{
    public static void Run()
    {
        static void Check(bool value, string reason) { if (!value) throw new InvalidOperationException(reason); }
        Check(NativeMessagePresentation.ImageWidth(554) == 148 && NativeMessagePresentation.ImageWidth(800) == 176
            && NativeMessagePresentation.ImageWidth(1200) == 220 && NativeMessagePresentation.ImageWidth(double.NaN) == 148,
            "image cards follow WebView's 22vw width with the 148–220 DIP bounds");
        var text = "Keep **literal Markdown** and `code`.\n\nUploaded files: uploads/a.csv, uploads/b.png\n\nSelected skills: review\n\nTarget runtimes: Python\n\nAI source-edit instruction: hidden\n\nFeedback context: hidden diagnostics\n\nA final user paragraph.";
        var parsed = NativeMessagePresentation.User(text, ["uploads/a.csv"]);
        Check(parsed.Body == "Keep **literal Markdown** and `code`.\n\nA final user paragraph." && !parsed.IsLong,
            "presentation preserves user text verbatim and removes only recognized whole metadata blocks");
        Check(parsed.Attachments.SequenceEqual(new[] { "uploads/a.csv", "uploads/b.png" })
            && parsed.References.SequenceEqual(new[] { new NativeMessageReference("skill", "review"), new NativeMessageReference("runtime", "Python") }),
            "attachments deduplicate while context cards retain their kinds and order");
        Check(NativeMessagePresentation.User("Explain Uploaded files: text").Body == "Explain Uploaded files: text", "inline words do not become hidden metadata");
        var unicode = NativeMessagePresentation.User(string.Concat(Enumerable.Repeat("🧬", 601)));
        Check(unicode.IsLong && unicode.ShortBody.EnumerateRunes().Count() == 601 && unicode.ShortBody.EndsWith("🧬…"), "long previews count Unicode scalars without splitting surrogate pairs");
        var lines = NativeMessagePresentation.User(string.Join("\n", Enumerable.Range(1, 13)));
        Check(lines.IsLong && lines.ShortBody.EndsWith("12…") && lines.Body.EndsWith("13"), "line previews retain all original text for expansion");
        Check(!NativeMessagePresentation.User(new string('a', 600)).IsLong, "600 characters is still shown in full");
        var now = new DateTimeOffset(2026, 11, 3, 12, 0, 0, TimeSpan.Zero);
        var seconds = now.AddHours(-1).ToUnixTimeSeconds(); var milliseconds = seconds * 1000;
        Check(NativeMessagePresentation.Time(seconds, "en", now, TimeZoneInfo.Utc)?.Compact == "11:00"
            && NativeMessagePresentation.Time(milliseconds, "en", now, TimeZoneInfo.Utc)?.Compact == "11:00", "historical seconds and milliseconds show the same local clock");
        Check(NativeMessagePresentation.Time(now.AddDays(-1).ToUnixTimeSeconds(), "zh", now, TimeZoneInfo.Utc)?.Compact == "11-02 12:00"
            && NativeMessagePresentation.Time(now.AddYears(-1).ToUnixTimeSeconds(), "en", now, TimeZoneInfo.Utc)?.Compact == "2025-11-03 12:00", "older messages distinguish day and year");
        Check(NativeMessagePresentation.Time(0, "en", now) == null && NativeMessagePresentation.Time(long.MaxValue, "en", now) == null, "invalid timestamps do not invent dates or break rendering");
    }
}
