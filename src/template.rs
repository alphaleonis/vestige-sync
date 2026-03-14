use std::error::Error;
use std::fmt;

const KNOWN_PLACEHOLDERS: &[&str] = &["hostname", "os", "user", "platform", "distro"];

#[derive(Debug)]
pub struct UnknownPlaceholder {
    pub name: String,
}

impl fmt::Display for UnknownPlaceholder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown template placeholder '{{{}}}'. Valid placeholders: {}",
            self.name,
            KNOWN_PLACEHOLDERS
                .iter()
                .map(|p| format!("{{{}}}", p))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl Error for UnknownPlaceholder {}

/// Detect if running under WSL by checking /proc/version for "microsoft".
fn is_wsl() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/version")
            .map(|v| v.to_lowercase().contains("microsoft"))
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Get the platform: like `os` but returns "wsl" instead of "linux" when under WSL.
fn get_platform() -> &'static str {
    match std::env::consts::OS {
        "linux" if is_wsl() => "wsl",
        other => other,
    }
}

/// Get the distro ID from /etc/os-release (e.g. "fedora", "ubuntu", "arch").
/// Falls back to the OS name on non-Linux or if parsing fails.
fn get_distro() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(contents) = std::fs::read_to_string("/etc/os-release") {
            for line in contents.lines() {
                if let Some(id) = line.strip_prefix("ID=") {
                    return id.trim_matches('"').to_string();
                }
            }
        }
    }
    std::env::consts::OS.to_string()
}

/// Expand template placeholders in a filename string.
///
/// Supported placeholders: `{hostname}`, `{os}`, `{user}`, `{platform}`, `{distro}`.
/// Returns an error if unknown `{...}` placeholders are found.
pub fn expand_filename(template: &str) -> Result<String, UnknownPlaceholder> {
    // First pass: check for unknown placeholders
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let after_brace = &rest[start + 1..];
        if let Some(end) = after_brace.find('}') {
            let name = &after_brace[..end];
            if !name.is_empty() && !KNOWN_PLACEHOLDERS.contains(&name) {
                return Err(UnknownPlaceholder {
                    name: name.to_string(),
                });
            }
            rest = &after_brace[end + 1..];
        } else {
            break;
        }
    }

    // Second pass: expand known placeholders
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());

    let result = template
        .replace("{hostname}", &hostname)
        .replace("{os}", std::env::consts::OS)
        .replace("{platform}", get_platform())
        .replace("{distro}", &get_distro())
        .replace(
            "{user}",
            &whoami::username().unwrap_or_else(|_| "unknown".to_string()),
        );

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_string_unchanged() {
        assert_eq!(expand_filename("my-desktop").unwrap(), "my-desktop");
    }

    #[test]
    fn expands_os() {
        let result = expand_filename("{os}").unwrap();
        assert_eq!(result, std::env::consts::OS);
    }

    #[test]
    fn expands_user() {
        let result = expand_filename("{user}").unwrap();
        assert_eq!(result, whoami::username().unwrap());
    }

    #[test]
    fn expands_hostname() {
        let result = expand_filename("{hostname}").unwrap();
        let expected = hostname::get().unwrap().to_string_lossy().into_owned();
        assert_eq!(result, expected);
    }

    #[test]
    fn expands_platform() {
        let result = expand_filename("{platform}").unwrap();
        // On native Linux: "linux", on WSL: "wsl", on macOS: "macos", etc.
        assert!(!result.is_empty());
    }

    #[test]
    fn expands_distro() {
        let result = expand_filename("{distro}").unwrap();
        // On Linux: distro ID like "fedora", "ubuntu"
        // On non-Linux: falls back to OS name
        assert!(!result.is_empty());
    }

    #[test]
    fn expands_multiple() {
        let result = expand_filename("{hostname}-{os}").unwrap();
        let expected = format!(
            "{}-{}",
            hostname::get().unwrap().to_string_lossy(),
            std::env::consts::OS
        );
        assert_eq!(result, expected);
    }

    #[test]
    fn unknown_placeholder_errors() {
        let err = expand_filename("{foo}").unwrap_err();
        assert_eq!(err.name, "foo");
        assert!(err.to_string().contains("unknown template placeholder"));
    }

    #[test]
    fn mixed_known_and_unknown_errors() {
        let err = expand_filename("{hostname}-{bogus}").unwrap_err();
        assert_eq!(err.name, "bogus");
    }

    #[test]
    fn bare_braces_ignored() {
        // Unmatched braces or empty {} are left alone
        assert_eq!(expand_filename("hello{world").unwrap(), "hello{world");
        assert_eq!(expand_filename("hello{}world").unwrap(), "hello{}world");
    }
}
