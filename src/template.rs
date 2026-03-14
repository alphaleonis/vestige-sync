use std::error::Error;
use std::fmt;

const KNOWN_PLACEHOLDERS: &[&str] = &["hostname", "os", "user"];

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

/// Expand template placeholders in a filename string.
///
/// Supported placeholders: `{hostname}`, `{os}`, `{user}`.
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
