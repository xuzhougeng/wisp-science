using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private string repository = "", gitRef = "main";
    private void SkillStore()
    {
        var card = Card("社区技能与 GitHub 导入");
        var catalog = new StackPanel { Spacing = 10 };
        var candidates = new StackPanel { Spacing = 12 };
        var source = new TextBox { Header = "GitHub 仓库 URL", Text = repository };
        var revision = new TextBox { Header = "分支、Tag 或 Commit", Text = gitRef };
        source.TextChanged += (_, _) => { repository = source.Text; candidates.Children.Clear(); };
        revision.TextChanged += (_, _) => { gitRef = revision.Text; candidates.Children.Clear(); };
        async Task Catalog(bool refresh)
        {
            JsonNode? response = null;
            if (!await model.InvokeAsync("list_community_skills", new() { ["refresh"] = refresh }, v => response = v)) return;
            catalog.Children.Clear();
            foreach (var item in Rows(response?["entries"]))
            {
                var entry = new StackPanel { Spacing = 6 };
                entry.Children.Add(Mute(S(item, "name") + " · " + S(item, "description")));
                if (S(item, "known_limits").Length > 0) entry.Children.Add(Warn(S(item, "known_limits")));
                entry.Children.Add(Button("选择此来源", () =>
                { source.Text = "https://github.com/" + S(item, "repository"); revision.Text = S(item, "git_ref"); return Task.CompletedTask; }));
                catalog.Children.Add(entry);
            }
        }
        card.Children.Add(Button("浏览社区目录", () => Catalog(false))); card.Children.Add(Button("更新社区目录", () => Catalog(true)));
        card.Children.Add(catalog); card.Children.Add(source); card.Children.Add(revision);
        card.Children.Add(Button("预览可安装技能", async () =>
        {
            var requestedSource = repository; var requestedRef = gitRef;
            JsonNode? response = null;
            if (!await model.InvokeAsync("preview_github_skills", new() { ["sourceUrl"] = requestedSource, ["exactRef"] = requestedRef }, v => response = v)
                || repository != requestedSource || gitRef != requestedRef) return;
            candidates.Children.Clear();
            foreach (var item in Rows(response))
            {
                var entry = new StackPanel { Spacing = 8 };
                entry.Children.Add(Design.Text(S(item, "name"), 18)); entry.Children.Add(Mute(S(item, "description")));
                entry.Children.Add(Mute("Commit: " + S(item["source"], "commit")));
                foreach (var key in new[] { "warnings", "format_errors", "resource_errors" })
                    foreach (var warning in item[key] as JsonArray ?? []) entry.Children.Add(Warn(warning?.GetValue<string>() ?? ""));
                var conflict = S(item, "conflict"); if (conflict.Length > 0) entry.Children.Add(Warn(conflict));
                entry.Children.Add(new TextBox { Header = "技能说明", Text = S(item, "markdown"), IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 300 });
                var install = Button("安装此版本", async () =>
                {
                    if (requestedSource != repository || requestedRef != gitRef) { model.Fail("来源已变更，请重新预览。"); return; }
                    await Run("install_github_skill", new() { ["source"] = item["source"]?.DeepClone() });
                });
                install.IsEnabled = conflict.Length == 0 && (item["format_errors"] as JsonArray)?.Count is not > 0
                    && (item["resource_errors"] as JsonArray)?.Count is not > 0 && item["source"] is JsonObject;
                entry.Children.Add(install); Design.ApplyTypography(entry); candidates.Children.Add(entry);
            }
            if (candidates.Children.Count == 0) candidates.Children.Add(Mute("未找到可安装的技能。"));
        }));
        card.Children.Add(candidates);
    }
}
