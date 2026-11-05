using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Wisp.Science.Preview;

internal static class PreferenceCheckBox
{
    public static CheckBox Create(WispDesign design, string label, bool selected, double size)
    {
        var check = new CheckBox { IsChecked = selected, MinWidth = 0, MinHeight = 0, Padding = new Thickness(8, 0, 0, 0), CornerRadius = new CornerRadius(2) };
        // WinUI's stock CheckBoxSize is a StaticResource, so per-control resources
        // cannot resize it. Keep the native CheckBox behavior with a compact visual.
        check.Tag = design.Icon("check", 10, design.ColorText("on-clay"));
        check.Template = (ControlTemplate)Microsoft.UI.Xaml.Markup.XamlReader.Load($$"""
            <ControlTemplate xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" TargetType="CheckBox">
              <Grid Background="Transparent">
                <VisualStateManager.VisualStateGroups>
                  <VisualStateGroup x:Name="CheckStates">
                    <VisualState x:Name="Unchecked" />
                    <VisualState x:Name="Checked"><VisualState.Setters>
                      <Setter Target="CheckSurface.Background" Value="{ThemeResource CheckBoxCheckBackgroundFillChecked}" />
                      <Setter Target="CheckSurface.BorderBrush" Value="{ThemeResource CheckBoxCheckBackgroundStrokeChecked}" />
                      <Setter Target="Mark.Visibility" Value="Visible" />
                    </VisualState.Setters></VisualState>
                  </VisualStateGroup>
                  <VisualStateGroup x:Name="CommonStates">
                    <VisualState x:Name="Normal" /><VisualState x:Name="PointerOver" /><VisualState x:Name="Pressed" />
                    <VisualState x:Name="Disabled"><VisualState.Setters><Setter Target="Layout.Opacity" Value="0.42" /></VisualState.Setters></VisualState>
                  </VisualStateGroup>
                </VisualStateManager.VisualStateGroups>
                <Grid x:Name="Layout">
                  <Grid.ColumnDefinitions><ColumnDefinition Width="Auto" /><ColumnDefinition Width="*" /></Grid.ColumnDefinitions>
                  <Border x:Name="CheckSurface" Width="{{size.ToString(System.Globalization.CultureInfo.InvariantCulture)}}" Height="{{size.ToString(System.Globalization.CultureInfo.InvariantCulture)}}" VerticalAlignment="Center" CornerRadius="{TemplateBinding CornerRadius}" BorderThickness="1" BorderBrush="{ThemeResource CheckBoxCheckBackgroundStrokeUnchecked}" Background="{ThemeResource CheckBoxCheckBackgroundFillUnchecked}">
                    <ContentPresenter x:Name="Mark" Content="{TemplateBinding Tag}" Visibility="Collapsed" HorizontalAlignment="Center" VerticalAlignment="Center" />
                  </Border>
                  <ContentPresenter Grid.Column="1" Content="{TemplateBinding Content}" Margin="{TemplateBinding Padding}" HorizontalAlignment="{TemplateBinding HorizontalContentAlignment}" VerticalAlignment="Center" />
                </Grid>
              </Grid>
            </ControlTemplate>
            """);
        foreach (var state in new[] { "Checked", "CheckedPointerOver", "CheckedPressed" })
        {
            check.Resources["CheckBoxCheckBackgroundFill" + state] = design.Brush("clay"); check.Resources["CheckBoxCheckBackgroundStroke" + state] = design.Brush("clay");
        }
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(check, label); return check;
    }
}
