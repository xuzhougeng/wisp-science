using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;

internal static class NativeAcpChecksTests
{
    public static void Run()
    {
        var memo = new NativeAcpChecks();
        var first = new JsonObject { ["id"] = "one", ["command"] = "agent" };
        var second = new JsonObject { ["id"] = "two", ["command"] = "other" };
        var info = JsonNode.Parse("""{"protocolVersion":1,"implementation":{"name":"fallback","title":"Study","version":"1.2"},"authMethods":[{"id":"device","name":"Device login","description":"Use your browser","type":"agent"}]}""");
        memo.Store(first, info);
        memo.Store(second, JsonNode.Parse("""{"protocolVersion":2,"authMethods":[]}"""));
        if (memo.For(first) is not { Version:"Study 1.2 · ACP v1", Methods:[{Id:"device", Name:"Device login"}] }
            || memo.For(second)?.Version != "ACP v2") throw new InvalidOperationException("Agent rows must retain their own version and methods.");
        first["command"] = "replacement";
        memo.Retain([first,second,new JsonObject()]);
        if (memo.For(first) != null || memo.For(second) == null) throw new InvalidOperationException("Changing one command invalidates only that row.");
        memo.Retain([]); if (memo.For(second) != null) throw new InvalidOperationException("A removed profile must lose its connection result.");
    }
}
