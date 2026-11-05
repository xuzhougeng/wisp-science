namespace Wisp.ProjectBrowser;

public sealed class NativeArtifactEditor
{
    public bool Editing { get; private set; }
    public string Draft { get; set; } = "";
    public void Toggle(string original) { if (!Editing) Draft = original; Editing = !Editing; }
    public void Cancel() => Editing = false;
    public void Reset() { Editing = false; Draft = ""; }
    public string? Message(string name, string language, string locale)
    {
        if (!Editing || string.IsNullOrWhiteSpace(Draft)) return null;
        var prompt = NativeLocalization.Normalize(locale) == "zh"
            ? $"我修改了生成 {name} 的代码，请用下面这版重新运行，重新生成产物："
            : $"I edited the code that produced {name}. Please re-run this version and regenerate the artifact:";
        return $"{prompt}\n```{language}\n{Draft.TrimEnd()}\n```";
    }
}
