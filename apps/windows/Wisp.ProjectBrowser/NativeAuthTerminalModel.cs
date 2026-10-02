using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Settings can only access the host-created, project-owned ACP authentication PTY.
public sealed class NativeAuthTerminalModel(INativeSettingsClient client, string projectId, string sessionId) : WorkspaceActionModel
{
    private readonly CancellationTokenSource lifetime = new();
    private bool reading;
    public string Output { get; private set; } = "";
    public bool Running { get; private set; }
    public bool InputUncertain { get; private set; }
    public bool CanWrite => !Closed && !Busy && Running && !InputUncertain;
    public async Task<bool> ReadAsync()
    {
        if (Closed || reading) return false;
        reading = true;
        try
        {
            var value = await client.InvokeAsync("native_terminal_snapshot", new() { ["sessionId"] = sessionId }, projectId, lifetime.Token);
            if (Closed) return false;
            if (value is not JsonObject snapshot || snapshot["text"] is not JsonValue || snapshot["running"] is not JsonValue)
                throw new InvalidDataException("授权终端返回了无效状态。");
            Output = Printable(snapshot["text"]!.GetValue<string>());
            Running = snapshot["running"]!.GetValue<bool>(); Notify(); return true;
        }
        catch (OperationCanceledException) when (Closed) { return false; }
        catch (Exception ex) { if (!Closed) { Running = false; Fail("无法读取授权终端：" + ex.Message); } return false; }
        finally { reading = false; }
    }
    public async Task<bool> WriteAsync(string data)
    {
        if (!CanWrite) return false;
        var success = await RunAsync(() => client.InvokeAsync("write_terminal", new() { ["sessionId"] = sessionId, ["data"] = data }, projectId, lifetime.Token), _ => { });
        if (!success && !Closed) { InputUncertain = true; Fail("输入是否送达无法确认；不会自动重发。检查终端后再恢复输入。"); }
        return success;
    }
    public void AcknowledgeInput() { if (!Closed && !Busy) { InputUncertain = false; Notify(); } }
    public async Task<bool> CloseAsync()
    {
        if (Closed || Busy) return false;
        var success = await RunAsync(() => client.InvokeAsync("close_terminal", new() { ["sessionId"] = sessionId }, projectId, lifetime.Token), _ => Running = false);
        if (success) Dispose(); return success;
    }
    public override void Dispose()
    {
        if (Closed) return;
        base.Dispose(); lifetime.Cancel(); lifetime.Dispose(); Output = ""; Running = false;
    }
    public static string Printable(string text)
    {
        var withoutOsc = Regex.Replace(text, @"\x1B\][^\x07\x1B]*(?:\x07|\x1B\\)", "");
        var withoutCsi = Regex.Replace(withoutOsc, @"\x1B\[[0-?]*[ -/]*[@-~]", "");
        return new string(withoutCsi.Where(c => c is '\r' or '\n' or '\t' || !char.IsControl(c)).ToArray());
    }
}
