using System.Text.Json;
using System.Text;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeDocumentSource(string ProjectId, string Path, NativeDocumentLocation Location);

public sealed record NativeComposerQuote(string SessionId, int UserIndex, string Role, string Text, NativeDocumentSource? Document = null, string? ExcerptLabel = null)
{
    // WebView cards show the source path (or excerpt caption), not prompt provenance.
    public string SourceLabel(NativeLocalization locale) => Role == "excerpt" ? ExcerptLabel ?? ""
        : Document?.Path ?? locale.Text("attachment.quote");
    public string Tooltip() => (Role == "excerpt" ? ExcerptLabel : Document?.Path) is { } source
        ? source + "\n\n" + Text : Text;
    public string Source => Role == "excerpt" ? ExcerptLabel ?? "" : Role == "runtime" ? "Runtime" : Document is { } source
        ? $"文件 {source.Path} · {source.Location.Label} · 项目 {source.ProjectId}"
        : $"会话 {SessionId} · 第 {UserIndex + 1} 轮 · {Role}";
    public string Message => (Role == "runtime" ? "" : Role == "excerpt"
        ? $"Quoted turns from an earlier conversation `{Source.Replace("`", "\\`").Replace('\r', ' ').Replace('\n', ' ').Trim()}`:\n"
        : $"引用来源：{Source}\n") + string.Join("\n", Text.Replace("\r\n", "\n").Replace('\r', '\n').Split('\n').Select(line => "> " + line));
    public static NativeComposerQuote FromExcerpt(NativeSessionExcerpt excerpt, NativeLocalization locale)
    {
        var title = string.IsNullOrWhiteSpace(excerpt.Title) ? locale.Text("excerpt.untitled") : excerpt.Title.Trim();
        var label = locale.Format(excerpt.StartUserIndex == excerpt.EndUserIndex ? "excerpt.source_single" : "excerpt.source",
            ("title", title), ("n", (excerpt.StartUserIndex + 1).ToString()),
            ("start", (excerpt.StartUserIndex + 1).ToString()), ("end", (excerpt.EndUserIndex + 1).ToString()));
        return new(excerpt.SessionId, excerpt.StartUserIndex, "excerpt", excerpt.Text.Trim(), ExcerptLabel: label);
    }
    public string Label
    {
        get
        {
            var line = Text.Trim().Replace("\r\n", "\n").Replace('\r', '\n').Split('\n')[0].EnumerateRunes().ToArray();
            return string.Concat(line.Take(40).Select(rune => rune.ToString())) + (line.Length > 40 ? "…" : "");
        }
    }
    public static NativeComposerQuote? From(ConversationSnapshot snapshot, int index, string text)
    {
        if (index < 0 || index >= snapshot.Items.Length || string.IsNullOrWhiteSpace(text)) return null;
        var user = (snapshot.UserOffset ?? 0) - 1;
        for (var n = 0; n <= index; n++) if (snapshot.Items[n].Role == "user") user++;
        return user < 0 ? null : new(snapshot.SessionId, user, snapshot.Items[index].Role, text.Trim());
    }
    public static string Compose(string draft, NativeComposerQuote[] quotes) =>
        string.Join("\n\n", quotes.Select(quote => quote.Message).Append(draft.Trim()).Where(value => value.Length > 0));
}

/// <summary>Uses persisted context totals, not cumulative billing tokens. Compaction
/// invalidates the old bucket split until a fresh usage row arrives.</summary>
public sealed record NativeContextUsageRow(string Key, string Label, ulong Tokens)
{
    public bool Expandable => Key != "conversation";
    // Shared context-usage colors in ui/src/styles/chat.css (same in both themes).
    public string Color => Key switch { "system_prompt" => "#7c7e7d", "tool_definitions" => "#7561d3", "rules" => "#087e4c",
        "skills" => "#b06d00", "mcp_dynamic_tools" => "#97145f", "subagent_definitions" => "#2e7fc5", _ => "#cf3715" };
}

public sealed record NativeContextUsage(ulong Used, ulong Max, IReadOnlyList<NativeContextUsageRow> Rows, bool HasBreakdown = true)
{
    public int Percent => Max == 0 ? 0 : (int)UInt128.Min(100, ((UInt128)Used * 100 + Max / 2) / Max);
    public string Label => Max == 0 ? "上下文 · 窗口未知" : $"上下文 · {Percent}%";
    public string CompactLabel => Max == 0 ? "—" : $"{Percent}%";
    public string Total => Max == 0 ? $"约 {Used:N0} Tokens · 窗口未知" : $"约 {Used:N0} / {Max:N0} Tokens";
    public string Tone => Max == 0 ? "text-muted" : Percent > 90 ? "err" : Percent >= 70 ? "warn" : "text-muted";
    public string LocalizedLabel(NativeLocalization locale) => Max == 0 ? locale.Text("context_usage.open_unknown")
        : locale.Format("context_usage.open_pct", ("pct", Percent.ToString()));
    public string LocalizedTotal(NativeLocalization locale) => locale.Format(Max == 0 ? "context_usage.total_used" : "context_usage.total_estimated",
        ("used", FormatTokens(Used)), ("max", FormatLimit(Max)));
    public static string FormatTokens(ulong tokens) => tokens < 1_000 ? tokens.ToString(System.Globalization.CultureInfo.InvariantCulture)
        : (tokens / (tokens < 1_000_000 ? 1_000d : 1_000_000d)).ToString("0.0", System.Globalization.CultureInfo.InvariantCulture)
            + (tokens < 1_000_000 ? "K" : "M");
    public static string FormatLimit(ulong tokens) => tokens >= 1_000_000 && tokens % 1_000_000 == 0 ? $"{tokens / 1_000_000}M"
        : tokens >= 1_000 && tokens % 1_000 == 0 ? $"{tokens / 1_000}K" : FormatTokens(tokens);
    public double SegmentShare(NativeContextUsageRow row) => (double)row.Tokens / Math.Max(1UL, Math.Max(Max, Used));
    public static NativeContextUsage? Latest(IEnumerable<ConversationItem> items, NativeLocalization? locale = null)
    {
        ulong? compacted = null;
        foreach (var item in items.Reverse())
        {
            if (item.Role is not ("usage" or "compaction")) continue;
            try
            {
                using var document = JsonDocument.Parse(item.Text);
                var root = document.RootElement;
                if (root.ValueKind != JsonValueKind.Object) continue;
                if (item.Role == "compaction")
                {
                    if (compacted == null && root.TryGetProperty("undone", out var undone) && undone.ValueKind == JsonValueKind.True) continue;
                    if (compacted == null && (!root.TryGetProperty("strategy", out var strategy) || strategy.GetString() != "auto_continue")
                        && root.TryGetProperty("after", out var after) && after.TryGetUInt64(out var value)) compacted = value;
                    continue;
                }
                static ulong Number(JsonElement node, string key) => node.ValueKind == JsonValueKind.Object
                    && node.TryGetProperty(key, out var value) && value.TryGetUInt64(out var number) ? number : 0;
                var used = Number(root, "ctx_tokens"); var max = Number(root, "max_context");
                if (used == 0 && max == 0) continue;
                var rows = new List<NativeContextUsageRow>();
                root.TryGetProperty("context_usage", out var buckets);
                foreach (var (key, label) in new[] { ("system_prompt", "系统提示词"), ("tool_definitions", "工具定义"),
                        ("rules", "规则"), ("skills", "技能"), ("mcp_dynamic_tools", "MCP 与动态工具"),
                        ("subagent_definitions", "子智能体定义"), ("conversation", "对话") })
                    rows.Add(new(key, locale?.Text("context_usage." + key) ?? label, compacted == null ? Number(buckets, key) : 0));
                var synthetic = rows.All(row => row.Tokens == 0);
                if (synthetic) rows[^1] = rows[^1] with { Tokens = compacted ?? used };
                return new(compacted ?? used, max, rows, compacted == null && !synthetic && buckets.ValueKind == JsonValueKind.Object);
            }
            catch (Exception ex) when (ex is JsonException or InvalidOperationException or FormatException) { }
        }
        return null;
    }
}
