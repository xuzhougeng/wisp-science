using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;

internal static class NativeSettingsSearchTests
{
    public static void Run()
    {
        var entry = JsonNode.Parse("""{"zh":"网络","en":"Network","group":"基础偏好","group_en":"Preferences","aliases":"proxy mirror 代理 镜像"}""")!.AsObject();
        var original = entry.ToJsonString();
        foreach (var query in new[] { "", "  ", "NETWORK", "代理", "Preferences proxy", "镜像\t网络", "network" })
            if (!NativeSettingsSearch.Matches(entry, query, "en")) throw new InvalidOperationException("Missing settings search: " + query);
        foreach (var query in new[] { "模型", "proxy unknown", "model", "基础偏好" })
            if (NativeSettingsSearch.Matches(entry, query, "en")) throw new InvalidOperationException("Unexpected settings search: " + query);
        if (!NativeSettingsSearch.Matches(entry, "基础偏好 proxy", "zh")
            || NativeSettingsSearch.Matches(entry, "Preferences", "zh")
            || !NativeSettingsSearch.Matches(entry, "NETWORK 镜像", "zh"))
            throw new InvalidOperationException("Search should use the displayed group and both languages' labels and aliases");
        var remoteAccess = JsonNode.Parse("""{"en":"Remote Access","zh":"远程接入","aliases":"sync feishu weixin device 同步 飞书 微信 设备"}""")!.AsObject();
        if (NativeSettingsSearch.Matches(remoteAccess, "channels", "en") || !NativeSettingsSearch.Matches(remoteAccess, "remote 飞书", "zh"))
            throw new InvalidOperationException("Search should use the section's visible names and aliases, not its routing ID");
        if (entry.ToJsonString() != original) throw new InvalidOperationException("Search changed navigation data");
        Console.WriteLine("Native settings search labels, aliases, multi-term matching and immutable navigation passed.");
    }
}
