namespace Wisp.ProjectBrowser;

// Template text and defaults match ui/src/mandates.rs.
public static class NativeMandateTemplates
{
    public static readonly string[] Icons = ["book", "doc", "server", "gauge"];
    public static NativeMandateDraft Create(int index, bool chinese, DateTimeOffset now, TimeZoneInfo zone)
    {
        var draft = index switch {
            0 => new NativeMandateDraft {
                Name = chinese ? "文献追踪" : "Literature watch",
                Goal = chinese ? "未来 12 个月，持续跟进本项目相关文献：发现与研究问题相关的新工作，筛选并入库，告诉我哪些会影响我们的计划。每周向我汇报。" : "For the next 12 months, keep this project's literature current: find newly published work that bears on its research question, screen it, file what matters, and tell me what changes our plans. Report weekly.",
                Autonomous = chinese ? "检索文献数据库、阅读论文、把文献和笔记保存到项目。" : "Search literature databases, read papers, and save references and notes to the project.",
                ReviewFirst = chinese ? "因某篇文献而修改项目的分析计划或结论。" : "Changing the project's analysis plan or conclusions because of a paper.",
                AskForHelp = chinese ? "文献需要机构登录才能获取，或有发现与我们的方案冲突、需要你判断。" : "A paper needs an institutional login, or a finding contradicts our approach and needs your judgement.",
                Kpis = [
                    new() { Name = chinese ? "筛选并入库的文献" : "Papers screened and filed", Definition = chinese ? "至少读过摘要、判断相关、保存到项目并附一句理由的文献数" : "Read at abstract level or deeper, judged relevant, and saved to the project with a one-line reason", Target = chinese ? "5" : "5", Period = chinese ? "每周" : "per week" },
                ],
                ReviewMutations = false,
                Hours = 24,
                ReportDays = 7,
                Until = TimeZoneInfo.ConvertTime(now.AddDays(365), zone).ToString("yyyy-MM-dd") },
            1 => new NativeMandateDraft {
                Name = chinese ? "投稿与返修跟进" : "Submission follow-up",
                Goal = chinese ? "跟进这篇稿件从投稿到接收：跟踪期刊状态，审稿意见返回后准备逐条回复和返修清单，盯住每一个截止日期。每周汇报，状态变化时立即汇报。" : "Follow this manuscript from submission to acceptance: track the journal's status, prepare point-by-point responses and a revision checklist when reviews arrive, and keep every deadline in view. Report weekly and whenever the status changes.",
                Autonomous = chinese ? "阅读稿件和审稿意见，起草回复与清单，查看期刊状态页。" : "Read the manuscript and reviews, draft responses and checklists, and check the journal's status page.",
                ReviewFirst = chinese ? "发给编辑或审稿人的任何文字，以及对稿件的任何修改。" : "Any text sent to editors or reviewers, and any change to the manuscript.",
                AskForHelp = chinese ? "登录投稿系统、确认作者身份或付款，或决定如何回应有争议的意见。" : "Logging in to the submission system, confirming authorship or payment, or deciding how to answer a contested point.",
                Kpis = [
                    new() { Name = chinese ? "未回应的审稿意见" : "Open reviewer points", Definition = chinese ? "尚未起草回复的审稿意见条数" : "Reviewer comments that have no drafted response yet", Target = chinese ? "0" : "0", Period = chinese ? "返修截止前" : "before the revision deadline" },
                    new() { Name = chinese ? "返修清单完成度（%）" : "Revision checklist done (%)", Definition = chinese ? "已完成并对照稿件核对过的清单条目占比" : "Checklist items finished and checked against the manuscript", Target = chinese ? "100" : "100", Period = chinese ? "返修截止前" : "before the revision deadline" },
                ],
                ReviewMutations = true,
                Hours = 72,
                ReportDays = 7,
                Until = TimeZoneInfo.ConvertTime(now.AddDays(180), zone).ToString("yyyy-MM-dd") },
            2 => new NativeMandateDraft {
                Name = chinese ? "长期计算任务" : "Long-running compute",
                Goal = chinese ? "把这批计算跑到得到经过核验的结果：提交并监控运行，诊断并重提失败任务，取回产出，确认通过验收检查。运行期间每天汇报。" : "See this computation through to verified results: submit and monitor the runs, diagnose and resubmit failures, harvest the outputs, and confirm they pass the acceptance checks. Report daily while runs are active.",
                Autonomous = chinese ? "在已配置的服务器上提交、监控、取消和重提运行；取回产出。" : "Submit, monitor, cancel and resubmit runs on the configured servers; harvest outputs.",
                ReviewFirst = chinese ? "修改分析参数、删除数据，或使用超出约定的算力。" : "Changing analysis parameters, deleting data, or using more compute than agreed.",
                AskForHelp = chinese ? "服务器需要凭据或配额，或同一失败在两种修复后仍然重复。" : "A server needs credentials or quota, or a failure repeats after two different fixes.",
                Kpis = [
                    new() { Name = chinese ? "完成的样本数" : "Samples completed", Definition = chinese ? "运行成功、产出已取回并核验的样本数" : "Samples whose run succeeded and whose outputs were harvested and checked", Target = chinese ? "12" : "12", Period = chinese ? "截止日期前" : "by the end date" },
                ],
                ReviewMutations = false,
                Hours = 6,
                ReportDays = 1,
                Until = TimeZoneInfo.ConvertTime(now.AddDays(30), zone).ToString("yyyy-MM-dd") },
            3 => new NativeMandateDraft {
                Name = chinese ? "方法指标迭代" : "Method metric iteration",
                Goal = chinese ? "未来 3 个月，围绕基准持续改进这个方法：测定基线，找出限制因素，每次只改一处，只保留在留出数据上站得住的改动。每周汇报当前指标。" : "Over the next 3 months, improve this method against its benchmark: measure the baseline, find what limits it, try one change at a time, and keep only changes that hold up on held-out data. Report weekly with the current metric.",
                Autonomous = chinese ? "运行评测和实验，分析结果，准备可供审阅的代码改动。" : "Run evaluations and experiments, analyze results, and prepare code changes for review.",
                ReviewFirst = chinese ? "合并代码，或修改基准与评测脚本。" : "Merging code, or changing the benchmark or the evaluation script.",
                AskForHelp = chinese ? "指标连续三轮停滞，或结果好得无法解释。" : "The metric stalls for three rounds, or a result looks too good to explain.",
                Kpis = [
                    new() { Name = chinese ? "基准指标" : "Benchmark metric", Definition = chinese ? "用固定评测脚本在留出基准上测得的主要指标" : "The primary metric on the held-out benchmark, measured with the fixed evaluation script", Target = chinese ? "0.90" : "0.90", Period = chinese ? "截止日期前" : "by the end date" },
                ],
                ReviewMutations = true,
                Hours = 24,
                ReportDays = 7,
                Until = TimeZoneInfo.ConvertTime(now.AddDays(90), zone).ToString("yyyy-MM-dd") },
            _ => throw new ArgumentOutOfRangeException(nameof(index))
        };
        return draft;
    }
}
