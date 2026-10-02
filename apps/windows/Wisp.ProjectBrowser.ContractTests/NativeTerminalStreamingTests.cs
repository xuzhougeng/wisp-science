using System.Text;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeTerminalStreamingTests
{
    public static async Task RunAsync()
    {
        var client = new Fake();
        var model = new WorkspaceTerminalModel(client, "p", "s");
        await model.LoadAsync();
        var bytes = Encoding.UTF8.GetBytes("根");
        client.Chunks.Enqueue(new("a", 0, 1, Convert.ToBase64String(bytes[..1]), true, null));
        client.Chunks.Enqueue(new("a", 1, 3, Convert.ToBase64String(bytes[1..]), false, null));
        await model.ReadAsync(); Check(model.Output == "", "partial UTF-8 is buffered");
        await model.ReadAsync(); Check(model.Output == "根", "UTF-8 survives byte chunk boundaries");
        client.PendingWrite = new();
        var first = model.WriteAsync([1]);
        var queued = model.WriteAsync([2]);
        model.Select("b");
        client.PendingWrite.SetException(new IOException("lost response"));
        await first; await queued;
        Check(client.Writes.SequenceEqual(["a"]) && !model.InputUncertain && model.Error == null,
            "late failure belongs to old terminal and queued old input is discarded");
        model.Select("a"); Check(model.InputUncertain, "uncertainty survives switching away and back");
        await model.WriteAsync([3]); Check(client.Writes.Count == 1, "uncertain input is never replayed");
        model.ResumeInput(); client.PendingWrite = null;
        await model.WriteAsync([4]); Check(client.Writes.Count == 2, "explicit acknowledgement enables new input");
        await model.ResizeAsync(30, 100); Check(client.Size == (30, 100), "terminal rows and columns reach host");
        client.PendingRead = new(); var lateRead = model.ReadAsync(); model.Select("b");
        client.PendingRead.SetResult(new("a", 3, 4, "eA==", false, null)); await lateRead;
        Check(model.Output == "", "late output cannot enter another terminal");
        client.PendingList = new(); var lateList = model.ReloadAsync(); model.Detach();
        client.PendingList.SetResult([new("late", "p", "local", "late", "pty", ".", true)]); await lateList;
        Check(model.Terminals.All(row => row.Id != "late"), "detached terminal ignores late list results");
        Console.WriteLine("Native VT streaming, UTF-8, ordered input, selection isolation, resize and detach passed.");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeTerminalClient
    {
        public Queue<NativeTerminalOutput> Chunks = new();
        public List<string> Writes = [];
        public TaskCompletionSource? PendingWrite;
        public TaskCompletionSource<NativeTerminalOutput>? PendingRead;
        public TaskCompletionSource<NativeTerminalInfo[]>? PendingList;
        public (ushort, ushort) Size;
        public Task<NativeTerminalInfo[]> ListAsync(string p, string s, CancellationToken token = default) => PendingList?.Task
            ?? Task.FromResult(new[] { new NativeTerminalInfo("a", "p", "local", "A", "pty", ".", true), new NativeTerminalInfo("b", "p", "local", "B", "pty", ".", true) });
        public Task<NativeTerminalInfo> OpenAsync(string p, string s, string c, CancellationToken token = default) => throw new NotImplementedException();
        public Task<NativeTerminalOutput> ReadAsync(string p, string s, string id, ulong? cursor, CancellationToken token = default) => PendingRead?.Task ?? Task.FromResult(Chunks.Dequeue());
        public Task WriteAsync(string p, string s, string id, byte[] bytes, CancellationToken token = default) { Writes.Add(id); return PendingWrite?.Task ?? Task.CompletedTask; }
        public Task ResizeAsync(string p, string s, string id, ushort rows, ushort cols, CancellationToken token = default) { Size = (rows, cols); return Task.CompletedTask; }
        public Task CloseAsync(string p, string s, string id, CancellationToken token = default) => Task.CompletedTask;
    }
}
