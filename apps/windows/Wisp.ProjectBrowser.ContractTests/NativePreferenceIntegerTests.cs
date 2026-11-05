using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;

internal static class NativePreferenceIntegerTests
{
    public static void Run()
    {
        static void Check(bool passed, string message) { if (!passed) throw new InvalidOperationException(message); }
        var settings = JsonNode.Parse("""{"max_iter":100,"auto_continue_limit":10,"auto_compact_percent":80,"semantic_compact_idle_hours":24}""")!.AsObject();
        var threshold = new SessionNumberDraft(settings, "auto_compact_percent");
        foreach (var text in new[] { "10", " 24 ", "95" }) { threshold.Edit(text); Check(!threshold.Invalid, "inclusive compaction limits"); }
        foreach (var text in new[] { "", " ", "bad", "80.5", "1e2", "1,000", "-1", "9", "96" })
        {
            threshold.Edit(text);
            Check(threshold.Invalid && threshold.Text == text && settings["auto_compact_percent"]!.GetValue<ulong>() == 95, "invalid text stays editable and out of the payload");
        }
        threshold.SetEnabled(false); threshold.Edit("33");
        Check(!threshold.Invalid && threshold.Text == "95" && settings["auto_compact_percent"]!.GetValue<ulong>() == 95, "turning compaction off restores the last valid draft and does not block Save");
        threshold.SetEnabled(true); threshold.Edit("42");
        Check(!threshold.Invalid && settings["auto_compact_percent"]!.GetValue<ulong>() == 42, "the re-enabled field can be edited again");
        var iterations = new SessionNumberDraft(settings, "max_iter");
        iterations.Edit("9223372036854775807"); iterations.Edit("9223372036854775808");
        Check(iterations.Invalid && settings["max_iter"]!.GetValue<long>() == long.MaxValue, "iteration limit uses the host's signed 64-bit range");
        iterations.Edit("0"); Check(!iterations.Invalid && settings["max_iter"]!.GetValue<long>() == 0, "zero iterations means unlimited");
        foreach (var name in new[] { "auto_continue_limit", "semantic_compact_idle_hours" })
        {
            var field = new SessionNumberDraft(settings, name); field.Edit("18446744073709551615"); field.Edit("18446744073709551616");
            Check(field.Invalid && JsonNode.Parse(settings.ToJsonString())![name]!.GetValue<ulong>() == ulong.MaxValue, "unsigned values survive JSON serialization without rounding");
            field.Edit("0"); Check(field.Invalid == (name == "auto_continue_limit"), "only the continuation count excludes zero");
        }
        Check(NativePreferenceInteger.Step("100", 1, 0, int.MaxValue) == "101"
            && NativePreferenceInteger.Step("101", -1, 0, int.MaxValue) == "100", "arrow and pointer stepping use one integer increment");
        Check(NativePreferenceInteger.Step("95", 1, 10, 95) == "95" && NativePreferenceInteger.Step("10", -1, 10, 95) == "10", "step controls respect both inclusive bounds");
        Check(NativePreferenceInteger.Step("80.5", 1, 10, 95) == "81" && NativePreferenceInteger.Step("80.5", -1, 10, 95) == "80", "fractional input steps to the adjacent integer");
        Check(NativePreferenceInteger.Step("", 1, 10, 95) == "10" && NativePreferenceInteger.Step("", -1, 10, 95) == "10", "an empty bounded field can be recovered by stepping");
        Check(NativePreferenceInteger.Step("96", 1, 10, 95) == "96" && NativePreferenceInteger.Step("96", -1, 10, 95) == "95", "an out-of-range edit only moves toward the range");
        Check(NativePreferenceInteger.Step("2147483647", 1, 0, int.MaxValue) == "2147483647"
            && NativePreferenceInteger.Step("79228162514264337593543950335", -1, 0, int.MaxValue) == "2147483647", "large pasted input never overflows during stepping");
        Check(NativePreferenceInteger.StepUnsigned("18446744073709551614", 1, 1, ulong.MaxValue) == "18446744073709551615"
            && NativePreferenceInteger.StepUnsigned("18446744073709551615", 1, 1, ulong.MaxValue) == "18446744073709551615"
            && NativePreferenceInteger.StepUnsigned("18446744073709551616", -1, 1, ulong.MaxValue) == "18446744073709551615", "workflow costs retain the exact host u64 boundary");
        Check(NativePreferenceInteger.StepUnsigned("50", 1, 1, 50) == "50"
            && NativePreferenceInteger.StepUnsigned("", -1, 1, 300) == "1"
            && NativePreferenceInteger.StepUnsigned("604801", -1, 1, 604800) == "604800", "candidate and duration controls respect the activity limits");
        Check(NativePreferenceInteger.StepUnsigned("", 1, 0, uint.MaxValue) == "1"
            && NativePreferenceInteger.StepUnsigned("1", -1, 0, ulong.MaxValue) == "0"
            && NativePreferenceInteger.StepUnsigned("4294967296", -1, 0, uint.MaxValue) == "4294967295",
            "optional Agent budgets start from an empty draft, allow explicit zero and retain u32 limits");
    }
}
