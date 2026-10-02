using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record JourneyDay(DateOnly Day, JourneyEntry[] Entries);

public sealed class WorkspaceJourneyModel(INativeJourneyClient client, string projectId) : WorkspaceActionModel
{
    public JourneyPage? Page { get; private set; }
    public string Query { get; set; } = "";
    public string Kind { get; set; } = "";
    public JourneyEntry? Selected { get; private set; }
    public JourneyArtifact? Artifact { get; private set; }
    public NativeRun? Run { get; private set; }
    public string? SourceRunId => Artifact != null ? Artifact.Source.RunId : Selected?.RunId;
    public string? SourceError { get; private set; }
    public bool SourceLoading { get; private set; }
    public bool HasParentSource => sources.Count > 0;
    private readonly Stack<JourneyArtifact> sources = new();
    private int sourceGeneration;

    public Task<bool> LoadAsync(long from, long until)
    {
        if (from >= until) { Fail("请选择有效日期范围。"); return Task.FromResult(false); }
        return RunAsync(() => client.ReadAsync(projectId, from, until), value => { Page = value; ClearSelection(); });
    }

    public JourneyDay[] Days(TimeZoneInfo? zone = null) => (Page?.Entries ?? [])
        .Where(row => (Kind.Length == 0 || row.Kind == Kind) && (row.Title.Contains(Query.Trim(), StringComparison.CurrentCultureIgnoreCase)
            || row.Summary.Contains(Query.Trim(), StringComparison.CurrentCultureIgnoreCase)))
        .OrderByDescending(row => row.OccurredAt).ThenBy(row => row.Id, StringComparer.Ordinal)
        .GroupBy(row => DateOnly.FromDateTime(TimeZoneInfo.ConvertTime(DateTimeOffset.FromUnixTimeSeconds(row.OccurredAt), zone ?? TimeZoneInfo.Local).DateTime))
        .Select(group => new JourneyDay(group.Key, group.ToArray())).ToArray();

    public async Task SelectAsync(JourneyEntry entry)
    {
        if (Closed) return;
        ClearSelection(); Selected = entry; Notify();
        if (entry.Kind == "artifact" && !entry.SourceDiscarded && !string.IsNullOrEmpty(entry.SourceId)) await OpenArtifactAsync(entry.SourceId);
    }

    public async Task OpenArtifactAsync(string version)
    {
        if (Closed || Selected == null || SourceLoading || string.IsNullOrWhiteSpace(version)) return;
        var current = ++sourceGeneration;
        var previous = Artifact;
        SourceLoading = true; SourceError = null; Notify();
        try
        {
            var result = await client.ArtifactAsync(projectId, version);
            if (Closed || current != sourceGeneration) return;
            if (result.VersionId != version) throw new InvalidDataException("产物版本不匹配。");
            if (previous != null) sources.Push(previous);
            Artifact = result;
        }
        catch (Exception ex) { if (!Closed && current == sourceGeneration) SourceError = ex.Message; }
        finally { if (!Closed && current == sourceGeneration) { SourceLoading = false; Notify(); } }
    }

    public async Task OpenRunAsync()
    {
        if (Closed || SourceLoading || SourceRunId is not { Length: > 0 } id) return;
        var current = ++sourceGeneration;
        SourceLoading = true; SourceError = null; Notify();
        try
        {
            var result = await client.RunAsync(projectId, id);
            if (Closed || current != sourceGeneration) return;
            if (result.Id != id) throw new InvalidDataException("运行来源不匹配。");
            Run = result;
        }
        catch (Exception ex) { if (!Closed && current == sourceGeneration) SourceError = ex.Message; }
        finally { if (!Closed && current == sourceGeneration) { SourceLoading = false; Notify(); } }
    }

    public void Back()
    {
        sourceGeneration++; SourceLoading = false; SourceError = null;
        if (Run != null) Run = null;
        else if (sources.TryPop(out var previous)) Artifact = previous;
        else ClearSelection();
        Notify();
    }
    private void ClearSelection() { sourceGeneration++; Selected = null; Artifact = null; Run = null; SourceLoading = false; SourceError = null; sources.Clear(); }
    public override void Dispose() { ClearSelection(); base.Dispose(); }
    public static string KindLabel(string kind) => kind switch
    {
        "artifact" => "产物", "run" => "运行", "session" => "会话", "archive" => "归档", "finding" => "发现",
        "decision" => "决策", "progress" => "进展", "next" => "下一步", "data" => "数据", "paper" => "文献",
        _ => string.IsNullOrEmpty(kind) ? "记录" : kind
    };
}
