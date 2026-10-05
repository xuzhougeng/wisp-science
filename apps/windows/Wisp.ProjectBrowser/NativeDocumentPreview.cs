namespace Wisp.ProjectBrowser;

/// <summary>Only explicitly supported document MIME types enter the local renderer.</summary>
public static class NativeDocumentPreview
{
    public static string? Kind(string? mime) => mime switch
    {
        "application/pdf" => "pdf",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        _ => null,
    };
}
