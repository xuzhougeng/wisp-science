namespace Wisp.ProjectBrowser;

public static class NativeBrowserPresentation
{
    // Presentation only: retain the original title in storage and tooltips.
    public static string SessionTitle(string title, NativeLocalization? locale = null)
    {
        foreach (var marker in new[] { "Selected skills:", "Uploaded files:", "Attached sessions:", "Target environments:" })
        {
            var index = title.IndexOf(marker, StringComparison.Ordinal);
            if (index >= 0) title = title[..index];
        }
        title = title.Trim();
        if (title.Length == 0) return locale?.Text("sidebar.untitled") ?? "未命名会话";
        var line = title.Split(['\r', '\n'], StringSplitOptions.RemoveEmptyEntries)[0].Trim();
        if (line.StartsWith('/') || line.Length > 2 && char.IsLetter(line[0]) && line[1] == ':' && line[2] is '\\' or '/')
        {
            var nextPath = System.Text.RegularExpressions.Regex.Match(line, @"\s+(?=(?:[A-Za-z]:[\\/]|/))");
            var path = (nextPath.Success ? line[..nextPath.Index] : line).TrimEnd('/', '\\');
            var parts = path.Replace('\\', '/').Split('/', StringSplitOptions.RemoveEmptyEntries);
            if (parts.Length == 0) return line;
            // Stored titles can truncate a path mid-component. Keep directory
            // context when there is no filename extension to identify the task.
            return Path.HasExtension(parts[^1]) ? parts[^1] : string.Join('/', parts.TakeLast(3));
        }
        return line;
    }

    public static string ConnectionError(Exception error) => error switch
    {
        OperationCanceledException or TimeoutException => "连接桌面服务超时。请重试；若仍失败，请检查完整安装包和桌面版本。",
        FileNotFoundException => "未找到此数据库可用的桌面服务。请安装完整原生版本，或先启动使用该数据库的桌面客户端。",
        System.Net.Http.HttpRequestException => "桌面服务连接已断开或拒绝请求。请重新连接后重试。",
        _ => "无法连接桌面服务：" + error.Message
    };

    public static string Status(string? status) => status switch
    {
        "running" => "运行中", "needs_you" => "待查看", "complete" or "completed" or "done" => "已完成",
        "idle" => "空闲", "failed" or "error" => "出错", "stopped" => "已停止", _ => "状态未知"
    };

    // Same thresholds and timestamp units as WebView format_relative_time.
    public static string RelativeTime(long timestamp, DateTimeOffset now, NativeLocalization? locale = null)
    {
        if (timestamp <= 0) return "";
        var millis = timestamp > 1_000_000_000_000 ? timestamp : timestamp * 1000;
        var current = now.ToUnixTimeMilliseconds();
        var age = millis >= current ? 0 : (current - millis) / 1000;
        if (age < 45) return locale?.Text("time.just_now") ?? "刚刚";
        var (key, count, unit) = age < 3600 ? ("time.minutes", Math.Max(1, age / 60), "分钟")
            : age < 86400 ? ("time.hours", age / 3600, "小时") : ("time.days", age / 86400, "天");
        return locale?.Format(key, ("n", count.ToString(System.Globalization.CultureInfo.InvariantCulture))) ?? $"{count} {unit}";
    }

    public static string ExactTime(long timestamp)
    {
        if (timestamp <= 0) return "";
        var millis = timestamp > 1_000_000_000_000 ? timestamp : timestamp * 1000;
        return millis > DateTimeOffset.MaxValue.ToUnixTimeMilliseconds() ? ""
            : DateTimeOffset.FromUnixTimeMilliseconds(millis).LocalDateTime.ToString("yyyy-MM-dd HH:mm:ss");
    }
}
