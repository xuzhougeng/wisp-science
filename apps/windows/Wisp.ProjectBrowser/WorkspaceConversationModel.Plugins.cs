using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspaceConversationModel
{
    public bool StageSpecialistChat(NativeSpecialistChat launch)
    {
        if (projectId != launch.ProjectId || sessionId != launch.SessionId || !CanAttach || Snapshot?.Running != false
            || VisibleItems.Length > 0 || Draft.Length > 0 || References.Length > 0 || Attachments.Length > 0 || Quotes.Length > 0
            || string.IsNullOrWhiteSpace(launch.Prompt)) return false;
        Draft = launch.Prompt; drafts[launch.SessionId] = Draft; Notify(); return true;
    }

    public bool StagePluginLaunch(NativePluginLaunch launch)
    {
        if (projectId != launch.ProjectId || sessionId != launch.SessionId || !CanAttach || Snapshot?.Running != false
            || VisibleItems.Length > 0 || Draft.Length > 0 || References.Length > 0 || Attachments.Length > 0 || Quotes.Length > 0
            || launch.SkillNames.Length > 64 || launch.SkillNames.Any(string.IsNullOrWhiteSpace)
            || launch.SkillNames.Length > 0 && Snapshot?.ComposerReferences != true) return false;
        Draft = launch.Prompt;
        References = launch.SkillNames.Distinct(StringComparer.Ordinal).Select(name => new NativeReferenceOption(new("skill", Name: name), name, "插件技能")).ToArray();
        drafts[launch.SessionId] = Draft; stagedReferences[launch.SessionId] = References; Notify(); return true;
    }
}
