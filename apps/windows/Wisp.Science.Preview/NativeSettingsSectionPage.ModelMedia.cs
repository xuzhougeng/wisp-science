using System.Text.Json.Nodes;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private StackPanel ModelMediaFields(JsonObject draft, NativeModelMediaKind kind)
    {
        var catalog = JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Assets", "model-media-options.json")))!.AsObject();
        void Select(string key, string label, string options, string fallback)
        {
            var values = NativeModelMedia.Options(catalog, options);
            var current = NativeModelMedia.Display(draft, key, fallback);
            Choice(Form, T(label), current, values.Select(value => (value, value)).ToArray(), value => draft[key] = value);
            // Preserve a future/previous provider value rather than silently replacing it.
            var combo = (ComboBox)Form.Children.Last();
            if (!values.Contains(current))
            {
                var item = new ComboBoxItem { Content = current + " (" + T("composer.effort.unverified") + ")", Tag = current, IsEnabled = false };
                combo.Items.Insert(0, item); combo.SelectedItem = item;
            }
        }
        return ModelFields(() =>
        {
            if (kind == NativeModelMediaKind.GrokImage)
            {
                ModelColumns(() => Select("image_aspect_ratio", "settings.image_aspect_ratio", "GROK_IMAGE_ASPECT_RATIOS", "auto"),
                    () => Select("image_resolution", "settings.image_resolution", "GROK_IMAGE_RESOLUTIONS", "1k"));
                ModelColumns(() => Select("image_quality", "settings.image_quality", "GROK_IMAGE_QUALITIES", "medium"), () => { });
            }
            else if (kind == NativeModelMediaKind.Image)
                ModelColumns(() => Select("image_size", "settings.image_size", "OPENAI_IMAGE_SIZES", "auto"),
                    () => Select("image_quality", "settings.image_quality", "OPENAI_IMAGE_QUALITIES", "auto"));
            else if (kind == NativeModelMediaKind.Video)
            {
                ModelColumns(() =>
                {
                    TextBox? input = null;
                    input = Field(T("settings.video_duration"), NativeModelMedia.Display(draft, "video_duration_secs", catalog["VIDEO_DURATION_DEFAULT_SECS"]!.ToString()), value =>
                    {
                        var duration = NativeModelMedia.Duration(value, catalog); draft["video_duration_secs"] = duration;
                        if (input != null && input.Text != duration.ToString()) { input.Text = duration.ToString(); input.SelectionStart = input.Text.Length; }
                    });
                    input.InputScope = new Microsoft.UI.Xaml.Input.InputScope { Names = { new Microsoft.UI.Xaml.Input.InputScopeName { NameValue = Microsoft.UI.Xaml.Input.InputScopeNameValue.Number } } };
                }, () => Select("video_aspect_ratio", "settings.video_aspect_ratio", "VIDEO_ASPECT_RATIOS", "16:9"));
                ModelColumns(() => Select("video_resolution", "settings.video_resolution", "VIDEO_RESOLUTIONS", "720p"), () => { });
            }
            if (kind != NativeModelMediaKind.Chat) Form.Children.Add(ModelHint(T(kind == NativeModelMediaKind.Video ? "settings.video_defaults_hint" : "settings.image_defaults_hint")));
        });
    }
}
