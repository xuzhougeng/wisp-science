namespace Wisp.ProjectBrowser;

public static class NativeBrowserPresentation
{
    public static string Status(string? status) => status switch
    {
        "running" => "运行中", "needs_you" => "待查看", "complete" or "completed" or "done" => "已完成",
        "idle" => "空闲", "failed" or "error" => "出错", "stopped" => "已停止", _ => "状态未知"
    };

    public static string RelativeTime(long timestamp, DateTimeOffset now, TimeZoneInfo? zone = null)
    {
        if (!TryTime(timestamp, out var time)) return "时间未知";
        var age = now - time;
        var local = TimeZoneInfo.ConvertTime(time, zone ?? TimeZoneInfo.Local);
        if (age.TotalSeconds < -60) return local.ToString("yyyy-MM-dd HH:mm");
        if (age.TotalMinutes < 1) return "刚刚";
        if (age.TotalHours < 1) return $"{(int)age.TotalMinutes} 分钟前";
        if (age.TotalDays < 1) return $"{(int)age.TotalHours} 小时前";
        if (age.TotalDays < 7) return $"{(int)age.TotalDays} 天前";
        return local.ToString(local.Year == TimeZoneInfo.ConvertTime(now, zone ?? TimeZoneInfo.Local).Year ? "MM-dd" : "yyyy-MM-dd");
    }

    public static string ExactTime(long timestamp) => TryTime(timestamp, out var value) ? value.LocalDateTime.ToString("yyyy-MM-dd HH:mm:ss") : "时间未知";
    private static bool TryTime(long timestamp, out DateTimeOffset value)
    {
        value = default;
        if (timestamp <= 0 || timestamp > 253402300799) return false;
        value = DateTimeOffset.FromUnixTimeSeconds(timestamp); return true;
    }
}
