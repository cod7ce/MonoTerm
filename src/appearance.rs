//! Window appearance preferences and an original HSL-derived classic palette.
//!
//! Colors are packed RGB (`0xRRGGBB`); alpha and the native glass effect are
//! separate so callers can apply them to the window rather than its text.

use serde::{Deserialize, Deserializer, Serialize};
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[serde(alias = "System")]
    System,
    #[default]
    #[serde(alias = "Dark")]
    Dark,
    #[serde(alias = "Light")]
    Light,
}

impl Theme {
    pub fn is_dark(self, system_is_dark: bool) -> bool {
        match self {
            Self::System => system_is_dark,
            Self::Dark => true,
            Self::Light => false,
        }
    }
}

/// Editable preferences. Angles use degrees; saturation, lightness, and opacity
/// use percentages; blur radius is a native backdrop-blur request, not a calibrated pixel size.
///
/// Missing JSON fields retain their defaults. Deserialization clamps finite
/// numbers to their supported ranges. NaN and infinities supplied in memory
/// reset to the corresponding default when clamped, rendered, or saved.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: Theme,
    #[serde(deserialize_with = "deserialize_range::<_, 0, 360, 240>")]
    pub hue: f32,
    #[serde(deserialize_with = "deserialize_range::<_, 0, 100, 0>")]
    pub saturation: f32,
    #[serde(deserialize_with = "deserialize_range::<_, 0, 30, 9>")]
    pub lightness: f32,
    #[serde(deserialize_with = "deserialize_range::<_, 0, 100, 85>")]
    pub opacity: f32,
    #[serde(deserialize_with = "deserialize_accent")]
    pub accent: u32,
    #[serde(deserialize_with = "deserialize_blur_radius")]
    pub blur_radius: u32,
    pub main_glass: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            hue: 240.0,
            saturation: 0.0,
            lightness: 9.0,
            opacity: 85.0,
            accent: 0x8989dd,
            blur_radius: 24,
            main_glass: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub shell: u32,
    pub panel: u32,
    pub terminal: u32,
    pub selected: u32,
    pub border: u32,
    pub divider: u32,
    pub text: u32,
    pub muted: u32,
    pub subtle: u32,
    pub accent: u32,
    /// Configured opacity in 0..=1. Light themes are always fully opaque.
    pub alpha: f32,
    /// Whether the host should enable its native glass effect.
    pub glass: bool,
}

impl Palette {
    pub fn terminal_alpha(self) -> f32 {
        if self.glass { self.alpha } else { 1.0 }
    }

    pub fn outer_alpha(self) -> f32 {
        // Reduce transparency by 30%, rather than subtracting percentage points.
        1.0 - (1.0 - self.alpha) * 0.7
    }

    pub fn outer_color(self) -> u32 {
        mix_rgb(self.shell, self.accent, if self.dark { 0.20 } else { 0.10 })
    }

    pub fn tab_background(self, selected: bool, hovered: bool) -> u32 {
        let alpha = match (selected, hovered, self.dark) {
            (true, true, true) => 42,
            (true, false, true) => 32,
            (false, true, true) => 12,
            (true, true, false) => 46,
            (true, false, false) => 36,
            (false, true, false) => 15,
            (false, false, _) => 0,
        };
        // The pinned renderer adds layer alpha. Leave part of the backdrop
        // visible even when the configured surround is almost opaque.
        let alpha = if self.dark && self.outer_alpha() < 1.0 {
            let remaining = (1.0 - self.outer_alpha()) * 255.0;
            let fraction = if selected {
                if hovered { 0.6 } else { 0.45 }
            } else {
                0.17
            };
            alpha.min((remaining * fraction).round() as u32)
        } else {
            alpha
        };
        // A small foreground mix keeps custom very dark/light accents visible.
        (mix_rgb(self.accent, self.text, 0.12) << 8) | alpha
    }

    /// Momentary hover treatment shared by buttons, rows and menu actions.
    pub fn control_hover(self) -> u32 {
        let alpha = if self.dark { 12 } else { 15 };
        (mix_rgb(self.accent, self.text, 0.12) << 8) | alpha
    }

    /// Momentary pressed treatment; it never survives activation.
    pub fn control_pressed(self) -> u32 {
        let alpha = if self.dark { 32 } else { 36 };
        (mix_rgb(self.accent, self.text, 0.12) << 8) | alpha
    }

    /// Persistent selection on opaque panels, matching selected tab rows.
    /// Tab rows use `tab_background` for the transparent surround's alpha cap.
    pub fn control_selected(self, hovered: bool) -> u32 {
        let alpha = match (hovered, self.dark) {
            (false, true) => 32,
            (true, true) => 42,
            (false, false) => 36,
            (true, false) => 46,
        };
        (mix_rgb(self.accent, self.text, 0.12) << 8) | alpha
    }
}

fn mix_rgb(base: u32, tint: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let base = ((base >> shift) & 0xff_u32) as f32;
        let tint = ((tint >> shift) & 0xff_u32) as f32;
        (base + (tint - base) * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

impl Appearance {
    /// Normalize public fields after editing them directly.
    pub fn clamp(&mut self) {
        self.hue = bounded::<0, 360, 240>(self.hue as f64);
        self.saturation = bounded::<0, 100, 0>(self.saturation as f64);
        self.lightness = bounded::<0, 30, 9>(self.lightness as f64);
        self.opacity = bounded::<0, 100, 85>(self.opacity as f64);
        self.accent = self.accent.min(0xffffff);
        self.blur_radius = self.blur_radius.min(64);
    }

    /// Return an RGB palette without changing the saved theme selection.
    /// Opacity and glass are independent preferences in dark mode.
    pub fn palette(&self, system_is_dark: bool) -> Palette {
        let mut appearance = *self;
        appearance.clamp();
        let h = appearance.hue;
        let s = appearance.saturation / 100.0;
        let l = appearance.lightness / 100.0;

        if appearance.theme.is_dark(system_is_dark) {
            Palette {
                dark: true,
                shell: hsl(h, s, l),
                panel: hsl(h, s, l - 0.01),
                terminal: hsl(h, s, l - 0.01),
                selected: hsl(h, s, l + 0.07),
                border: hsl(h, s, l + 0.045),
                divider: 0x212029,
                text: hsl(h, s * 0.1, 0.93),
                muted: hsl(h, s * 0.1, 0.64),
                subtle: hsl(h, s * 0.1, 0.45),
                accent: appearance.accent,
                alpha: appearance.opacity / 100.0,
                glass: appearance.main_glass,
            }
        } else {
            // Keep the dark-mode lightness preference dormant in light mode.
            // Foreground colors stay dark for readable contrast.
            let light = 0.96;
            let tint = s * 0.35;
            Palette {
                dark: false,
                shell: hsl(h, tint, light),
                panel: hsl(h, tint, light - 0.02),
                terminal: hsl(h, tint, light + 0.02),
                selected: hsl(h, tint, light - 0.08),
                border: hsl(h, tint, light - 0.075),
                divider: hsl(h, tint, light - 0.075),
                text: hsl(h, s * 0.1, 0.12),
                muted: hsl(h, s * 0.1, 0.35),
                subtle: hsl(h, s * 0.1, 0.48),
                accent: light_accent(appearance.accent),
                alpha: 1.0,
                glass: false,
            }
        }
    }

    /// Resolve the app-specific file without accessing the filesystem.
    pub fn config_path() -> io::Result<PathBuf> {
        Ok(config_directory()?.join("appearance.json"))
    }

    /// Missing, unreadable, or malformed preferences safely use all defaults.
    /// A missing file is normal first-run behavior; other errors are returned
    /// as a diagnostic. This does not create or repair a preferences file.
    pub fn load() -> (Self, Option<String>) {
        let path = match Self::config_path() {
            Ok(path) => path,
            Err(error) => return (Self::default(), Some(error.to_string())),
        };
        let bytes = match fs::read(&path) {
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
            Ok(appearance) => (appearance, None),
            Err(error) => (
                Self::default(),
                Some(format!("Could not decode {}: {error}", path.display())),
            ),
        }
    }

    /// Save a normalized snapshot using a flushed temporary file in the same
    /// directory followed by an atomic rename. All save failures are returned;
    /// a failed write leaves the previous preferences file intact.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path().map_err(|error| error.to_string())?;
        let mut appearance = *self;
        appearance.clamp();
        let mut bytes = serde_json::to_vec_pretty(&appearance)
            .map_err(|error| format!("Could not encode appearance: {error}"))?;
        bytes.push(b'\n');
        write_atomic(&path, &bytes)
            .map_err(|error| format!("Could not save {}: {error}", path.display()))
    }
}

fn bounded<const MIN: u16, const MAX: u16, const DEFAULT: u16>(value: f64) -> f32 {
    if value.is_finite() {
        value.clamp(MIN as f64, MAX as f64) as f32
    } else {
        DEFAULT as f32
    }
}

fn deserialize_range<'de, D, const MIN: u16, const MAX: u16, const DEFAULT: u16>(
    deserializer: D,
) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    // Clamp before narrowing, so a valid large JSON number cannot overflow f32.
    f64::deserialize(deserializer).map(bounded::<MIN, MAX, DEFAULT>)
}

fn deserialize_blur_radius<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    // Accept fractional/negative numbers from older or manually edited files,
    // then normalize to a whole native radius inside the supported range.
    deserialize_range::<D, 0, 64, 24>(deserializer).map(|value| value.round() as u32)
}

fn deserialize_accent<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    u32::deserialize(deserializer).map(|color| color.min(0xffffff))
}

pub fn parse_accent(text: &str) -> Option<u32> {
    let text = text.trim().strip_prefix('#').unwrap_or(text.trim());
    (text.len() == 6 && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(text, 16).ok())
        .flatten()
}

fn light_accent(color: u32) -> u32 {
    // Darken pastel accents in light mode so selected labels remain readable.
    let channel = |shift: u32| (((color >> shift) & 0xff_u32) as f32 * 0.62).round() as u32;
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn hsl(hue: f32, saturation: f32, lightness: f32) -> u32 {
    let h = hue.rem_euclid(360.0) / 60.0;
    let s = saturation.clamp(0.0, 1.0);
    let l = lightness.clamp(0.0, 1.0);
    let chroma = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let secondary = chroma * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as u8 {
        0 => (chroma, secondary, 0.0),
        1 => (secondary, chroma, 0.0),
        2 => (0.0, chroma, secondary),
        3 => (0.0, secondary, chroma),
        4 => (secondary, 0.0, chroma),
        _ => (chroma, 0.0, secondary),
    };
    let offset = l - chroma / 2.0;
    let channel = |value: f32| ((value + offset).clamp(0.0, 1.0) * 255.0).round() as u32;
    (channel(r) << 16) | (channel(g) << 8) | channel(b)
}

/// `MONOTERM_CONFIG_DIR`, when set, replaces the entire config directory.
/// An empty override is an error, avoiding accidental use of real settings
/// by an incorrectly configured GUI test.
pub(crate) fn config_directory() -> io::Result<PathBuf> {
    match env::var_os("MONOTERM_CONFIG_DIR") {
        Some(directory) if !directory.is_empty() => Ok(PathBuf::from(directory)),
        Some(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "MONOTERM_CONFIG_DIR must not be empty",
        )),
        None => {
            let home = env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not configured"))?;
            Ok(PathBuf::from(home).join("Library/Application Support/MonoTerm"))
        }
    }
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let directory = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "preferences need a directory")
    })?;
    fs::create_dir_all(directory)?;

    // create_new prevents concurrent saves (or stale files after a crash) from
    // sharing a temporary file. The final successful rename wins.
    for _ in 0..128 {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(
            ".preferences.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&temporary) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        return result;
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary preferences file",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_partial_deserialization_preserve_preferences() {
        let defaults = Appearance::default();
        assert_eq!(Theme::default(), Theme::Dark);
        assert_eq!(defaults.hue, 240.0);
        assert_eq!(defaults.saturation, 0.0);
        assert_eq!(defaults.lightness, 9.0);
        assert_eq!(defaults.opacity, 85.0);
        assert_eq!(defaults.accent, 0x8989dd);
        assert_eq!(defaults.blur_radius, 24);
        assert!(defaults.main_glass);
        assert_eq!(serde_json::from_str::<Appearance>("{}").unwrap(), defaults);
        let partial: Appearance =
            serde_json::from_str(r#"{"theme":"system","opacity":40}"#).unwrap();
        assert_eq!(
            partial,
            Appearance {
                theme: Theme::System,
                opacity: 40.0,
                ..defaults
            }
        );
        let json = serde_json::to_string(&partial).unwrap();
        assert_eq!(serde_json::from_str::<Appearance>(&json).unwrap(), partial);
    }

    #[test]
    fn deserialization_clamps_finite_values_before_narrowing() {
        let appearance: Appearance = serde_json::from_str(
            r#"{"hue":1e100,"saturation":-20,"lightness":50,"opacity":-1,"blur_radius":200}"#,
        )
        .unwrap();
        assert_eq!(appearance.hue, 360.0);
        assert_eq!(appearance.saturation, 0.0);
        assert_eq!(appearance.lightness, 30.0);
        assert_eq!(appearance.opacity, 0.0);
        assert_eq!(appearance.blur_radius, 64);
        for (json, expected) in [
            (r#"{"blur_radius":-1}"#, 0),
            (r#"{"blur_radius":23.6}"#, 24),
            (r#"{"blur_radius":1e100}"#, 64),
        ] {
            let appearance: Appearance = serde_json::from_str(json).unwrap();
            assert_eq!(appearance.blur_radius, expected);
        }
        for json in [r#"{"theme":"unknown"}"#, r#"{"opacity":null}"#, "{"] {
            assert!(serde_json::from_str::<Appearance>(json).is_err());
        }
    }

    #[test]
    fn clamp_covers_both_bounds_and_non_finite_values() {
        for (input, expected) in [
            (-1000.0, [0.0, 0.0, 0.0, 0.0]),
            (1000.0, [360.0, 100.0, 30.0, 100.0]),
            (f32::NAN, [240.0, 0.0, 9.0, 85.0]),
            (f32::INFINITY, [240.0, 0.0, 9.0, 85.0]),
            (f32::NEG_INFINITY, [240.0, 0.0, 9.0, 85.0]),
        ] {
            let mut appearance = Appearance {
                hue: input,
                saturation: input,
                lightness: input,
                opacity: input,
                ..Appearance::default()
            };
            appearance.clamp();
            assert_eq!(
                [
                    appearance.hue,
                    appearance.saturation,
                    appearance.lightness,
                    appearance.opacity,
                ],
                expected
            );
            let once = appearance;
            appearance.clamp();
            assert_eq!(appearance, once);
        }
    }

    #[test]
    fn clamp_bounds_integer_blur_radius() {
        for (input, expected) in [(0, 0), (24, 24), (64, 64), (u32::MAX, 64)] {
            let mut appearance = Appearance {
                blur_radius: input,
                ..Appearance::default()
            };
            appearance.clamp();
            assert_eq!(appearance.blur_radius, expected);
        }
    }

    #[test]
    fn palettes_resolve_system_theme_and_keep_light_opaque() {
        let appearance = Appearance::default();
        let dark = appearance.palette(false);
        assert_eq!(dark.shell, 0x171717);
        assert_eq!(dark.panel, 0x141414);
        assert_eq!(dark.terminal, 0x141414);
        assert!((dark.alpha - 0.85).abs() < f32::EPSILON);
        assert!(dark.glass);
        assert_eq!(appearance.palette(true), dark);

        let light = Appearance {
            theme: Theme::Light,
            opacity: 15.0,
            ..appearance
        }
        .palette(true);
        assert_eq!(light.alpha, 1.0);
        assert!(!light.glass);
        assert_eq!(
            light,
            Appearance {
                theme: Theme::Light,
                lightness: 30.0,
                ..appearance
            }
            .palette(true)
        );
        assert!(contrast(light.text, light.shell) > 7.0);
        assert!(contrast(dark.text, dark.shell) > 7.0);
        assert!(luminance(light.shell) > luminance(dark.shell));
        assert!(luminance(light.text) < luminance(dark.text));
        let system = Appearance {
            theme: Theme::System,
            ..appearance
        };
        assert_eq!(system.palette(true), dark);
        assert_eq!(system.palette(false), light);
        assert_eq!(system.theme, Theme::System);

        let no_glass = Appearance {
            main_glass: false,
            ..appearance
        }
        .palette(true);
        assert!(!no_glass.glass);
        assert_eq!(no_glass.alpha, dark.alpha);
    }

    #[test]
    fn shell_opacity_and_surround_transparency_remain_independent() {
        for (opacity, terminal, outer) in [(0.0, 0.0, 0.3), (60.0, 0.6, 0.72), (100.0, 1.0, 1.0)] {
            let appearance = Appearance {
                opacity,
                ..Appearance::default()
            };
            let palette = appearance.palette(true);
            assert!((palette.terminal_alpha() - terminal).abs() < 0.00001);
            assert!((palette.outer_alpha() - outer).abs() < 0.00001);
            let solid = Appearance {
                main_glass: false,
                ..appearance
            }
            .palette(true);
            assert_eq!(solid.terminal_alpha(), 1.0);
            assert_eq!(solid.outer_alpha(), palette.outer_alpha());
            let light = Appearance {
                theme: Theme::Light,
                ..appearance
            }
            .palette(true);
            assert_eq!(light.terminal_alpha(), 1.0);
            assert_eq!(light.outer_alpha(), 1.0);
        }
    }

    #[test]
    fn accent_is_independent_and_legacy_preferences_keep_their_material() {
        let legacy: Appearance =
            serde_json::from_str(r#"{"hue":280,"opacity":40,"blur_radius":12}"#).unwrap();
        assert_eq!(legacy.accent, Appearance::default().accent);
        assert_eq!(legacy.palette(true).alpha, 0.4);
        let blue = Appearance {
            accent: 0x6aa9ef,
            ..legacy
        };
        assert_eq!(blue.palette(true).shell, legacy.palette(true).shell);
        assert_eq!(blue.palette(true).accent, 0x6aa9ef);
        assert_eq!(parse_accent(" #abcdef "), Some(0xabcdef));
        assert_eq!(parse_accent("6AA9EF"), Some(0x6aa9ef));
        for invalid in ["#abc", "blue", "#12345678", "GGGGGG", ""] {
            assert_eq!(parse_accent(invalid), None);
        }
        let clear = Appearance {
            opacity: 0.0,
            blur_radius: 0,
            ..blue
        };
        assert_eq!(clear.palette(true).alpha, 0.0);
        assert_eq!(
            serde_json::from_str::<Appearance>(&serde_json::to_string(&clear).unwrap()).unwrap(),
            clear
        );
    }

    #[test]
    fn control_states_share_the_tab_selection_family() {
        for theme in [Theme::Dark, Theme::Light] {
            let appearance = Appearance {
                theme,
                opacity: 100.0,
                main_glass: false,
                ..Appearance::default()
            };
            let p = appearance.palette(true);
            let hover = p.control_hover() & 0xff;
            let pressed = p.control_pressed() & 0xff;
            let selected = p.control_selected(false) & 0xff;
            let selected_hover = p.control_selected(true) & 0xff;
            assert!(hover < pressed && pressed < selected_hover);
            assert_eq!(selected, pressed);
            assert!(selected < selected_hover);
            assert_eq!(p.tab_background(true, false), p.control_selected(false));
            assert_eq!(p.tab_background(true, true), p.control_selected(true));
        }
    }

    #[test]
    fn palette_hue_tints_surfaces_and_wraps_at_360_degrees() {
        let red = Appearance {
            hue: 0.0,
            saturation: 100.0,
            ..Appearance::default()
        };
        let green = Appearance { hue: 120.0, ..red };
        assert_eq!(red.palette(true).shell, 0x2e0000);
        assert_eq!(green.palette(true).shell, 0x002e00);
        assert_eq!(
            red.palette(true),
            Appearance { hue: 360.0, ..red }.palette(true)
        );
        assert_eq!(
            Appearance {
                hue: f32::NAN,
                opacity: f32::INFINITY,
                ..Appearance::default()
            }
            .palette(true),
            Appearance::default().palette(true)
        );
        assert_eq!(hsl(240.0, 1.0, 0.5), 0x0000ff);
    }

    fn luminance(rgb: u32) -> f32 {
        let linear = |shift: u32| {
            let c = ((rgb >> shift) & 0xff_u32) as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0)
    }

    fn contrast(a: u32, b: u32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}
