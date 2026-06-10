// SPDX-License-Identifier: GPL-3.0-or-later
//! Small runtime environment probes for platform-specific integration fixes.

pub fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .as_deref()
        .is_some_and(env_flag_value_enabled)
}

fn env_flag_value_enabled(value: &str) -> bool {
    let value = value.trim();
    value == "1"
        || value.eq_ignore_ascii_case("true")
        || value.eq_ignore_ascii_case("yes")
        || value.eq_ignore_ascii_case("on")
}

pub fn running_under_wsl() -> bool {
    std::env::var_os("WSL_INTEROP").is_some()
        || std::env::var_os("WSL_DISTRO_NAME").is_some()
        || std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|release| linux_release_indicates_wsl(&release))
            .unwrap_or(false)
}

fn linux_release_indicates_wsl(release: &str) -> bool {
    release.to_ascii_lowercase().contains("microsoft")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_flags_require_truthy_values() {
        for value in ["1", "true", "TRUE", "yes", "on", " On "] {
            assert!(
                env_flag_value_enabled(value),
                "{value:?} should enable a flag"
            );
        }
        for value in ["", "0", "false", "no", "off", "disabled"] {
            assert!(
                !env_flag_value_enabled(value),
                "{value:?} should leave a flag disabled"
            );
        }
    }

    #[test]
    fn linux_release_probe_matches_wsl_kernels() {
        assert!(linux_release_indicates_wsl(
            "5.15.167.4-microsoft-standard-WSL2"
        ));
        assert!(linux_release_indicates_wsl("4.19.128-microsoft-standard"));
        assert!(!linux_release_indicates_wsl("6.8.0-90-generic"));
    }
}
