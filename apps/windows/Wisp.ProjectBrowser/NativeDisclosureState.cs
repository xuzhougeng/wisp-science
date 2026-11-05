namespace Wisp.ProjectBrowser;

// Automatic expansion follows lifecycle changes until the user chooses. The
// nullable choice is preserved independently of the previous rendered value.
public sealed class NativeDisclosureState(bool automatic = false)
{
    public bool Automatic { get; set; } = automatic;
    public bool? Choice { get; set; }
    public bool HasBody { get; set; } = true;
    public bool Expanded => HasBody && (Choice ?? Automatic);
}
