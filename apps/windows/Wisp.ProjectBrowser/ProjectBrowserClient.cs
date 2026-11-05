using System.Diagnostics;
using System.Text;
using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>One bounded, hidden service process per query. SQLite stays in Rust.</summary>
public sealed class ProjectBrowserClient(string executablePath, TimeSpan? timeout = null) : IProjectBrowserClient, IProjectChatHistory
{
    public async Task<ProjectListSnapshot> ListProjectsAsync(string databasePath, CancellationToken cancellationToken = default)
    {
        var reply = await QueryAsync(databasePath, new() { ["type"] = "list_projects" }, "projects", cancellationToken);
        return new(reply.Projects ?? throw InvalidResponse(), reply.ActivitySource!, reply.Privacy is { ProjectIds: not null } privacy ? privacy : throw InvalidResponse());
    }

    public async Task<ProjectListSnapshot> SetProjectStarredAsync(string databasePath, string projectId, bool starred,
        CancellationToken cancellationToken = default)
    {
        var reply = await QueryAsync(databasePath, new() { ["type"] = "set_project_starred", ["project_id"] = projectId,
            ["starred"] = starred }, "projects", cancellationToken);
        return new(reply.Projects ?? throw InvalidResponse(), reply.ActivitySource!, reply.Privacy is { ProjectIds: not null } privacy ? privacy : throw InvalidResponse());
    }

    public async Task<IReadOnlyList<BrowserSession>> ListSessionsAsync(string databasePath, string? projectId = null,
        CancellationToken cancellationToken = default)
    {
        var reply = await QueryAsync(databasePath, new() { ["type"] = "list_sessions", ["project_id"] = projectId }, "sessions", cancellationToken);
        return reply.Sessions ?? throw InvalidResponse();
    }

    public async Task<TranscriptPage> GetTranscriptAsync(string databasePath, string projectId, string sessionId,
        long? beforeSeq = null, CancellationToken cancellationToken = default)
    {
        var reply = await QueryAsync(databasePath, new()
        {
            ["type"] = "get_transcript", ["project_id"] = projectId,
            ["session_id"] = sessionId, ["before_seq"] = beforeSeq
        }, "transcript", cancellationToken);
        return new(reply.Messages ?? throw InvalidResponse(), reply.NextBeforeSeq);
    }

    public static ProjectBrowserResponse Decode(string json, string requestId, string expectedType)
    {
        ProjectBrowserResponse reply;
        try { reply = JsonSerializer.Deserialize<ProjectBrowserResponse>(json) ?? throw InvalidResponse(); }
        catch (JsonException ex) { throw new InvalidDataException("查询服务返回了无效 JSON。", ex); }
        if (reply.Schema != ProjectBrowserProtocol.Schema || reply.Id != requestId) throw InvalidResponse();
        if (reply.Type == "error") throw new InvalidOperationException(reply.Message ?? "查询失败。");
        if (reply.Type != expectedType || (expectedType is "projects" or "sessions" && reply.ActivitySource != ProjectBrowserProtocol.PersistedOnly))
            throw InvalidResponse();
        return reply;
    }

    public async Task<ProjectChatMatches> SearchChatsAsync(string database, string project, string query, ProjectChatCursor? cursor, CancellationToken token)
    {
        var reply = await QueryAsync(database, new() { ["type"] = "search_project_chats", ["project_id"] = project,
            ["query"] = query, ["cursor"] = cursor }, "project_chat_matches", token);
        if (reply.ProjectId != project || reply.Query != query || reply.Page is not { Items: { Length: <= 50 } items } page
            || items.Any(h => h == null || string.IsNullOrWhiteSpace(h.SessionId) || h.SessionTitle == null
                || h.Role is not ("user" or "assistant" or "tool") || h.Before == null || string.IsNullOrEmpty(h.Matched) || h.After == null)
            || page.NextCursor is { } next && (items.LastOrDefault() is not { } last ||
                next != new ProjectChatCursor(last.Ts, last.SessionId, last.MessageSeq) || next == cursor)) throw InvalidResponse();
        return page;
    }

    public async Task<ProjectChatLocation?> LocateChatAsync(string database, string project, ProjectChatHit hit, CancellationToken token)
    {
        var reply = await QueryAsync(database, new() { ["type"] = "locate_project_chat_match", ["project_id"] = project,
            ["session_id"] = hit.SessionId, ["message_seq"] = hit.MessageSeq }, "project_chat_location", token);
        if (reply.ProjectId != project || reply.SessionId != hit.SessionId || reply.MessageSeq != hit.MessageSeq) throw InvalidResponse();
        if (reply.Target.ValueKind == JsonValueKind.Null) return null;
        return reply.Target.ValueKind == JsonValueKind.Object && reply.Target.Deserialize<ProjectChatLocation>() is { UserIndex: >= 0 } target
            ? target : throw InvalidResponse();
    }

    private async Task<ProjectBrowserResponse> QueryAsync(string databasePath, Dictionary<string, object?> command,
        string expectedType, CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        if (!File.Exists(executablePath)) throw new FileNotFoundException("找不到 wisp-service.exe，请重新构建原生预览。", executablePath);
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        deadline.CancelAfter(timeout ?? TimeSpan.FromSeconds(20));
        var token = deadline.Token;
        var id = Guid.NewGuid().ToString("N");
        command["schema"] = ProjectBrowserProtocol.Schema;
        command["id"] = id;
        var start = new ProcessStartInfo(executablePath)
        {
            UseShellExecute = false, CreateNoWindow = true,
            RedirectStandardInput = true, RedirectStandardOutput = true, RedirectStandardError = true,
            StandardInputEncoding = new UTF8Encoding(false), StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding = Encoding.UTF8
        };
        start.ArgumentList.Add("--database");
        start.ArgumentList.Add(databasePath);
        if (Equals(command["type"], "set_project_starred")) start.ArgumentList.Add("--allow-project-writes");
        using var process = new Process { StartInfo = start };
        process.Start();
        // Drain both pipes concurrently, including large transcripts. Bound output as well as time.
        var output = ReadBoundedAsync(process.StandardOutput, 32 * 1024 * 1024, token);
        var errors = ReadBoundedAsync(process.StandardError, 64 * 1024, token);
        try
        {
            await process.StandardInput.WriteLineAsync(JsonSerializer.Serialize(command).AsMemory(), token);
            process.StandardInput.Close();
            await Task.WhenAll(output, errors, process.WaitForExitAsync(token));
            if (process.ExitCode != 0) throw new InvalidOperationException($"查询服务退出 ({process.ExitCode})：{await errors}");
            return Decode(await output, id, expectedType);
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            throw new TimeoutException("服务响应超时。若正在保存收藏，结果可能已写入；请刷新核实。");
        }
        finally
        {
            if (!process.HasExited) process.Kill(entireProcessTree: true);
            await process.WaitForExitAsync();
            deadline.Cancel();
            // Observe reader failures on cancellation or a broken stdin pipe.
            try { await Task.WhenAll(output, errors); } catch { }
        }
    }

    private static async Task<string> ReadBoundedAsync(StreamReader reader, int limit, CancellationToken token)
    {
        var text = new StringBuilder();
        var buffer = new char[8192];
        while (await reader.ReadAsync(buffer.AsMemory(), token) is var count && count > 0)
        {
            if (text.Length + count > limit) throw new InvalidDataException("查询响应超过预览版读取上限。");
            text.Append(buffer, 0, count);
        }
        return text.ToString();
    }

    private static InvalidDataException InvalidResponse() => new("查询服务协议不兼容，请重新构建原生预览版。");
}
