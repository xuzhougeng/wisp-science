using System.Text.Json;
using System.Text.RegularExpressions;

namespace Wisp.ProjectBrowser;

/// <summary>Selection position only. Project, session and path belong to the mounted native panel.</summary>
public sealed record NativeDocumentLocation(string Kind, int? Page = null, int? EndPage = null,
    string? Sheet = null, string? Cells = null, int? Line = null, int? EndLine = null, bool Unsaved = false)
{
    private static bool Position(int? value) => value is > 0 and <= 1000000;
    public bool Valid => Kind switch {
        "pdf" or "docx" or "pptx" => Position(Page) && (EndPage == null || Position(EndPage) && EndPage >= Page)
            && Sheet == null && Cells == null && Line == null && EndLine == null && !Unsaved,
        "xlsx" => !string.IsNullOrWhiteSpace(Sheet) && Sheet.Length <= 256 && !Sheet.Any(char.IsControl)
            && Cells != null && Regex.IsMatch(Cells, @"^[A-Z]{1,3}[1-9][0-9]{0,6}(:[A-Z]{1,3}[1-9][0-9]{0,6})?$")
            && Page == null && EndPage == null && Line == null && EndLine == null && !Unsaved,
        "source" => Position(Line) && Position(EndLine) && EndLine >= Line && Page == null && EndPage == null && Sheet == null && Cells == null,
        _ => false,
    };
    public string Label => Kind switch {
        "xlsx" => $"工作表 {Sheet} · {Cells}",
        "source" => $"源文本第 {Line}{(EndLine == Line ? "" : "–" + EndLine)} 行" + (Unsaved ? "（未保存草稿）" : ""),
        _ => $"{(Kind == "docx" ? "预览" : "")}第 {Page}{(EndPage == null || EndPage == Page ? "" : "–" + EndPage)} {(Kind == "pptx" ? "张幻灯片" : "页")}",
    };
}

public sealed record NativeDocumentSelection(string Text, NativeDocumentLocation Location, bool Jump)
{
    public NativeDocumentSelection(string text, int page, bool jump) : this(text, new NativeDocumentLocation("pdf", Page: page), jump) { }
    public int Page => Location.Page ?? 0;
    public bool Valid => !string.IsNullOrWhiteSpace(Text) && Text.Length <= 32768 && Location.Valid;
    public static NativeDocumentSelection? Parse(JsonElement value, string expectedKind = "pdf")
    {
        try {
            if (value.ValueKind != JsonValueKind.Object
                || !value.TryGetProperty("text", out var text) || text.ValueKind != JsonValueKind.String
                || !value.TryGetProperty("jump", out var jump) || jump.ValueKind is not (JsonValueKind.True or JsonValueKind.False)) return null;
            NativeDocumentLocation? location;
            if (value.TryGetProperty("location", out var node)) {
                location = node.Deserialize<NativeDocumentLocation>(new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.CamelCase });
            } else if (expectedKind == "pdf" && value.TryGetProperty("page", out var page) && page.TryGetInt32(out var number)) {
                location = new("pdf", Page: number);
            } else return null;
            if (location == null || location.Kind != expectedKind) return null;
            var selection = new NativeDocumentSelection(text.GetString()!, location, jump.GetBoolean());
            return selection.Valid ? selection : null;
        } catch (Exception e) when (e is JsonException or InvalidOperationException or FormatException) { return null; }
    }
    public static NativeDocumentSelection? FromSource(string text, int start, int length, bool jump, bool unsaved = false)
    {
        if (start < 0 || length <= 0 || start > text.Length - length || length > 32768) return null;
        var end = start + length;
        bool Boundary(int i) => i == 0 || i == text.Length || !(char.IsHighSurrogate(text[i - 1]) && char.IsLowSurrogate(text[i]));
        if (!Boundary(start) || !Boundary(end)) return null;
        int LineAt(int offset) {
            var line = 1;
            for (var i = 0; i < offset; i++) {
                if (text[i] == '\r') { line++; if (i + 1 < offset && text[i + 1] == '\n') i++; }
                else if (text[i] == '\n') line++;
            }
            return line;
        }
        var selection = new NativeDocumentSelection(text.Substring(start, length),
            new NativeDocumentLocation("source", Line: LineAt(start), EndLine: LineAt(end - 1), Unsaved: unsaved), jump);
        return selection.Valid ? selection : null;
    }
}
