using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeJourneyConversationPage : NativeActionPage
{
    public NativeJourneyConversationPage(INativeConversationClient client, string project, string session, WispDesign design, Action close)
        : base(design, "来源会话 · 只读", new WorkspaceActionModel(), close)
    {
        long? before = null;
        var older = new Button { Content = "更早的消息", Visibility = Visibility.Collapsed };
        older.Click += async (_, _) => await Read();
        Form.Children.Add(older);
        Form.Children.Add(Button("重新读取最新消息", async () => { before = null; await Read(); }));
        async Task Read()
        {
            await State.RunAsync(() => client.SnapshotAsync(project, session, before), snapshot =>
            {
                before = snapshot.NextBeforeSeq; older.Visibility = before == null ? Visibility.Collapsed : Visibility.Visible;
                Results.Children.Clear();
                foreach (var row in snapshot.Items) Results.Children.Add(TranscriptView.Create(new BrowserMessage(0, row.Role, row.Text, row.ToolName), design));
                if (snapshot.Items.Length == 0) Results.Children.Add(Mute("没有可显示的消息。"));
            });
        }
        _ = Read();
    }
}
