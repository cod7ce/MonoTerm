use gpui::Font;
use gpui_ghostty_terminal::{default_terminal_font, default_terminal_font_features};
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub const DEFAULT_FONT_SIZE: f32 = 14.0;
pub const MIN_FONT_SIZE: f32 = 10.0;
pub const MAX_FONT_SIZE: f32 = 28.0;
const DEFAULT_FONT_FAMILY: &str = "Menlo";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Typography {
    #[serde(deserialize_with = "deserialize_family")]
    pub font_family: String,
    #[serde(deserialize_with = "deserialize_size")]
    pub font_size: f32,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            font_family: DEFAULT_FONT_FAMILY.into(),
            font_size: DEFAULT_FONT_SIZE,
        }
    }
}

fn normalized_family(family: &str) -> &str {
    match family.trim() {
        "" | "menlo" => DEFAULT_FONT_FAMILY,
        "monaco" => "Monaco",
        "courier" => "Courier",
        "jetbrains_mono" => "JetBrains Mono",
        "jetbrains_mono_nerd_font_mono" => "JetBrainsMono Nerd Font Mono",
        family => family,
    }
}

fn validated_size(size: f32) -> f32 {
    if size.is_finite() {
        size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
    } else {
        DEFAULT_FONT_SIZE
    }
}

fn deserialize_family<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let family = String::deserialize(deserializer)?;
    if family.chars().any(char::is_control) {
        return Err(serde::de::Error::custom(
            "font family contains control characters",
        ));
    }
    Ok(normalized_family(&family).to_owned())
}

fn deserialize_size<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    Ok(validated_size(f32::deserialize(deserializer)?))
}

impl Typography {
    pub fn clamp(&mut self) {
        self.font_size = validated_size(self.font_size);
        self.font_family = if self.font_family.chars().any(char::is_control) {
            DEFAULT_FONT_FAMILY.into()
        } else {
            normalized_family(&self.font_family).to_owned()
        };
    }

    pub fn font(&self) -> Font {
        self.font_from(installed_fonts())
    }

    fn font_from(&self, fonts: &[InstalledFont]) -> Font {
        // Keep the terminal's Nerd Font, CJK, and emoji fallback chain for every family.
        let mut font = default_terminal_font();
        font.family = if has_family(fonts, &self.font_family) {
            self.font_family.clone().into()
        } else {
            DEFAULT_FONT_FAMILY.into()
        };
        font.features = default_terminal_font_features();
        font
    }

    pub fn missing_font_notice(&self) -> Option<String> {
        self.missing_font_notice_from(installed_fonts())
    }

    fn missing_font_notice_from(&self, fonts: &[InstalledFont]) -> Option<String> {
        (!has_family(fonts, &self.font_family)).then(|| {
            format!(
                "Font \"{}\" is not installed. Using Menlo; your chosen font is kept.",
                self.font_family
            )
        })
    }

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
        let mut validated = self.clone();
        validated.clamp();
        let mut bytes = serde_json::to_vec_pretty(&validated)
            .map_err(|error| format!("Could not encode typography: {error}"))?;
        bytes.push(b'\n');
        crate::appearance::write_atomic(path, &bytes)
            .map_err(|error| format!("Could not save {}: {error}", path.display()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledFont {
    pub family: String,
    pub monospace: bool,
}

pub fn installed_fonts() -> &'static [InstalledFont] {
    static FONTS: OnceLock<Vec<InstalledFont>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut fonts = platform_fonts();
        fonts.retain(|font| !font.family.trim().is_empty());
        fonts.sort_by_cached_key(|font| font.family.to_lowercase());
        fonts.dedup_by(|a, b| a.family == b.family);
        fonts
    })
}

fn has_family(fonts: &[InstalledFont], family: &str) -> bool {
    fonts.iter().any(|font| font.family == family)
}

pub fn filter_fonts(fonts: &[InstalledFont], query: &str, show_all: bool) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    fonts
        .iter()
        .enumerate()
        .filter(|(_, font)| {
            (show_all || font.monospace) && font.family.to_lowercase().contains(&query)
        })
        .map(|(index, _)| index)
        .collect()
}

#[cfg(target_os = "macos")]
fn platform_fonts() -> Vec<InstalledFont> {
    use std::ffi::{CStr, c_char, c_void};

    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontManagerCopyAvailableFontFamilyNames() -> *const c_void;
        fn CTFontCreateWithName(
            name: *const c_void,
            size: f64,
            matrix: *const c_void,
        ) -> *const c_void;
        fn CTFontGetSymbolicTraits(font: *const c_void) -> u32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        fn CFStringGetLength(string: *const c_void) -> isize;
        fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
        fn CFStringGetCString(
            string: *const c_void,
            buffer: *mut c_char,
            size: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(object: *const c_void);
    }

    const UTF8: u32 = 0x0800_0100;
    const MONOSPACE: u32 = 1 << 10;
    let mut fonts = Vec::new();
    // SAFETY: CoreText returns a retained CFArray of borrowed CFStrings. Every
    // created/copied object is released after use; no native pointer escapes.
    unsafe {
        let families = CTFontManagerCopyAvailableFontFamilyNames();
        if families.is_null() {
            return fonts;
        }
        for index in 0..CFArrayGetCount(families) {
            let name = CFArrayGetValueAtIndex(families, index);
            if name.is_null() {
                continue;
            }
            let capacity = CFStringGetMaximumSizeForEncoding(CFStringGetLength(name), UTF8) + 1;
            if capacity <= 0 {
                continue;
            }
            let mut buffer = vec![0_u8; capacity as usize];
            if CFStringGetCString(name, buffer.as_mut_ptr().cast(), capacity, UTF8) == 0 {
                continue;
            }
            let family = CStr::from_ptr(buffer.as_ptr().cast())
                .to_string_lossy()
                .into_owned();
            let font = CTFontCreateWithName(name, DEFAULT_FONT_SIZE as f64, std::ptr::null());
            let monospace = if font.is_null() {
                false
            } else {
                let monospace = CTFontGetSymbolicTraits(font) & MONOSPACE != 0;
                CFRelease(font);
                monospace
            };
            fonts.push(InstalledFont { family, monospace });
        }
        CFRelease(families);
    }
    fonts
}

#[cfg(not(target_os = "macos"))]
fn platform_fonts() -> Vec<InstalledFont> {
    vec![InstalledFont {
        family: DEFAULT_FONT_FAMILY.into(),
        monospace: true,
    }]
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

    fn fonts() -> Vec<InstalledFont> {
        [
            ("Menlo", true),
            ("Example Sans", false),
            ("自定义等宽", true),
        ]
        .into_iter()
        .map(|(family, monospace)| InstalledFont {
            family: family.into(),
            monospace,
        })
        .collect()
    }

    #[test]
    fn defaults_and_legacy_families_migrate_to_actual_names() {
        assert_eq!(Typography::default().font_family, "Menlo");
        assert_eq!(Typography::default().font_size, 14.0);
        assert_eq!(
            serde_json::from_str::<Typography>("{}").unwrap(),
            Typography::default()
        );
        for (legacy, family) in [
            ("menlo", "Menlo"),
            ("monaco", "Monaco"),
            ("courier", "Courier"),
            ("jetbrains_mono", "JetBrains Mono"),
            (
                "jetbrains_mono_nerd_font_mono",
                "JetBrainsMono Nerd Font Mono",
            ),
        ] {
            let typography: Typography =
                serde_json::from_value(serde_json::json!({"font_family": legacy})).unwrap();
            assert_eq!(typography.font_family, family);
            assert_eq!(typography.font_size, DEFAULT_FONT_SIZE);
            assert_eq!(
                serde_json::to_value(typography).unwrap()["font_family"],
                family
            );
        }
    }

    #[test]
    fn arbitrary_families_and_fractional_sizes_round_trip() {
        let typography = Typography {
            font_family: "自定义 Mono".into(),
            font_size: 17.5,
        };
        let json = serde_json::to_string(&typography).unwrap();
        assert_eq!(
            serde_json::from_str::<Typography>(&json).unwrap(),
            typography
        );
        for json in [
            r#"{"font_family":null}"#,
            r#"{"font_family":"bad\nname"}"#,
            r#"{"font_size":"14"}"#,
            "{",
        ] {
            assert!(serde_json::from_str::<Typography>(json).is_err());
        }
    }

    #[test]
    fn sizes_are_validated_at_load_and_save() {
        for (size, expected) in [(-12.0, 10.0), (0.0, 10.0), (99.0, 28.0), (18.5, 18.5)] {
            let typography: Typography =
                serde_json::from_value(serde_json::json!({"font_size":size})).unwrap();
            assert_eq!(typography.font_size, expected);
        }
        for size in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut typography = Typography {
                font_size: size,
                ..Typography::default()
            };
            typography.clamp();
            assert_eq!(typography.font_size, DEFAULT_FONT_SIZE);
        }
        let directory = TestDirectory::new();
        let path = directory.0.join("typography.json");
        Typography {
            font_size: 100.0,
            ..Typography::default()
        }
        .save_to(&path)
        .unwrap();
        assert_eq!(Typography::load_from(&path).0.font_size, MAX_FONT_SIZE);
    }

    #[test]
    fn missing_families_use_menlo_without_losing_preference_or_fallbacks() {
        let fonts = fonts();
        for family in ["Menlo", "Example Sans", "Removed Font"] {
            let typography = Typography {
                font_family: family.into(),
                ..Typography::default()
            };
            let font = typography.font_from(&fonts);
            assert_eq!(
                font.family.as_str(),
                if family == "Removed Font" {
                    "Menlo"
                } else {
                    family
                }
            );
            assert_eq!(font.fallbacks, default_terminal_font().fallbacks);
            assert_eq!(font.features, default_terminal_font_features());
            assert_eq!(typography.font_family, family);
            assert_eq!(
                typography.missing_font_notice_from(&fonts).is_some(),
                family == "Removed Font"
            );
        }
        let directory = TestDirectory::new();
        let path = directory.0.join("typography.json");
        let missing = Typography {
            font_family: "Removed Font".into(),
            font_size: 19.0,
        };
        missing.save_to(&path).unwrap();
        assert_eq!(Typography::load_from(&path), (missing, None));
    }

    #[test]
    fn font_filter_defaults_to_monospace_and_searches_all_when_requested() {
        let fonts = fonts();
        assert_eq!(filter_fonts(&fonts, "", false), vec![0, 2]);
        assert_eq!(filter_fonts(&fonts, "", true), vec![0, 1, 2]);
        assert_eq!(filter_fonts(&fonts, "  mEn  ", false), vec![0]);
        assert!(filter_fonts(&fonts, "sans", false).is_empty());
        assert_eq!(filter_fonts(&fonts, "SANS", true), vec![1]);
        assert_eq!(filter_fonts(&fonts, "等宽", false), vec![2]);
        assert!(filter_fonts(&fonts, "missing", true).is_empty());
    }

    #[test]
    fn missing_or_invalid_files_return_safe_defaults_without_rewriting() {
        let directory = TestDirectory::new();
        let path = directory.0.join("typography.json");
        assert_eq!(Typography::load_from(&path), (Typography::default(), None));
        assert!(!path.exists());
        let invalid = br#"{"font_family":null}"#;
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
            font_family: "Monaco".into(),
            font_size: 16.0,
        };
        first.save_to(&path).unwrap();
        assert_eq!(Typography::load_from(&path), (first, None));
        let second = Typography {
            font_family: "Example Sans".into(),
            font_size: 20.0,
        };
        second.save_to(&path).unwrap();
        assert_eq!(Typography::load_from(&path), (second.clone(), None));
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
