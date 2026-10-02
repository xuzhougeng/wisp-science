using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed class WorkspaceScratchModel(INativeScratchClient client) : WorkspaceActionModel
{
    public ScratchSession? Session { get; private set; }
    public Task<bool> OpenAsync() => Session != null ? Task.FromResult(false) : RunAsync(async () =>
    {
        var value = await client.OpenAsync();
        if (!value.ProjectId.StartsWith("scratch:", StringComparison.Ordinal) || string.IsNullOrWhiteSpace(value.SessionId))
            throw new InvalidDataException("Invalid scratch session identity");
        return value;
    }, value => Session = value);
    public Task<bool> CloseAsync() => Session == null ? Task.FromResult(false) : RunAsync(async () =>
    {
        if (!await client.CloseAsync(Session.ProjectId)) throw new InvalidOperationException("随手聊尚未确认关闭。");
        return true;
    }, _ => Session = null);
}
