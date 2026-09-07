//! Proxy environment for locally launched code. Never mutate the host environment.

use std::sync::RwLock;

static COMMAND_PROXY: RwLock<String> = RwLock::new(String::new());

pub fn set_command_proxy(proxy: &str) {
    *COMMAND_PROXY.write().unwrap() = proxy.trim().to_owned();
}

pub fn command_proxy_env() -> Vec<(String, String)> {
    proxy_env(&COMMAND_PROXY.read().unwrap())
}

/// Empty inherits the child environment; `none` disables proxy discovery.
/// Set both cases because curl and Python libraries differ on precedence.
pub fn proxy_env(proxy: &str) -> Vec<(String, String)> {
    let proxy = proxy.trim();
    if proxy.is_empty() {
        return Vec::new();
    }
    let direct = proxy == "none";
    let value = if direct { "" } else { proxy };
    let mut envs = [
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
    ]
    .into_iter()
    .map(|key| (key.to_owned(), value.to_owned()))
    .collect::<Vec<_>>();
    for key in ["no_proxy", "NO_PROXY"] {
        envs.push((key.to_owned(), if direct { "*" } else { "" }.to_owned()));
    }
    envs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_environment_overrides_inherited_proxy_without_changing_parent() {
        let mut command = std::process::Command::new("unused");
        command.env("HTTPS_PROXY", "http://stale.invalid:8080");
        command.env("no_proxy", "*");
        command.envs(proxy_env("http://localhost:7890"));
        let envs = command
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.unwrap().to_string_lossy().into_owned(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(envs["HTTPS_PROXY"], "http://localhost:7890");
        assert_eq!(envs["https_proxy"], "http://localhost:7890");
        assert_eq!(envs["no_proxy"], "");
        command.envs(proxy_env("none"));
        assert!(command
            .get_envs()
            .any(|(k, v)| k == "NO_PROXY" && v == Some(std::ffi::OsStr::new("*"))));
    }

    #[test]
    fn inherit_direct_and_explicit_proxy_override_both_cases() {
        assert!(proxy_env("  ").is_empty());
        for (key, value) in proxy_env("none") {
            assert_eq!(
                value,
                if key.eq_ignore_ascii_case("no_proxy") {
                    "*"
                } else {
                    ""
                }
            );
        }
        let overrides = proxy_env(" http://127.0.0.1:7890 ");
        assert_eq!(overrides.len(), 8);
        for (key, value) in overrides {
            assert_eq!(
                value,
                if key.eq_ignore_ascii_case("no_proxy") {
                    ""
                } else {
                    "http://127.0.0.1:7890"
                }
            );
        }
    }
}
