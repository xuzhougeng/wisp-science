using System.Globalization;
using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;

namespace Wisp.ProjectBrowser;

/// <summary>Display-only source preparation. Copy/export keep the original source.</summary>
public sealed record NativeCodePreview(string Text, string? Language, string Gutter, int LineCount)
{
    private static readonly IComparer<string> JsonKeyOrder = Comparer<string>.Create((left, right) =>
    {
        var a = left.EnumerateRunes(); var b = right.EnumerateRunes();
        while (a.MoveNext())
        {
            if (!b.MoveNext()) return 1;
            var order = a.Current.Value.CompareTo(b.Current.Value); if (order != 0) return order;
        }
        return b.MoveNext() ? -1 : 0;
    });
    public static NativeCodePreview Create(string source, string? language)
    {
        // A CRLF split across colored runs must not become two XAML line breaks.
        // Retain leading/trailing blank lines, indentation and Unicode verbatim.
        var text = source.Replace("\r\n", "\n").Replace('\r', '\n');
        var count = text.Count(c => c == '\n') + 1;
        return new(text, NativeCodeHighlight.NormalizeLanguage(language),
            string.Join("\n", Enumerable.Range(1, count).Select(n => n.ToString(CultureInfo.InvariantCulture))), count);
    }

    public static string? FileLanguage(string path, string name, string kind)
        => kind == "json" ? "json" : ExtensionLanguage(path) ?? ExtensionLanguage(name);

    // Match text.rs::code_lang, including opaque artifact paths whose display
    // filename is the only language hint. Unsupported native grammars stay plain.
    private static string? ExtensionLanguage(string path) => !path.Contains('.') ? null : path.Split('.').Last().ToLowerInvariant() switch
    {
        "r" => "r", "py" or "pyw" => "python", "sh" or "bash" or "zsh" => "bash",
        "js" or "mjs" or "cjs" => "javascript", "ts" or "tsx" or "mts" or "cts" => "typescript",
        "rs" => "rust", "go" => "go", "java" => "java", "kt" or "kts" => "kotlin",
        "swift" => "swift", "rb" => "ruby", "pl" or "pm" => "perl", "lua" => "lua", "php" => "php",
        "c" or "h" => "c", "cpp" or "cc" or "cxx" or "hpp" or "hh" => "cpp", "cs" => "csharp",
        "m" => "objectivec", "sql" => "sql", "css" => "css", "scss" => "scss", "less" => "less",
        "xml" or "xsl" or "rss" => "xml", "yaml" or "yml" => "yaml",
        "toml" or "ini" or "cfg" or "conf" => "ini", "diff" or "patch" => "diff",
        "json" or "jsonl" or "ipynb" => "json", _ => null
    };

    public static string FormatFile(string source, string? language)
    {
        if (language != "json") return source;
        try
        {
            using var json = JsonDocument.Parse(source);
            using var output = new MemoryStream();
            using (var writer = new Utf8JsonWriter(output, new JsonWriterOptions
                { Indented = true, Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping }))
                WriteJson(writer, json.RootElement);
            return Encoding.UTF8.GetString(output.ToArray());
        }
        catch (JsonException) { return source; }
    }
    private static void WriteJson(Utf8JsonWriter writer, JsonElement value)
    {
        if (value.ValueKind == JsonValueKind.Object)
        {
            writer.WriteStartObject();
            // serde_json's map sorts keys and retains the last duplicate value.
            foreach (var property in value.EnumerateObject().GroupBy(p => p.Name, StringComparer.Ordinal)
                .Select(group => group.Last()).OrderBy(p => p.Name, JsonKeyOrder))
            { writer.WritePropertyName(property.Name); WriteJson(writer, property.Value); }
            writer.WriteEndObject();
        }
        else if (value.ValueKind == JsonValueKind.Array)
        { writer.WriteStartArray(); foreach (var item in value.EnumerateArray()) WriteJson(writer, item); writer.WriteEndArray(); }
        else value.WriteTo(writer);
    }
}
