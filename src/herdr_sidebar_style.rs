//! Read the workspace-name colors from Herdr's shared sidebar configuration.
use crate::appearance::Palette;
use std::{io::Read, path::PathBuf};
use toml::Value;

#[derive(Default)]
pub(super) struct SidebarStyle {
    spaces: Vec<Value>,
    agents: Vec<Value>,
}

impl SidebarStyle {
    pub(super) fn load() -> Self {
        let path = herdr_client::local_var("HERDR_CONFIG_PATH")
            .map(PathBuf::from)
            .or_else(|| {
                let root = herdr_client::local_var("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .or_else(|| {
                        herdr_client::local_var("HOME")
                            .map(|home| PathBuf::from(home).join(".config"))
                    })?;
                Some(root.join("herdr/config.toml"))
            });
        let Some(path) = path else {
            return Self::default();
        };
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                eprintln!("Read Herdr sidebar colors: {error}");
                return Self::default();
            }
        };
        let mut text = String::new();
        if let Err(error) = file.take(1024 * 1024 + 1).read_to_string(&mut text) {
            eprintln!("Read Herdr sidebar colors: {error}");
            return Self::default();
        }
        if text.len() > 1024 * 1024 {
            eprintln!("Herdr sidebar config exceeds 1 MiB");
            return Self::default();
        }
        match text.parse::<Value>() {
            Ok(config) => Self::from_config(&config),
            Err(error) => {
                eprintln!("Parse Herdr sidebar colors: {error}");
                Self::default()
            }
        }
    }

    fn from_config(config: &Value) -> Self {
        let tokens = |scope| {
            config
                .get("ui")
                .and_then(|ui| ui.get("sidebar"))
                .and_then(|sidebar| sidebar.get(scope))
                .and_then(|scope| scope.get("rows"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_array)
                .flatten()
                .filter(|token| token.get("token").and_then(Value::as_str) == Some("workspace"))
                .cloned()
                .collect()
        };
        Self {
            spaces: tokens("spaces"),
            agents: tokens("agents"),
        }
    }

    pub(super) fn color(&self, label: &str, agent: bool, palette: Palette) -> Option<u32> {
        let tokens = if agent { &self.agents } else { &self.spaces };
        let color = tokens.iter().find_map(|token| {
            let rule = token
                .get("rules")
                .and_then(Value::as_array)
                .and_then(|rules| rules.iter().find(|rule| matches(rule, label)));
            rule.and_then(|rule| rule.get("fg"))
                .or_else(|| token.get("fg"))
                .and_then(Value::as_str)
                .and_then(parse_color)
        })?;
        if palette.dark {
            Some(color)
        } else {
            // TUI colors are usually pastels; darken them on the opaque Light surface.
            let channel = |shift: u32| ((color >> shift) & 255u32) * 45 / 100;
            Some((channel(16) << 16) | (channel(8) << 8) | channel(0))
        }
    }
}

fn matches(rule: &Value, label: &str) -> bool {
    let keys = ["equals", "contains", "starts_with", "gt", "lt"];
    let mut conditions = keys
        .iter()
        .filter_map(|key| rule.get(key).map(|value| (*key, value)));
    let Some((kind, expected)) = conditions.next() else {
        return false;
    };
    if conditions.next().is_some() {
        return false;
    }
    if matches!(kind, "gt" | "lt") {
        let threshold = expected
            .as_float()
            .or_else(|| expected.as_integer().map(|n| n as f64));
        return label
            .parse::<f64>()
            .ok()
            .zip(threshold)
            .is_some_and(|(number, threshold)| {
                number.is_finite()
                    && threshold.is_finite()
                    && if kind == "gt" {
                        number > threshold
                    } else {
                        number < threshold
                    }
            });
    }
    let Some(expected) = expected.as_str() else {
        return false;
    };
    let folded = rule
        .get("ignore_case")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let (label, expected) = if folded {
        (label.to_ascii_lowercase(), expected.to_ascii_lowercase())
    } else {
        (label.to_owned(), expected.to_owned())
    };
    match kind {
        "equals" => label == expected,
        "contains" => label.contains(&expected),
        "starts_with" => label.starts_with(&expected),
        _ => false,
    }
}

fn parse_color(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#')?;
    if !matches!(hex.len(), 3 | 6) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let expanded = if hex.len() == 3 {
        hex.chars().flat_map(|ch| [ch, ch]).collect::<String>()
    } else {
        hex.into()
    };
    u32::from_str_radix(&expanded, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_color_rules_are_scoped_and_first_match_wins() {
        let config = r##"
[ui.sidebar.spaces]
rows = [[{ token = "workspace", fg = "#abc", rules = [{ equals = "🐸 Repo", fg = "#bb9af7" }, { contains = "Repo", fg = "#fff" }] }]]
[ui.sidebar.agents]
rows = [[{ token = "workspace", rules = [{ starts_with = "🐸 repo", ignore_case = true, fg = "#bb9af7" }] }]]
"##.parse::<Value>().expect("valid display fixture");
        let style = SidebarStyle::from_config(&config);
        let palette = crate::appearance::Appearance::default().palette(true);
        assert_eq!(style.color("🐸 Repo", false, palette), Some(0xbb9af7));
        assert_eq!(style.color("🐸 Repo", true, palette), Some(0xbb9af7));
        assert_eq!(style.color("Other", false, palette), Some(0xaabbcc));
        assert_eq!(style.color("Other", true, palette), None);
        assert_eq!(parse_color("#badcolor"), None);
    }
}
