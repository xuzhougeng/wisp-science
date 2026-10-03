using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSideChatPresentationTests
{
    public static async Task RunAsync()
    {
        var client = new Client();
        var model = new WorkspaceSideChatModel(client, "project", "session");
        await model.LoadOptionsAsync();
        Check(client.Selected == null && client.AskCount == 0, "mounting the model picker must not write settings or send a question");
        Check(!model.CanSend, "empty draft cannot send");
        model.Draft = "保留这个未发送的问题";
        Check(model.CanSend, "typing a draft enables sending");
        var select = model.SelectAsync(model.Options[1]);
        Check(model.ChangingModel && !model.CanSend, "sending stays disabled while model selection is unconfirmed");
        Check(model.Draft == "保留这个未发送的问题", "changing models preserves the unsent draft");
        client.Confirm.TrySetResult();
        await select;
        Check(model.Selected?.Id == "b" && model.CanSend && client.AskCount == 0, "confirmed selection restores sending without dispatching the draft");
        model.Draft = "";
        Check(!model.CanSend, "clearing the input disables sending again");
        Console.WriteLine("Native side-chat picker and unsent draft states passed.");
    }

    private static void Check(bool valid, string message) { if (!valid) throw new InvalidOperationException(message); }
    private sealed class Client : INativeSideChatClient
    {
        public TaskCompletionSource Confirm { get; } = new();
        public string? Selected;
        public int AskCount;
        public Task<NativeSideChatOption[]> OptionsAsync(string project, string session, CancellationToken token = default)
            => Task.FromResult<NativeSideChatOption[]>([new("a", "First", "http", Selected != "b"), new("b", "Second", "http", Selected == "b")]);
        public async Task SelectHttpModelAsync(string project, string modelId, CancellationToken token = default)
        { await Confirm.Task; Selected = modelId; }
        public Task<NativeSideChatResponse> AskAsync(string project, string session, string question, string? acpAgentId = null, CancellationToken token = default)
        { AskCount++; throw new InvalidOperationException("UI state checks must not send"); }
    }
}
