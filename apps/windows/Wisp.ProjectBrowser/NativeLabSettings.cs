using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// Maps the existing Lab contract without inventing membership on a failed read.
public static class NativeLabSettings
{
    private static string Text(JsonNode? value, string key) => value?[key]?.GetValue<string>() ?? "";
    public static JsonObject JoinDraft() => new()
    { ["invite"] = "", ["relay_url"] = "", ["relay_token"] = "", ["name"] = "", ["lab_name"] = "" };
    public static string? JoinBlockReason(JsonObject draft) => string.IsNullOrWhiteSpace(Text(draft, "name")) ? "请填写你的名字。"
        : Text(draft, "name").Trim().Length > 64 || Text(draft, "lab_name").Trim().Length > 64 ? "名字和课题组名称最多 64 个字符。" : null;
    public static JsonObject JoinArguments(JsonObject draft)
    {
        if (JoinBlockReason(draft) is { } error) throw new InvalidDataException(error);
        return new() { ["invite"] = Text(draft, "invite").Trim(), ["relayUrl"] = Text(draft, "relay_url").Trim(),
            ["relayToken"] = Text(draft, "relay_token"), ["name"] = Text(draft, "name").Trim(), ["labName"] = Text(draft, "lab_name").Trim() };
    }
    public static string StateLabel(JsonNode? status, NativeLocalization? locale = null) => Text(status, "state") switch
    {
        "active" => status?["leader"]?.GetValue<bool>() == true ? locale?.Text("channels.lab.role_leader") ?? "组长" : locale?.Text("channels.lab.role_member") ?? "成员",
        "pending" => locale?.Text("channels.lab.pending") ?? "待审核",
        "removed" or "error" => locale?.Text("channels.state.error") ?? "出错",
        "none" => locale?.Text("channels.lab.not_joined") ?? "未加入", _ => locale?.Choose("状态未读取", "Status unavailable") ?? "状态未读取"
    };
    public static bool CanManageMember(JsonNode status, JsonNode member) => Text(status, "state") == "active"
        && status["leader"]?.GetValue<bool>() == true && Text(member, "id").Length > 0 && Text(member, "id") != Text(status, "member_id");
    public static string Explain(string error, NativeLocalization? locale = null) => string.Join("\n", error.Split('\n').Select(line =>
    {
        var raw = line.TrimEnd('\r');
        var known = ExplainLine(raw);
        // Only known protocol codes enter the catalog; server/user text stays literal.
        return locale != null && known != raw ? locale.Text("channels.lab.error." + raw[4..]) : known;
    }));
    private static string ExplainLine(string error) => error switch
    {
        "lab:lab_disabled" => "这台中继没有开启课题组功能，需由管理员以 WISP_LAB=1 启动。", "lab:unauthorized" => "中继访问令牌不正确。",
        "lab:invite_required" => "这个课题组已有组长，请向组长索取校验码。", "lab:invalid_invite" => "校验码无效，可能已被使用或已过期。",
        "lab:invalid_name" => "请输入不超过 64 个字符的名称。", "lab:leader_cannot_leave" => "组长不能退出课题组。",
        "lab:lab_full" => "课题组成员数已达上限。", "lab:too_many_invites" => "未使用的校验码过多，请等它们被使用或过期。",
        "lab:member_not_found" => "该成员已不在课题组中。", "lab:storage_error" => "中继保存失败，请检查其数据目录。",
        "lab:not_active" => "尚未通过组长审核。", "lab:not_leader" => "只有组长可以执行此操作。",
        "lab:unknown_member" => "这台电脑已不在课题组中。", "lab:no_knowledge_base" => "组长还没有发布知识库。",
        "lab:knowledge_base_other_relay" => "请先通过课题组使用的中继同步该项目。", "lab:knowledge_base_unreadable" => "无法解密知识库，请检查是否加入了正确的课题组。",
        _ => error
    };
}
