using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Foundation;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private int petPreviewVersion;
    private void Pet()
    {
        if (model.Values["get_settings"] is not JsonObject) return;
        model.EditWindowPreferences(WindowPreferences);
        var draft = model.Draft!["get_settings"]!.AsObject();
        var pet = NativePetPresentation.From(model.Values["get_pet"]);
        Form.Spacing = 22; Body.Padding = new Thickness(24, 16, 24, 22);
        var hero = new Grid { ColumnSpacing = 24, RowSpacing = 18, MinHeight = 126 };
        hero.ColumnDefinitions.Add(new() { Width = new GridLength(116) }); hero.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        hero.RowDefinitions.Add(new() { Height = GridLength.Auto }); hero.RowDefinitions.Add(new() { Height = GridLength.Auto });
        // This question mark is the WebView's empty-state text, not an action icon.
        var placeholder = Design.Text("?", 38); placeholder.Foreground = Design.Brush("text-faint");
        placeholder.HorizontalAlignment = HorizontalAlignment.Center; placeholder.VerticalAlignment = VerticalAlignment.Center;
        var preview = new Border { Width = 116, Height = 126, CornerRadius = new CornerRadius(24), Child = placeholder,
            Background = new SolidColorBrush(Mix(Microsoft.UI.Colors.Transparent, Design.Brush("bg-sunken").Color, .76)),
            BorderBrush = new SolidColorBrush(Mix(Microsoft.UI.Colors.Transparent, Design.Brush("border").Color, .75)),
            BorderThickness = new Thickness(1), VerticalAlignment = VerticalAlignment.Center,
            HorizontalAlignment = pet.HasAsset ? HorizontalAlignment.Left : HorizontalAlignment.Center };
        hero.Children.Add(preview);
        var copy = new StackPanel { Spacing = 7, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Left };
        var name = Design.Text(pet.HasAsset ? pet.Name : T("pet.not_configured"), 21); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        copy.Children.Add(name);
        var description = Design.Text(pet.Description.Length > 0 ? pet.Description : T("pet.description"), 13);
        description.Foreground = Design.Brush("text-muted"); description.MaxWidth = 560; description.HorizontalAlignment = HorizontalAlignment.Left;
        description.LineHeight = 20.8 * Design.Typography.UiSize / 14; copy.Children.Add(description);
        if (pet.HasAsset)
        {
            var metadata = new NativeActionWrap { Margin = new Thickness(0, 5, 0, 0) };
            foreach (var value in new[] { pet.Id, "v" + pet.Version, pet.Directory })
            {
                var text = Design.Text(value, 11); text.FontFamily = Design.Font(true); text.Foreground = Design.Brush("text-muted");
                text.TextWrapping = TextWrapping.NoWrap; text.TextTrimming = TextTrimming.CharacterEllipsis;
                var tag = new Border { Child = text, MaxWidth = 360, Padding = new Thickness(7, 3, 7, 3), CornerRadius = new CornerRadius(5), Background = Design.Brush("bg-sunken") };
                ToolTipService.SetToolTip(tag, value); metadata.Children.Add(tag);
            }
            copy.Children.Add(metadata);
            if (pet.SpriteBytes != null) _ = LoadPetPreview(preview, pet.SpriteBytes, petPreviewVersion);
        }
        hero.Children.Add(copy); Grid.SetColumn(copy, 1);
        hero.SizeChanged += (_, _) =>
        {
            var layout = NativeSettingsLayout.PetHero(XamlRoot?.Size.Width ?? ActualWidth, hero.ActualWidth, pet.HasAsset);
            hero.ColumnDefinitions[0].Width = layout.FlexiblePreview ? new GridLength(1, GridUnitType.Star) : new GridLength(116);
            hero.ColumnDefinitions[1].Width = layout.Stacked ? new GridLength(0) : layout.FlexiblePreview ? GridLength.Auto : new GridLength(1, GridUnitType.Star);
            copy.MaxWidth = layout.CopyWidth;
            Grid.SetColumn(copy, layout.Stacked ? 0 : 1); Grid.SetRow(copy, layout.Stacked ? 1 : 0);
            hero.ColumnSpacing = layout.Stacked ? 0 : 24;
            hero.RowSpacing = layout.Stacked ? 24 : 0;
        };
        // CSS layers a circular glow over the diagonal background. Keep that
        // circle in DIPs when the card changes aspect ratio, rather than stretching
        // it into a bounding-box ellipse on a wide settings window.
        var layers = new Grid(); hero.Margin = new Thickness(24, 18, 24, 18);
        var heroCard = Design.Card(layers); heroCard.Padding = new Thickness(0);
        var gradient = new LinearGradientBrush { StartPoint = new Point(0, 0), EndPoint = new Point(1, 1) };
        gradient.GradientStops.Add(new GradientStop { Color = Mix(Design.Brush("bg-elev").Color, Design.Brush("clay-soft").Color, .1), Offset = 0 });
        gradient.GradientStops.Add(new GradientStop { Color = Design.Brush("bg-app").Color, Offset = 1 }); heroCard.Background = gradient;
        var glow = new RadialGradientBrush { MappingMode = BrushMappingMode.Absolute };
        var accent = Design.Brush("clay").Color;
        glow.GradientStops.Add(new GradientStop { Color = Mix(Microsoft.UI.Colors.Transparent, accent, .16), Offset = 0 });
        glow.GradientStops.Add(new GradientStop { Color = Windows.UI.Color.FromArgb(0, accent.R, accent.G, accent.B), Offset = 1 });
        var glowLayer = new Border { Background = glow, CornerRadius = heroCard.CornerRadius, IsHitTestVisible = false };
        layers.Children.Add(glowLayer); layers.Children.Add(hero);
        layers.SizeChanged += (_, e) =>
        {
            glow.Center = glow.GradientOrigin = new Point(e.NewSize.Width * .16, e.NewSize.Height * .2);
            glow.RadiusX = glow.RadiusY = NativeSettingsLayout.PetGlowRadius(e.NewSize.Width, e.NewSize.Height);
        };
        heroCard.BorderBrush = new SolidColorBrush(Mix(Design.Brush("border").Color, Design.Brush("clay").Color, .28)); Form.Children.Add(heroCard);

        var preferences = new StackPanel(); var card = Design.Card(preferences); card.Padding = new Thickness(24, 0, 24, 18); Form.Children.Add(card);
        InlineToggle(preferences, T("pet.enabled"), T("pet.enabled_hint"), B(draft, "pet_enabled"), value => draft["pet_enabled"] = value);
        var directory = new StackPanel { Spacing = 8, Margin = new Thickness(0, 18, 0, 0) };
        var label = Design.Text(T("pet.directory"), 12); label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        label.Foreground = Design.Brush("text-muted"); directory.Children.Add(label);
        var pathRow = new Grid { ColumnSpacing = 8 };
        pathRow.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); pathRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var input = new TextBox { Text = S(draft, "pet_directory"), PlaceholderText = T("pet.directory_placeholder"), MinWidth = 0,
            CornerRadius = new CornerRadius(8), Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1), Padding = new Thickness(10, 8, 10, 8) };
        Design.BindTypography(input, code: true, size: 12); AutomationProperties.SetName(input, T("pet.directory")); textMenus.Register(input);
        input.TextChanged += (_, _) => { draft["pet_directory"] = input.Text; LockConfirmation(); }; pathRow.Children.Add(input);
        var choose = Button(T("projects.choose_dir"), async () =>
        {
            if (pickPath == null) return;
            var editing = model.Draft;
            try
            {
                var selected = await pickPath(true);
                if (!model.Closed && ReferenceEquals(model.Draft, editing) && !string.IsNullOrEmpty(selected)) input.Text = selected;
            }
            catch (Exception error) { if (!model.Closed) model.Fail(error.Message); }
        });
        choose.IsEnabled = pickPath != null; Grid.SetColumn(choose, 1); pathRow.Children.Add(choose); directory.Children.Add(pathRow);
        var hint = Mute(T("pet.directory_hint")); Design.BindTypography(hint, 12); directory.Children.Add(hint); preferences.Children.Add(directory);
        if (pet.Error.Length > 0) Form.Children.Add(Warn(pet.Error));
        InlineSaveActions(); MovePreferenceFooter(Form);
    }
    private async Task LoadPetPreview(Border target, byte[] bytes, int generation)
    {
        try
        {
            using var stream = new InMemoryRandomAccessStream();
            using (var writer = new DataWriter(stream.GetOutputStreamAt(0))) { writer.WriteBytes(bytes); await writer.StoreAsync(); }
            stream.Seek(0);
            var decoder = await BitmapDecoder.CreateAsync(stream);
            if (decoder.PixelWidth != NativePetPresentation.SheetWidth || decoder.PixelHeight != NativePetPresentation.SheetHeight) return;
            stream.Seek(0); var source = new BitmapImage(); await source.SetSourceAsync(stream);
            if (model.Closed || generation != petPreviewVersion) return;
            var viewport = new Canvas { Width = NativePetPresentation.PreviewWidth, Height = NativePetPresentation.PreviewHeight,
                Clip = new RectangleGeometry { Rect = new Rect(0, 0, NativePetPresentation.PreviewWidth, NativePetPresentation.PreviewHeight) } };
            viewport.Children.Add(new Image { Source = source, Width = NativePetPresentation.PreviewWidth * NativePetPresentation.Columns,
                Height = NativePetPresentation.PreviewHeight * NativePetPresentation.Rows, Stretch = Stretch.Fill });
            target.Child = viewport;
        }
        catch { /* Invalid image bytes leave the same bounded empty preview. */ }
    }
    private static Windows.UI.Color Mix(Windows.UI.Color background, Windows.UI.Color foreground, double amount)
    {
        // CSS color-mix retains alpha; treating a translucent black border as
        // opaque creates a dark outline instead of the WebView's soft tint.
        var back = background.A * (1 - amount); var front = foreground.A * amount; var alpha = back + front;
        byte Channel(byte b, byte f) => alpha == 0 ? (byte)0 : (byte)Math.Round((b * back + f * front) / alpha);
        return Windows.UI.Color.FromArgb((byte)Math.Round(alpha), Channel(background.R, foreground.R),
            Channel(background.G, foreground.G), Channel(background.B, foreground.B));
    }
}
