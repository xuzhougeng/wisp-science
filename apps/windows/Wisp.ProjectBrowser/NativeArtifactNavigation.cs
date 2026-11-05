namespace Wisp.ProjectBrowser;

public static class NativeArtifactNavigation
{
    public static int Direction(int key, bool blocked, bool editing, bool modifiers)
        => blocked || editing || modifiers ? 0 : key == 37 ? -1 : key == 39 ? 1 : 0;
    public static int PdfDirection(int key)
        => key is 33 or 37 or 38 ? -1 : key is 34 or 39 or 40 ? 1 : 0;
}
