use gpui::Font;
use gpui_ghostty_terminal::{default_terminal_font, default_terminal_font_features};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FontFamily {
    #[default]
    Menlo,
    Monaco,
    Courier,
    #[serde(rename = "jetbrains_mono")]
    JetBrainsMono,
    #[serde(rename = "jetbrains_mono_nerd_font_mono")]
    JetBrainsMonoNerdFontMono,
}

impl FontFamily {
    pub const ALL: [Self; 5] = [
        Self::Menlo,
        Self::Monaco,
        Self::Courier,
        Self::JetBrainsMono,
        Self::JetBrainsMonoNerdFontMono,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::JetBrainsMonoNerdFontMono => "JetBrains Mono Nerd Font Mono",
            _ => self.family_name(),
        }
    }

    pub fn family_name(self) -> &'static str {
        match self {
            Self::Menlo => "Menlo",
            Self::Monaco => "Monaco",
            Self::Courier => "Courier",
            Self::JetBrainsMono => "JetBrains Mono",
            Self::JetBrainsMonoNerdFontMono => "JetBrainsMono Nerd Font Mono",
        }
    }

    pub fn font(self) -> Font {
        // Keep the terminal's Nerd Font, CJK, and emoji fallback chain for every family.
        let mut font = default_terminal_font();
        font.family = self.family_name().into();
        font.features = default_terminal_font_features();
        font
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Typography {
    pub font_family: FontFamily,
}

impl Typography {
    pub fn config_path() -> io::Result<PathBuf> {
        Ok(crate::appearance::config_directory()?.join("typography.json"))
    }

    pub fn load() -> (Self, Option<String>) {
        match Self::config_path() {
            Ok(path) => Self::load_from(&path),
            Err(error) => (Self::default(), Some(error.to_string())),
        }
    }

    fn load_from(path: &Path) -> (Self, Option<String>) {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return (Self::default(), None);
            }
            Err(error) => {
                return (
                    Self::default(),
                    Some(format!("Could not read {}: {error}", path.display())),
                );
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(typography) => (typography, None),
            Err(error) => (
                Self::default(),
                Some(format!("Could not decode {}: {error}", path.display())),
            ),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path().map_err(|error| error.to_string())?;
        self.save_to(&path)
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        let mut bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| format!("Could not encode typography: {error}"))?;
        bytes.push(b'\n');
        crate::appearance::write_atomic(path, &bytes)
            .map_err(|error| format!("Could not save {}: {error}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "monoterm-typography-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                eprintln!(
                    "Could not remove test directory {}: {error}",
                    self.0.display()
                );
            }
        }
    }

    #[test]
    fn font_preferences_default_to_menlo_and_round_trip() {
        assert_eq!(Typography::default().font_family, FontFamily::Menlo);
        assert_eq!(
            serde_json::from_str::<Typography>("{}").unwrap(),
            Typography::default()
        );
        for font_family in FontFamily::ALL {
            let typography = Typography { font_family };
            let json = serde_json::to_string(&typography).unwrap();
            assert_eq!(
                serde_json::from_str::<Typography>(&json).unwrap(),
                typography
            );
        }
        let typography: Typography =
            serde_json::from_str(r#"{"font_family":"jetbrains_mono_nerd_font_mono"}"#).unwrap();
        assert_eq!(
            typography.font_family,
            FontFamily::JetBrainsMonoNerdFontMono
        );
        for json in [
            r#"{"font_family":"unknown"}"#,
            r#"{"font_family":null}"#,
            "{",
        ] {
            assert!(serde_json::from_str::<Typography>(json).is_err());
        }
    }

    #[test]
    fn every_family_keeps_nerd_font_fallbacks_and_terminal_features() {
        let default_font = default_terminal_font();
        for family in FontFamily::ALL {
            let font = family.font();
            assert_eq!(font.family.as_str(), family.family_name());
            assert_eq!(font.fallbacks, default_font.fallbacks);
            assert_eq!(font.features, default_terminal_font_features());
            assert!(font.fallbacks.as_ref().unwrap().fallback_list().iter().any(
                |fallback| fallback == FontFamily::JetBrainsMonoNerdFontMono.family_name()
            ));
            assert!(family.label().is_ascii());
        }
    }

    #[test]
    fn missing_or_invalid_files_return_safe_defaults_without_rewriting() {
        let directory = TestDirectory::new();
        let path = directory.0.join("typography.json");
        assert_eq!(Typography::load_from(&path), (Typography::default(), None));
        assert!(!path.exists());
        let invalid = br#"{"font_family":"unknown"}"#;
        fs::write(&path, invalid).unwrap();
        let (typography, diagnostic) = Typography::load_from(&path);
        assert_eq!(typography, Typography::default());
        assert!(diagnostic.unwrap().contains("Could not decode"));
        assert_eq!(fs::read(&path).unwrap(), invalid);
        let (_, diagnostic) = Typography::load_from(&directory.0);
        assert!(diagnostic.unwrap().contains("Could not read"));
    }

    #[test]
    fn saving_replaces_preferences_atomically_and_reports_failures() {
        let directory = TestDirectory::new();
        let path = directory.0.join("preferences").join("typography.json");
        let first = Typography {
            font_family: FontFamily::Monaco,
        };
        first.save_to(&path).unwrap();
        assert_eq!(Typography::load_from(&path), (first, None));
        let second = Typography {
            font_family: FontFamily::JetBrainsMonoNerdFontMono,
        };
        second.save_to(&path).unwrap();
        assert_eq!(Typography::load_from(&path), (second, None));
        assert!(fs::read(&path).unwrap().ends_with(b"\n"));
        let entries = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert!(
            second
                .save_to(&directory.0)
                .unwrap_err()
                .contains("Could not save")
        );
    }
}
