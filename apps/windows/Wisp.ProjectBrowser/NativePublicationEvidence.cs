using System.Globalization;
using System.Text;
using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public static class NativePublicationEvidence
{
    public static readonly (string Id, string Label)[] SelectionChoices = [("candidate", "候选"), ("selected", "采用"), ("rejected", "排除")];
    public static string SourceLabel(NativePublicationSource source)
    {
        if (source.Kind != "message_span") return source.Title + " · " + source.Detail;
        var text = source.Text?.Replace('\r', ' ').Replace('\n', ' ') ?? "";
        var excerpt = string.Concat(text.EnumerateRunes().Take(80).Select(r => r.ToString()));
        return $"{source.Title} · 消息 {source.MessageSeq} · {source.Detail}\n{excerpt}";
    }

    // Match the WebView precise-source form. The host resolves and checks the
    // exact owner/content before registering any binding.
    public static NativePublicationSource Precise(string kind, string frame, string sequence, string start, string end, string call, string exact)
    {
        string id;
        if (kind is "execution_log" or "code_cell" or "external_resource")
        {
            id = exact.Trim();
            if (id.Length == 0) throw new InvalidDataException("请填写精确来源 ID。");
        }
        else if (kind is "message_span" or "tool_call")
        {
            if (string.IsNullOrWhiteSpace(frame) || !long.TryParse(sequence, NumberStyles.None, CultureInfo.InvariantCulture, out var seq) || seq < 1)
                throw new InvalidDataException("请填写会话 ID 和正整数消息序号。");
            var fields = new SortedDictionary<string, object?> { ["frame_id"] = frame.Trim(), ["message_seq"] = seq };
            if (kind == "message_span")
            {
                if (!long.TryParse(start, NumberStyles.None, CultureInfo.InvariantCulture, out var first)
                    || !long.TryParse(end, NumberStyles.None, CultureInfo.InvariantCulture, out var last) || first < 0 || last <= first)
                    throw new InvalidDataException("请填写有效的 UTF-8 字节范围，结束位置必须大于起始位置。");
                fields["byte_start"] = first; fields["byte_end"] = last;
            }
            else
            {
                if (string.IsNullOrWhiteSpace(call)) throw new InvalidDataException("请填写工具调用 ID。");
                fields["tool_call_id"] = call.Trim();
            }
            id = JsonSerializer.Serialize(fields);
        }
        else throw new InvalidDataException("不支持的精确来源类型。");
        var title = kind switch { "message_span" => $"消息 {sequence} · 字节 {start}–{end}", "tool_call" => "工具调用 · " + call.Trim(),
            "execution_log" => "执行日志", "code_cell" => "代码单元", _ => "外部资源" };
        return new(kind, id, title, "精确来源", null, null, null, null);
    }
}
