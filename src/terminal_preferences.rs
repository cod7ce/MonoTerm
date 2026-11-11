use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub const DEFAULT_SCROLLBACK_LINES: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct TerminalPreferences {
    pub scrollback_lines: usize,
    pub default_directory: Option<PathBuf>,
}

impl Default for TerminalPreferences {
    fn default() -> Self {
        Self {
            scrollback_lines: DEFAULT_SCROLLBACK_LINES,
            default_directory: None,
        }
    }
}

impl TerminalPreferences {
    pub fn startup_directory(
        &self,
        explicit: Option<&Path>,
    ) -> Result<(PathBuf, Option<String>), String> {
        if let Some(directory) = explicit {
            return existing_directory(directory).map(|directory| (directory, None));
        }
        self.default_directory_or_home(&home_directory()?)
    }

    fn default_directory_or_home(&self, home: &Path) -> Result<(PathBuf, Option<String>), String> {
        match &self.default_directory {
            Some(directory) => match existing_directory(directory) {
                Ok(directory) => Ok((directory, None)),
                Err(_) => Ok((
                    existing_directory(home)?,
                    Some("The default directory is unavailable. Opened the home directory.".into()),
                )),
            },
            None => existing_directory(home).map(|directory| (directory, None)),
        }
    }

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

pub fn existing_directory(directory: &Path) -> Result<PathBuf, String> {
    if !directory.is_absolute() {
        return Err("Enter an absolute directory or a path starting with ~/.".into());
    }
    directory
        .canonicalize()
        .ok()
        .filter(|directory| directory.is_dir())
        .ok_or_else(|| "Choose an existing, accessible directory.".into())
}

fn home_directory() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute() && home.is_dir())
        .ok_or_else(|| "Could not find the home directory.".into())
}

pub fn parse_default_directory(value: &str) -> Result<Option<PathBuf>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let directory = if value == "~" {
        home_directory()?
    } else if let Some(relative) = value.strip_prefix("~/") {
        home_directory()?.join(relative)
    } else {
        PathBuf::from(value)
    };
    existing_directory(&directory).map(Some)
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
            ..Default::default()
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
                    scrollback_lines: 0,
                    ..Default::default()
                },
                None
            )
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn default_directory_is_optional_and_preserves_legacy_preferences() {
        let preferences: TerminalPreferences =
            serde_json::from_str(r#"{"scrollback_lines":2500}"#).unwrap();
        assert_eq!(preferences.default_directory, None);
        assert_eq!(preferences.scrollback_lines, 2500);
        assert_eq!(parse_default_directory("  "), Ok(None));
        assert!(parse_default_directory("relative/path").is_err());
        assert!(parse_default_directory("~someone").is_err());
        assert_eq!(
            parse_default_directory("~/"),
            existing_directory(&home_directory().unwrap()).map(Some)
        );
    }

    #[test]
    fn startup_directory_uses_preference_home_and_explicit_override() {
        let home = std::env::temp_dir().canonicalize().unwrap();
        let configured = home.join(format!("monoterm-directory-{}", std::process::id()));
        fs::create_dir(&configured).unwrap();
        let mut preferences = TerminalPreferences::default();
        assert_eq!(
            preferences.default_directory_or_home(&home),
            Ok((home.clone(), None))
        );
        preferences.default_directory = Some(configured.clone());
        assert_eq!(
            parse_default_directory(configured.to_str().unwrap()),
            Ok(Some(configured.clone()))
        );
        assert_eq!(
            preferences.default_directory_or_home(&home),
            Ok((configured.clone(), None))
        );
        assert_eq!(
            preferences.startup_directory(Some(&home)),
            Ok((home.clone(), None))
        );
        assert_eq!(
            serde_json::from_slice::<TerminalPreferences>(
                &serde_json::to_vec(&preferences).unwrap()
            )
            .unwrap(),
            preferences
        );
        fs::remove_dir(&configured).unwrap();
        let (directory, notice) = preferences.default_directory_or_home(&home).unwrap();
        assert_eq!(directory, home);
        assert!(notice.is_some());
        assert!(preferences.startup_directory(Some(&configured)).is_err());
        let file = configured.with_extension("txt");
        fs::write(&file, b"not a directory").unwrap();
        assert!(parse_default_directory(file.to_str().unwrap()).is_err());
        fs::remove_file(file).unwrap();
    }
}
