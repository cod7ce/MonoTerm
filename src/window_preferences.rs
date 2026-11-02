//! Window layout preferences; terminal sessions remain daemon- or process-owned.
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io, path::Path};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SidebarView {
    Spaces,
    Agents,
    #[default]
    Both,
}

impl SidebarView {
    pub(crate) fn spaces(self) -> bool {
        self != Self::Agents
    }
    pub(crate) fn agents(self) -> bool {
        self != Self::Spaces
    }
    pub(crate) fn both(self) -> bool {
        self == Self::Both
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub(crate) struct MainPreferences {
    pub visible: bool,
    pub width: f32,
    pub zen: bool,
}

impl Default for MainPreferences {
    fn default() -> Self {
        Self {
            visible: true,
            width: crate::window_chrome::SIDEBAR_WIDTH,
            zen: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub(crate) struct HerdrPreferences {
    pub visible: bool,
    pub width: f32,
    pub ratio: f32,
    pub view: SidebarView,
    pub collapsed_repos: HashSet<String>,
}

impl Default for HerdrPreferences {
    fn default() -> Self {
        Self {
            visible: true,
            width: crate::window_chrome::SIDEBAR_WIDTH,
            ratio: 0.5,
            view: SidebarView::Both,
            collapsed_repos: HashSet::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub(crate) struct WindowPreferences {
    pub main: MainPreferences,
    pub herdr: HerdrPreferences,
}

impl WindowPreferences {
    fn clamp(&mut self) {
        fn bounded(value: f32, min: f32, max: f32, default: f32) -> f32 {
            if value.is_finite() {
                value.clamp(min, max)
            } else {
                default
            }
        }
        self.main.width = bounded(
            self.main.width,
            180.0,
            480.0,
            crate::window_chrome::SIDEBAR_WIDTH,
        );
        self.herdr.width = bounded(
            self.herdr.width,
            180.0,
            480.0,
            crate::window_chrome::SIDEBAR_WIDTH,
        );
        // Pixel minimums depend on the viewport and are applied when rendering.
        self.herdr.ratio = bounded(self.herdr.ratio, 0.0, 1.0, 0.5);
    }

    pub fn load() -> (Self, Option<String>) {
        match crate::appearance::config_directory() {
            Ok(directory) => Self::load_from(&directory.join("windows.json")),
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
        match serde_json::from_slice::<Self>(&bytes) {
            Ok(mut preferences) => {
                preferences.clamp();
                (preferences, None)
            }
            Err(error) => (
                Self::default(),
                Some(format!("Could not decode {}: {error}", path.display())),
            ),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = crate::appearance::config_directory()
            .map_err(|error| error.to_string())?
            .join("windows.json");
        self.save_to(&path)
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        let mut preferences = self.clone();
        preferences.clamp();
        let mut bytes = serde_json::to_vec_pretty(&preferences)
            .map_err(|error| format!("Could not encode window preferences: {error}"))?;
        bytes.push(b'\n');
        crate::appearance::write_atomic(path, &bytes)
            .map_err(|error| format!("Could not save {}: {error}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_preferences_fill_missing_fields_and_bound_geometry() {
        let mut preferences: WindowPreferences =
            serde_json::from_str(r#"{"main":{"visible":false,"width":900},"herdr":{"ratio":-4}}"#)
                .unwrap();
        preferences.clamp();
        assert!(!preferences.main.visible);
        assert_eq!(preferences.main.width, 480.0);
        assert!(preferences.herdr.visible);
        assert_eq!(preferences.herdr.ratio, 0.0);
        assert_eq!(preferences.herdr.view, SidebarView::Both);
    }

    #[test]
    fn window_layouts_and_repository_identities_survive_reload() {
        let directory = std::env::temp_dir().join(format!(
            "monoterm-window-prefs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("windows.json");
        let mut preferences = WindowPreferences::default();
        preferences.main.visible = false;
        preferences.main.zen = true;
        preferences.herdr.width = 320.0;
        preferences.herdr.ratio = 0.08;
        preferences.herdr.view = SidebarView::Agents;
        preferences.herdr.collapsed_repos.insert("/repo/one".into());
        preferences.save_to(&path).unwrap();
        let (loaded, notice) = WindowPreferences::load_from(&path);
        assert!(notice.is_none());
        assert_eq!(loaded, preferences);
        fs::write(&path, b"{broken").unwrap();
        assert!(WindowPreferences::load_from(&path).1.is_some());
        fs::remove_dir_all(directory).unwrap();
    }
}
