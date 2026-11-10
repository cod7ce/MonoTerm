use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

pub const DEFAULT_SCROLLBACK_LINES: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct TerminalPreferences {
    pub scrollback_lines: usize,
}

impl Default for TerminalPreferences {
    fn default() -> Self {
        Self {
            scrollback_lines: DEFAULT_SCROLLBACK_LINES,
        }
    }
}

impl TerminalPreferences {
    pub fn load() -> (Self, Option<String>) {
        match crate::appearance::config_directory() {
            Ok(directory) => Self::load_from(&directory.join("terminal.json")),
            Err(error) => (Self::default(), Some(error.to_string())),
        }
    }

    fn load_from(path: &Path) -> (Self, Option<String>) {
        match fs::read(path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(preferences) => (preferences, None),
                Err(error) => (
                    Self::default(),
                    Some(format!("Could not decode {}: {error}", path.display())),
                ),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => (Self::default(), None),
            Err(error) => (
                Self::default(),
                Some(format!("Could not read {}: {error}", path.display())),
            ),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let directory = crate::appearance::config_directory().map_err(|error| error.to_string())?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        crate::appearance::write_atomic(&directory.join("terminal.json"), &bytes)
            .map_err(|error| error.to_string())
    }
}

pub fn parse_scrollback_lines(value: &str) -> Result<usize, &'static str> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Enter a non-negative whole number; 0 means unlimited.");
    }
    value
        .parse()
        .map_err(|_| "Enter a non-negative whole number; 0 means unlimited.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_saved_unlimited_are_distinct() {
        let default: TerminalPreferences = serde_json::from_str("{}").unwrap();
        assert_eq!(default.scrollback_lines, 10_000);
        let unlimited = TerminalPreferences {
            scrollback_lines: 0,
        };
        assert_eq!(
            serde_json::from_slice::<TerminalPreferences>(&serde_json::to_vec(&unlimited).unwrap())
                .unwrap(),
            unlimited
        );
        assert!(serde_json::from_str::<TerminalPreferences>(r#"{"scrollback_lines":-1}"#).is_err());
    }

    #[test]
    fn accepts_only_non_negative_integers_without_overflow() {
        assert_eq!(parse_scrollback_lines(" 10000 "), Ok(10_000));
        assert_eq!(parse_scrollback_lines("0"), Ok(0));
        assert_eq!(parse_scrollback_lines("250000"), Ok(250_000));
        for invalid in [
            "",
            "-1",
            "+1",
            "1.5",
            "1e4",
            "NaN",
            "100000000000000000000000",
        ] {
            assert!(parse_scrollback_lines(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn missing_and_invalid_files_fall_back_to_defaults() {
        let path = std::env::temp_dir().join(format!(
            "monoterm-terminal-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert_eq!(
            TerminalPreferences::load_from(&path),
            (TerminalPreferences::default(), None)
        );
        fs::write(&path, br#"{"scrollback_lines":"invalid"}"#).unwrap();
        let (preferences, notice) = TerminalPreferences::load_from(&path);
        assert_eq!(preferences, TerminalPreferences::default());
        assert!(notice.is_some());
        fs::write(&path, br#"{"scrollback_lines":0}"#).unwrap();
        assert_eq!(
            TerminalPreferences::load_from(&path),
            (
                TerminalPreferences {
                    scrollback_lines: 0
                },
                None
            )
        );
        fs::remove_file(path).unwrap();
    }
}
