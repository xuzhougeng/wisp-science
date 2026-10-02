using System.Globalization;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record TranscriptSection(string Text, string? ToolName = null, bool IsResult = false);
public sealed record TranscriptGroup(bool IsProcess, IReadOnlyList<BrowserMessage> Messages);

public static partial class TranscriptPresentation
{
    // v1 appends each saved assistant tool call as a name line and a JSON arguments line.
    // Only recognize that exact shape; leave ordinary prose and malformed data intact.
    public static IReadOnlyList<TranscriptSection> Sections(BrowserMessage message)
    {
        if (message.Role == "tool" && message.ToolName == "attempt_completion") return [new(message.Text)];
        if (message.Role == "tool") return [new(ReadableArguments(message.Text), message.ToolName ?? "工具", true)];
        if (message.Role != "assistant") return [new(message.Text)];
        var sections = new List<TranscriptSection>();
        var text = new StringBuilder();
        var lines = message.Text.Replace("\r\n", "\n").Split('\n');
        for (var i = 0; i < lines.Length; i++)
        {
            if (i + 1 < lines.Length && ToolNamePattern().IsMatch(lines[i]) && IsJsonObject(lines[i + 1]))
            {
                if (text.ToString().Trim().Length > 0) sections.Add(new(text.ToString()));
                text.Clear();
                var completion = lines[i] == "attempt_completion" ? CompletionResult(lines[i + 1]) : null;
                sections.Add(completion != null ? new(completion) : new(ReadableArguments(lines[i + 1]), lines[i]));
                i++;
            }
            else text.AppendLine(lines[i]);
        }
        if (text.ToString().Trim().Length > 0) sections.Add(new(text.ToString()));
        return sections;
    }

    private static string? CompletionResult(string json)
    {
        using var document = JsonDocument.Parse(json);
        return document.RootElement.TryGetProperty("result", out var result) && result.ValueKind == JsonValueKind.String
            ? result.GetString() : null;
    }

    public static IReadOnlyList<BrowserMessage> ReadableMessages(IReadOnlyList<BrowserMessage> messages)
    {
        var result = new List<BrowserMessage>();
        foreach (var message in messages)
        {
            // Only suppress a successful-looking exact echo adjacent to its call.
            // Different text (including errors) remains visible.
            if (message is { Role: "tool", ToolName: "attempt_completion" } && result.LastOrDefault() is { Role: "assistant" } previous
                && HasCompletionResult(previous.Text, message.Text)) continue;
            result.Add(message);
        }
        return result;
    }

    private static bool HasCompletionResult(string text, string expected)
    {
        var lines = text.Replace("\r\n", "\n").Split('\n');
        for (var i = 0; i + 1 < lines.Length; i++)
            if (lines[i] == "attempt_completion" && IsJsonObject(lines[i + 1])
                && CompletionResult(lines[i + 1])?.Trim() == expected.Trim()) return true;
        return false;
    }

    public static IReadOnlyList<TranscriptGroup> Groups(IReadOnlyList<BrowserMessage> messages)
    {
        var groups = new List<TranscriptGroup>();
        var process = new List<BrowserMessage>();
        foreach (var message in ReadableMessages(messages))
        {
            var sections = Sections(message);
            // This persisted schema has no success flag. Keep ambiguous/error
            // results visible rather than claiming that their work succeeded.
            var activity = message.Role == "assistant" && sections.Count > 0 && sections.All(s => s.ToolName != null);
            if (activity) process.Add(message);
            else
            {
                Flush(); groups.Add(new(false, [message]));
            }
        }
        Flush(); return groups;
        void Flush() { if (process.Count > 0) { groups.Add(new(true, process.ToArray())); process.Clear(); } }
    }

    private static bool IsJsonObject(string value)
    {
        try { using var json = JsonDocument.Parse(value); return json.RootElement.ValueKind == JsonValueKind.Object; }
        catch (JsonException) { return false; }
    }

    public static string ReadableArguments(string value)
    {
        try
        {
            using var json = JsonDocument.Parse(value);
            if (json.RootElement.ValueKind != JsonValueKind.Object) return value;
            return string.Join("\n\n", json.RootElement.EnumerateObject().Select(p =>
                p.Name + ":\n" + (p.Value.ValueKind == JsonValueKind.String ? p.Value.GetString() : p.Value.GetRawText())));
        }
        catch (JsonException) { return value; }
    }

    /// <summary>Compact per-turn usage row matching the WebView's usage-line
    /// wording and token formatting. Returns null for non-usage or malformed
    /// rows so callers fall back to plain text instead of losing data.</summary>
    public static string? UsageSummary(string role, string text)
    {
        if (role != "usage") return null;
        try
        {
            using var json = JsonDocument.Parse(text);
            var root = json.RootElement;
            if (root.ValueKind != JsonValueKind.Object) return null;
            long Input(string name) => root.TryGetProperty(name, out var v) && v.TryGetInt64(out var n) ? n : 0;
            var summary = $"输入 {FmtTokens(Input("input"))} · 输出 {FmtTokens(Input("output"))} tokens";
            if (Input("cached") > 0) summary += $" · 缓存 {FmtTokens(Input("cached"))}";
            if (Input("reasoning") > 0) summary += $" · 思考 {FmtTokens(Input("reasoning"))}";
            return summary;
        }
        catch (JsonException) { return null; }
    }

    /// <summary>Mirrors the WebView fmt_tokens: values under 1000 stay plain,
    /// everything else renders as one-decimal k.</summary>
    public static string FmtTokens(long n) =>
        n < 1000 ? n.ToString(CultureInfo.InvariantCulture) : (n / 1000.0).ToString("0.0", CultureInfo.InvariantCulture) + "k";

    /// <summary>view_image tool results carry "Image: <path>" on the first line.
    /// Returns the path only, without existing-file checks.</summary>
    public static string? ToolImagePath(string text)
    {
        var line = text.Replace("\r\n", "\n").Split('\n').FirstOrDefault()?.Trim() ?? "";
        if (!line.StartsWith("Image: ", StringComparison.Ordinal)) return null;
        var path = line["Image: ".Length..].Trim();
        // Trim the "(resized for model)" suffix the host appends.
        var cut = path.LastIndexOf(" (", StringComparison.Ordinal);
        if (cut > 0) path = path[..cut].Trim();
        return path.Length > 0 ? path : null;
    }

    [GeneratedRegex(@"^[A-Za-z_][A-Za-z0-9_.:-]*$", RegexOptions.CultureInvariant)]
    private static partial Regex ToolNamePattern();
}
