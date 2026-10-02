using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;

internal static class NativeSettingsSearchTests
{
    public static void Run()
    {
        var entry = JsonNode.Parse("""{"zh":"网络","en":"Network","group":"基础偏好","group_en":"Preferences","aliases":"proxy mirror 代理 镜像"}""")!.AsObject();
        var original = entry.ToJsonString();
        foreach (var query in new[] { "", "  ", "NETWORK", "代理", "Preferences proxy", "镜像\t网络", "network" })
            if (!NativeSettingsSearch.Matches("network", entry, query)) throw new InvalidOperationException("Missing settings search: " + query);
        foreach (var query in new[] { "模型", "proxy unknown", "model" })
            if (NativeSettingsSearch.Matches("network", entry, query)) throw new InvalidOperationException("Unexpected settings search: " + query);
        if (entry.ToJsonString() != original) throw new InvalidOperationException("Search changed navigation data");
        if (!NativeSettingsSearch.Matches("future", new JsonObject(), "future")) throw new InvalidOperationException("Missing optional labels must retain ID search");
        Console.WriteLine("Native settings search labels, aliases, multi-term matching and immutable navigation passed.");
    }
}
