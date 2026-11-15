//! The places outside the app it can send you to. They open in the system's
//! browser, never inside the app, and are the WebView desktop's.

/// The tutorials.
pub const DOCS: &str = "https://wispscience.com/tutorials.html";
/// The project's own page, to star it.
pub const REPOSITORY: &str = "https://github.com/xuzhougeng/wisp-science";
/// Where problems are reported and can be searched first.
pub const ISSUES: &str = "https://github.com/xuzhougeng/wisp-science/issues";
/// Where a new problem report is written; the model adds the title and
/// body to it for the researcher to open.
pub const ISSUE_NEW: &str = "https://github.com/xuzhougeng/wisp-science/issues/new";

/// The first message of a conversation that drafts a problem report, as
/// the WebView's Feedback starts one: what is collected about the app
/// (and nothing about the researcher or their work), and how the model is
/// to help.
pub fn issue_report_prompt(version: &str, os: &str, arch: &str, model: &str) -> String {
    crate::i18n::fill(
        crate::i18n::tr(
            "Help me file a GitHub issue for xuzhougeng/wisp-science.\n\n\
             [Collected automatically; do not ask me for API keys, transcripts, project files, environment variables, usernames or absolute paths]\n\
             - Wisp version: {} (GPUI desktop)\n\
             - OS / architecture: {} / {}\n\
             - Model: {}\n\n\
             Guide me in plain language: what happened, steps to reproduce, expected and actual behaviour, and any Run ID or error text I know. \
             When we have enough detail, propose a short issue title and a Markdown body with sections \
             (Problem, Steps to reproduce, Expected, Actual, Environment). \
             Then give a prefilled GitHub link using this base address: {}?title=...&body=... \
             Remind me that screenshots must be attached on GitHub by hand; Wisp does not upload them.",
        ),
        &[&version, &os, &arch, &model, &ISSUE_NEW],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_problem_report_starts_with_what_is_known_about_the_app_only() {
        let prompt = issue_report_prompt("1.25.0", "macos", "aarch64", "GPT-5");
        assert!(prompt.starts_with("Help me file a GitHub issue"));
        assert!(prompt.contains("- Wisp version: 1.25.0 (GPUI desktop)"));
        assert!(prompt.contains("- OS / architecture: macos / aarch64"));
        assert!(prompt.contains("- Model: GPT-5"));
        assert!(prompt.contains(&format!("{ISSUE_NEW}?title=...&body=...")));
        assert!(!prompt.contains("{}"));
    }

    #[test]
    fn links_are_secure_and_carry_nothing_about_the_user() {
        for link in [DOCS, ISSUES, ISSUE_NEW] {
            assert!(link.starts_with("https://"), "{link}");
            // Nothing is appended at run time, so nothing can leak.
            assert!(!link.contains('?') && !link.contains('#'), "{link}");
        }
        assert!(DOCS.starts_with("https://wispscience.com/"));
        assert!(ISSUES.starts_with("https://github.com/xuzhougeng/wisp-science/"));
    }
}
