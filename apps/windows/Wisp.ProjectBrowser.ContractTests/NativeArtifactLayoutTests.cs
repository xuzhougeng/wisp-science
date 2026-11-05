using Wisp.ProjectBrowser;

internal static class NativeArtifactLayoutTests
{
    public static void Run()
    {
        static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
        Check(new[] { 33, 37, 38 }.All(key => NativeArtifactNavigation.PdfDirection(key) == -1)
            && new[] { 34, 39, 40 }.All(key => NativeArtifactNavigation.PdfDirection(key) == 1)
            && new[] { 27, 32, 65 }.All(key => NativeArtifactNavigation.PdfDirection(key) == 0),
            "PDF page keys include Page Up/Down and arrows without consuming Escape, spaces or typing");
        var large = NativeArtifactLayout.Fit(1600, 1200, "image", 100, 800);
        Check(large.Width + NativeArtifactLayout.Inset == 880 && large.FigureHeight == 624 && large.DetailsHeight == 360,
            "large image viewer exposes canonical 52vh preview and 30vh provenance instead of smaller native caps");
        var shortWindow = NativeArtifactLayout.Fit(900, 500, "image", 100, 800);
        Check(shortWindow.FigureHeight < 260 && shortWindow.DetailsHeight == 150
            && shortWindow.FigureHeight + shortWindow.DetailsHeight + 100 <= shortWindow.MaxHeight,
            "short windows shrink the figure while keeping provenance and header controls inside the dialog");
        var empty = NativeArtifactLayout.Fit(900, 500, "image", 100, 50);
        Check(empty.FigureHeight > shortWindow.FigureHeight && empty.DetailsHeight == 50,
            "an empty or short provenance pane gives available space back to the preview");
        var narrow = NativeArtifactLayout.Fit(400, 800, "html", 100, 100);
        var html = NativeArtifactLayout.Fit(1600, 1200, "html", 100, 100);
        Check(narrow.Width + NativeArtifactLayout.Inset <= 360 && html.Width + NativeArtifactLayout.Inset == 1240,
            "HTML viewers use the wider canonical cap while narrow windows retain both overlay margins");
        foreach (var kind in new[] { "docx", "xlsx", "pptx" })
        {
            var office = NativeArtifactLayout.Fit(1600, 1200, kind, 100, 100);
            Check(office.FigureHeight == 792, "office document previews use their 66vh viewport");
        }
        var tiny = NativeArtifactLayout.Fit(60, 60, "image", 100, 300);
        Check(tiny.Width == 0 && tiny.MaxHeight == 0 && tiny.FigureHeight == 0 && tiny.DetailsHeight == 0,
            "temporarily minimized or extremely small window geometry cannot produce negative XAML sizes");
        var landscape = NativeRasterSize.Fit(1600, 900, 800, 500);
        var portraitViewport = NativeRasterSize.Fit(1600, 900, 360, 600);
        Check(landscape.ImageWidth == 800 && landscape.ImageHeight == 450
            && portraitViewport.ImageWidth == 360 && portraitViewport.ImageHeight == 202.5,
            "100 percent fits the width and resizing retains image aspect ratio");
        var shortImage = NativeRasterSize.Fit(1600, 900, 800, 60);
        Check(shortImage.CanvasHeight == 24 && shortImage.ImageHeight == 450,
            "short figures preserve fit-to-width image size and allow vertical panning at 100 percent");
        var tall = NativeRasterSize.Fit(900, 1600, 450, 500);
        Check(tall.ImageWidth == 450 && tall.ImageHeight == 800 && tall.CanvasHeight == 464,
            "portrait images overflow vertically without reducing the fit-to-width baseline");
        var quarter = NativeRasterSize.Fit(1600, 900, 800, 500, 25);
        var fourTimes = NativeRasterSize.Fit(1600, 900, 800, 500, 400);
        Check(quarter.ImageWidth == 200 && quarter.ImageHeight == 112.5
            && fourTimes.ImageWidth == 3200 && fourTimes.ImageHeight == 1800,
            "zoom scales only the image while preserving its aspect ratio");
        Check(quarter.CanvasWidth == fourTimes.CanvasWidth && quarter.CanvasHeight == fourTimes.CanvasHeight,
            "zoom does not grow the viewport or its parent dialog");
        Check(NativeRasterSize.StepZoom(100, 120) == 125 && NativeRasterSize.StepZoom(100, -240) == 75
            && NativeRasterSize.StepZoom(25, -1) == 25 && NativeRasterSize.StepZoom(400, 1) == 400
            && NativeRasterSize.StepZoom(100, 0) == 100,
            "buttons and wheel use 25 percentage point increments with canonical 25 to 400 bounds");
        Check(NativeRasterSize.Fit(1600, 900, 800, 500, 0) == quarter
            && NativeRasterSize.Fit(1600, 900, 800, 500, 500) == fourTimes,
            "out of range zoom values cannot produce unsupported image sizes");
    }
}
