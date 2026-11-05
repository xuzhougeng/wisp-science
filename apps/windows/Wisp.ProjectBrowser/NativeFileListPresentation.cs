using System.Globalization;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public static class NativeFileListPresentation
{
    public static string DownloadName(string name)
    {
        var clean = new string(name.Select(c => c < 32 || "<>:\"/\\|?*".Contains(c) ? '_' : c).ToArray()).Trim().TrimEnd('.');
        if (clean.Length == 0) return "download";
        var stem = clean.Split('.')[0].TrimEnd();
        if (new[] { "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$" }.Contains(stem, StringComparer.OrdinalIgnoreCase)
            || System.Text.RegularExpressions.Regex.IsMatch(stem, @"^(COM|LPT)[1-9¹²³]$", System.Text.RegularExpressions.RegexOptions.IgnoreCase)) clean = "_" + clean;
        return clean;
    }
    public static NativePanelFile[] VisibleFiles(IEnumerable<NativePanelFile> files, string query, string sort = "name")
    {
        var ordered = files.Where(file => file.Name.Contains(query.Trim(), StringComparison.CurrentCultureIgnoreCase))
            .OrderByDescending(file => file.IsDir);
        if (sort == "size") ordered = ordered.ThenByDescending(file => file.Size);
        if (sort == "modified") ordered = ordered.ThenByDescending(file => file.ModifiedUnixMillis ?? 0);
        return ordered.ThenBy(file => file.Name, StringComparer.CurrentCultureIgnoreCase).ToArray();
    }

    public static string Size(ulong bytes)
    {
        string[] units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
        var value = (double)bytes;
        var unit = 0;
        while (value >= 1024 && unit < units.Length - 1) { value /= 1024; unit++; }
        return value.ToString(unit == 0 ? "0" : "0.#", CultureInfo.InvariantCulture) + " " + units[unit];
    }
}
