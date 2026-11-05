namespace Wisp.ProjectBrowser;

/// <summary>Ordered message controls; selection actions have their own contextual menu.</summary>
public sealed record NativeMessageAction(string Kind, string LabelKey, string? Icon, bool Enabled);
