using Wisp.ProjectBrowser;

internal static class NativeBrowserPresentationTests
{
    public static void Run()
    {
        var now = new DateTimeOffset(2026, 10, 2, 12, 0, 0, TimeSpan.Zero);
        var seconds = now.ToUnixTimeSeconds();
        Check(NativeBrowserPresentation.Status("running") == "运行中", "running must not look complete");
        Check(NativeBrowserPresentation.Status("future_state") == "状态未知", "unknown status is not successful completion");
        Check(NativeBrowserPresentation.Status("needs_you") == "待查看", "needs-you status stays visible");
        Check(NativeBrowserPresentation.RelativeTime(seconds - 59, now) == "刚刚", "minute boundary");
        Check(NativeBrowserPresentation.RelativeTime(seconds - 60, now) == "1 分钟前", "exact minute");
        Check(NativeBrowserPresentation.RelativeTime(seconds - 3600, now) == "1 小时前", "exact hour");
        Check(NativeBrowserPresentation.RelativeTime(seconds - 86400, now) == "1 天前", "exact day");
        Check(NativeBrowserPresentation.RelativeTime(seconds + 3600, now, TimeZoneInfo.Utc) == "2026-10-02 13:00", "future clocks show an exact date");
        Check(NativeBrowserPresentation.RelativeTime(0, now) == "时间未知" && NativeBrowserPresentation.RelativeTime(long.MaxValue, now) == "时间未知", "missing and invalid timestamps cannot crash cards");
        Check(NativeBrowserPresentation.RelativeTime(new DateTimeOffset(2025, 12, 1, 0, 0, 0, TimeSpan.Zero).ToUnixTimeSeconds(), now, TimeZoneInfo.Utc) == "2025-12-01", "older years remain unambiguous");
        Console.WriteLine("Native browser status and timestamp presentation passed.");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
}
