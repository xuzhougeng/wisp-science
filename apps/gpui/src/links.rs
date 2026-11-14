//! The places outside the app it can send you to. They open in the system's
//! browser, never inside the app, and are the WebView desktop's.

/// The tutorials.
pub const DOCS: &str = "https://wispscience.com/tutorials.html";
/// Where problems are reported and can be searched first.
pub const ISSUES: &str = "https://github.com/xuzhougeng/wisp-science/issues";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_secure_and_carry_nothing_about_the_user() {
        for link in [DOCS, ISSUES] {
            assert!(link.starts_with("https://"), "{link}");
            // Nothing is appended at run time, so nothing can leak.
            assert!(!link.contains('?') && !link.contains('#'), "{link}");
        }
        assert!(DOCS.starts_with("https://wispscience.com/"));
        assert!(ISSUES.starts_with("https://github.com/xuzhougeng/wisp-science/"));
    }
}
