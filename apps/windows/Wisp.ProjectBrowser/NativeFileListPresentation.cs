using System.Globalization;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public static class NativeFileListPresentation
{
    public static NativePanelFile[] VisibleFiles(IEnumerable<NativePanelFile> files, string query) => files
        .Where(file => file.Name.Contains(query.Trim(), StringComparison.CurrentCultureIgnoreCase))
        .OrderByDescending(file => file.IsDir)
        .ThenBy(file => file.Name, StringComparer.CurrentCultureIgnoreCase).ToArray();

    public static string Size(ulong bytes)
    {
        string[] units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
        var value = (double)bytes;
        var unit = 0;
        while (value >= 1024 && unit < units.Length - 1) { value /= 1024; unit++; }
        return value.ToString(unit == 0 ? "0" : "0.#", CultureInfo.InvariantCulture) + " " + units[unit];
    }
}
