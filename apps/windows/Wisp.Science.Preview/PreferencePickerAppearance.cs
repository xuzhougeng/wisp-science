using System.Runtime.CompilerServices;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Data;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

// Keep ComboBox's selection, type-ahead, scrolling, automation and Escape stack.
// Only adapt its stock template's spacing, colors and placement target.
internal static class PreferencePickerAppearance
{
    private static readonly ConditionalWeakTable<ComboBox, Adapter> adapters = new();
    public static void Apply(ComboBox combo, WispDesign design)
        => adapters.GetValue(combo, item => new Adapter(item, design));
    public static void SizeToOptions(ComboBox combo, WispDesign design)
        => adapters.GetValue(combo, item => new Adapter(item, design)).EnableAutoWidth();

    private sealed class Adapter
    {
        private readonly ComboBox combo;
        private readonly WispDesign design;
        private readonly double maximumHeight;
        private readonly ConditionalWeakTable<ComboBoxItem, object> items = new();
        private FrameworkElement? templateRoot;
        private TextBlock? openValue;
        private bool placementQueued;
        private Popup? positionedPopup;
        private ItemsPresenter? positionedItems;
        private ScrollViewer? positionedScroll;
        private long verticalOffsetToken, horizontalOffsetToken;
        private long itemsMarginToken;
        private bool keepingOffsets;
        private bool autoWidth;

        public void EnableAutoWidth()
        {
            if (autoWidth) return;
            autoWidth = true;
            void MeasureOptions()
            {
                var sample = new TextBlock { FontFamily = combo.FontFamily, FontWeight = combo.FontWeight, FontSize = combo.FontSize };
                var longest = 0d;
                foreach (var item in combo.Items.OfType<ComboBoxItem>())
                {
                    sample.Text = item.Content as string ?? "";
                    sample.Measure(new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity));
                    longest = Math.Max(longest, sample.DesiredSize.Width);
                }
                combo.Width = Wisp.ProjectBrowser.NativeSettingsLayout.GeneralChoiceWidth(longest);
            }
            combo.Loaded += (_, _) => MeasureOptions();
            combo.RegisterPropertyChangedCallback(Control.FontSizeProperty, (_, _) => MeasureOptions());
            combo.RegisterPropertyChangedCallback(Control.FontFamilyProperty, (_, _) => MeasureOptions());
            MeasureOptions();
            PrepareTemplate(); // the adapter may have already realized the stock template
        }

        public Adapter(ComboBox combo, WispDesign design)
        {
            this.combo = combo; this.design = design; maximumHeight = combo.MaxDropDownHeight;
            combo.Resources["ComboBoxDropdownContentMargin"] = new Thickness(0);
            combo.Resources["ComboBoxDropdownBorderPadding"] = new Thickness(0);
            combo.Resources["ComboBoxItemThemePadding"] = new Thickness(11, 4, 11, 4);
            combo.Resources["ComboBoxItemCornerRadius"] = new CornerRadius(0);
            combo.Resources["ComboBoxDropDownBackground"] = design.Brush("bg-input");
            combo.Resources["ComboBoxDropDownBorderBrush"] = design.Brush("border-strong");
            combo.Resources["ComboBoxItemForeground"] = design.Brush("text");
            var selected = new SolidColorBrush(Windows.UI.Color.FromArgb(255, 118, 118, 118));
            var white = new SolidColorBrush(Microsoft.UI.Colors.White);
            foreach (var state in new[] { "Selected", "SelectedUnfocused", "SelectedPointerOver", "SelectedPressed" })
            {
                combo.Resources["ComboBoxItemBackground" + state] = selected;
                combo.Resources["ComboBoxItemForeground" + state] = white;
            }
            combo.Loaded += (_, _) => { PrepareTemplate(); PrepareItems(); UpdateOpenValue(); };
            combo.DropDownOpened += (_, _) =>
            {
                PrepareTemplate();
                UpdateOpenValue();
                PrepareItems();
                // ComboBox computes offsets while opening. Place its popup after
                // that pass, while retaining native root-boundary fallback.
                SchedulePlacement();
            };
            combo.DropDownClosed += (_, _) => UpdateOpenValue();
            combo.SelectionChanged += (_, _) => { UpdateOpenValue(); SchedulePlacement(); };
            combo.SizeChanged += (_, _) => SchedulePlacement();
            combo.Unloaded += (_, _) => DetachPopup();
            PrepareTemplate();
            PrepareItems();
        }

        private void PrepareTemplate()
        {
            combo.ApplyTemplate();
            if (Find(combo, "LayoutRoot") is not FrameworkElement root) return;
            // The native 38px arrow column clips long English choices even
            // when the outer control has the exact WebView select width.
            if (autoWidth && Find(root, "DropDownGlyph") is FrameworkElement { Parent: Grid arrowGrid } glyph
                && Grid.GetColumn(glyph) == 1 && arrowGrid.ColumnDefinitions.Count > 1)
            {
                arrowGrid.ColumnDefinitions[1].Width = new GridLength(Wisp.ProjectBrowser.NativeSettingsLayout.GeneralChoiceArrow);
                glyph.Margin = new Thickness(0, 0, 10, 0);
            }
            if (ReferenceEquals(root, templateRoot) && openValue != null) return;
            // Early row decoration can see LayoutRoot before ContentPresenter
            // is realized. Retry on Loaded/open instead of caching that miss.
            if (!ReferenceEquals(root, templateRoot)) openValue = null;
            templateRoot = root;
            if (Find(root, "ContentPresenter") is ContentPresenter presenter && presenter.Parent is Grid grid)
            {
                // ComboBox moves the selected item's content into its popup.
                // Keep a noninteractive text copy in the original field while
                // open; it must not become a second accessibility value.
                openValue = new TextBlock { IsHitTestVisible = false, TextTrimming = TextTrimming.CharacterEllipsis,
                    MaxLines = 1, VerticalAlignment = VerticalAlignment.Center, Margin = presenter.Margin,
                    Visibility = Visibility.Collapsed };
                foreach (var (property, path) in new[] { (TextBlock.FontSizeProperty, "FontSize"),
                    (TextBlock.FontFamilyProperty, "FontFamily"), (TextBlock.FontWeightProperty, "FontWeight"),
                    (TextBlock.ForegroundProperty, "Foreground") })
                    openValue.SetBinding(property, new Binding { Source = combo, Path = new PropertyPath(path), Mode = BindingMode.OneWay });
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetAccessibilityView(openValue,
                    Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
                Grid.SetRow(openValue, Grid.GetRow(presenter)); Grid.SetColumn(openValue, Grid.GetColumn(presenter));
                Grid.SetRowSpan(openValue, Grid.GetRowSpan(presenter)); Grid.SetColumnSpan(openValue, Grid.GetColumnSpan(presenter));
                grid.Children.Add(openValue);
            }
            // The stock split animation hides the selected value and clips the
            // menu for its old overlapping position. Our menu stays below the
            // visible field, so retain the states but omit that animation pair.
            foreach (var group in VisualStateManager.GetVisualStateGroups(root))
                if (group.Name == "DropDownStates")
                    foreach (var state in group.States)
                        if (state.Storyboard is { } storyboard)
                            for (var index = storyboard.Children.Count - 1; index >= 0; index--)
                                if (storyboard.Children[index] is SplitOpenThemeAnimation or SplitCloseThemeAnimation)
                                    storyboard.Children.RemoveAt(index);
        }

        private void UpdateOpenValue()
        {
            if (openValue == null) return;
            var value = (combo.SelectedItem is ComboBoxItem selected ? selected.Content : combo.SelectedItem) as string;
            openValue.Text = value ?? "";
            openValue.Visibility = combo.IsDropDownOpen && value != null ? Visibility.Visible : Visibility.Collapsed;
        }

        private void SchedulePlacement()
        {
            if (!combo.IsDropDownOpen || placementQueued) return;
            placementQueued = combo.DispatcherQueue.TryEnqueue(() =>
            {
                placementQueued = false;
                // Type-ahead commits selection while the native menu remains
                // open and recomputes its old overlapping offsets afterwards.
                if (combo.IsDropDownOpen && combo.IsLoaded) Place();
            });
        }

        private void PrepareItems()
        {
            for (var index = 0; index < combo.Items.Count; index++)
            {
                var item = combo.Items[index] as ComboBoxItem ?? combo.ContainerFromIndex(index) as ComboBoxItem;
                if (item == null) continue;
                if (!items.TryGetValue(item, out _))
                {
                    items.Add(item, new object());
                    item.Padding = new Thickness(11, 4, 11, 4);
                    item.BorderThickness = new Thickness(0);
                    item.MinHeight = 0; item.CornerRadius = new CornerRadius(0);
                    item.UseSystemFocusVisuals = false;
                    item.Loaded += (_, _) => TrimItem(item);
                }
                TrimItem(item);
            }
        }

        private static void TrimItem(ComboBoxItem item)
        {
            item.ApplyTemplate();
            if (Find(item, "LayoutRoot") is FrameworkElement root) root.Margin = new Thickness(0);
            if (Find(item, "Pill") is FrameworkElement pill) pill.Visibility = Visibility.Collapsed;
        }

        private void Place()
        {
            combo.ApplyTemplate();
            if (Find(combo, "Popup") is not Popup popup || popup.Child is not FrameworkElement child
                || Find(combo, "Background") is not FrameworkElement field || combo.XamlRoot == null) return;
            PrepareItems();
            var origin = field.TransformToVisual(combo.XamlRoot.Content).TransformPoint(new Windows.Foundation.Point());
            combo.MaxDropDownHeight = Math.Min(maximumHeight,
                NativeSettingsLayout.PreferenceMenuAvailableHeight(combo.XamlRoot.Size.Height, origin.Y, field.ActualHeight));
            child.Margin = new Thickness(0);
            // WinUI wraps PopupBorder in a canvas when opening. Style the named
            // border inside that wrapper rather than assuming it is Popup.Child.
            if (Find(child, "PopupBorder") is Border border)
            {
                border.Margin = new Thickness(0); border.Width = field.ActualWidth;
                border.Shadow = null; border.Translation = System.Numerics.Vector3.Zero;
                border.Padding = new Thickness(0); border.CornerRadius = new CornerRadius(0);
                border.Background = design.Brush("bg-input"); border.BorderBrush = design.Brush("border-strong");
                border.BorderThickness = new Thickness(1);
            }
            var scroll = Find(child, "ScrollViewer") as ScrollViewer;
            if (scroll != null) scroll.MinWidth = 0;
            child.Width = field.ActualWidth;
            popup.PlacementTarget = field;
            popup.DesiredPlacement = PopupPlacementMode.BottomEdgeAlignedLeft;
            if (positionedPopup != popup)
            {
                DetachPopup(); positionedPopup = popup;
                verticalOffsetToken = popup.RegisterPropertyChangedCallback(Popup.VerticalOffsetProperty, (_, _) => KeepOffsets());
                horizontalOffsetToken = popup.RegisterPropertyChangedCallback(Popup.HorizontalOffsetProperty, (_, _) => KeepOffsets());
            }
            positionedScroll = scroll;
            if (FindType<ItemsPresenter>(child) is { } presenter && positionedItems != presenter)
            {
                if (positionedItems != null) positionedItems.UnregisterPropertyChangedCallback(FrameworkElement.MarginProperty, itemsMarginToken);
                positionedItems = presenter;
                itemsMarginToken = presenter.RegisterPropertyChangedCallback(FrameworkElement.MarginProperty, (_, _) => SchedulePlacement());
            }
            UpdateItemsInset();
            KeepOffsets();
        }

        private void KeepOffsets()
        {
            if (keepingOffsets || positionedPopup == null || !combo.IsDropDownOpen) return;
            // Native type-ahead updates these after SelectionChanged and layout,
            // even after our dispatcher callback. Keep the target-relative gap;
            // DesiredPlacement still owns flipping and root-boundary clamping.
            keepingOffsets = true;
            try { positionedPopup.HorizontalOffset = 0; positionedPopup.VerticalOffset = 3; }
            finally { keepingOffsets = false; }
        }

        private void UpdateItemsInset()
        {
            if (positionedItems == null || positionedScroll == null) return;
            // Do not mutate the carousel's margin during native type-ahead:
            // it is used while WinUI locates the selected element. Cancel the
            // visual inset at its surrounding scrollport, giving the final row
            // its full height without changing the carousel's native contract.
            positionedScroll.Margin = new Thickness(0, -positionedItems.Margin.Top, 0, -positionedItems.Margin.Bottom);
        }

        private void DetachPopup()
        {
            if (positionedItems != null)
                positionedItems.UnregisterPropertyChangedCallback(FrameworkElement.MarginProperty, itemsMarginToken);
            positionedItems = null;
            positionedScroll = null;
            if (positionedPopup == null) return;
            positionedPopup.UnregisterPropertyChangedCallback(Popup.VerticalOffsetProperty, verticalOffsetToken);
            positionedPopup.UnregisterPropertyChangedCallback(Popup.HorizontalOffsetProperty, horizontalOffsetToken);
            positionedPopup = null;
        }

        private static T? FindType<T>(DependencyObject parent) where T : DependencyObject
        {
            if (parent is T match) return match;
            for (var i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++)
                if (FindType<T>(VisualTreeHelper.GetChild(parent, i)) is { } found) return found;
            return null;
        }

        private static DependencyObject? Find(DependencyObject parent, string name)
        {
            if (parent is FrameworkElement element && element.Name == name) return element;
            for (var i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++)
                if (Find(VisualTreeHelper.GetChild(parent, i), name) is { } found) return found;
            return null;
        }
    }
}
