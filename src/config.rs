//! Configuration file loading and defaults.

use serde::{Deserialize, Deserializer};
use std::path::{Path, PathBuf};

/// One entry of the `+` and `⌄` menus: a name to show and the shell to start.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ProfileConfig {
    pub name: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl ProfileConfig {
    /// The shells a first run offers before any config file exists.
    ///
    /// Windows has two named shells to choose from. Every other target has one
    /// profile with an empty `program`, which makes `build_command` resolve the
    /// user's `$SHELL`; a fixed name here could only guess wrong and would stop
    /// the app from spawning at all on a clean machine.
    fn platform_defaults() -> Vec<Self> {
        #[cfg(windows)]
        {
            vec![
                Self {
                    name: "Windows PowerShell".to_string(),
                    program: "powershell.exe".to_string(),
                    args: Vec::new(),
                },
                Self {
                    name: "Command Prompt".to_string(),
                    program: "cmd.exe".to_string(),
                    args: Vec::new(),
                },
            ]
        }
        #[cfg(not(windows))]
        {
            vec![Self {
                name: "Default".to_string(),
                program: String::new(),
                args: Vec::new(),
            }]
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub font: FontConfig,
    pub bidi: BidiConfig,
    pub window: WindowConfig,
    pub scrollback: ScrollbackConfig,
    pub selection: SelectionConfig,
    pub theme: ThemeConfig,
    pub spellcheck: SpellcheckConfig,
    pub shell: ShellConfig,
    #[serde(default)]
    pub profiles: Vec<ProfileConfig>,
    #[serde(default)]
    pub default_profile: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: FontConfig::default(),
            bidi: BidiConfig::default(),
            window: WindowConfig::default(),
            scrollback: ScrollbackConfig::default(),
            selection: SelectionConfig::default(),
            theme: ThemeConfig::default(),
            spellcheck: SpellcheckConfig::default(),
            shell: ShellConfig::default(),
            profiles: ProfileConfig::platform_defaults(),
            default_profile: 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct FontConfig {
    pub family: String,
    /// Empty means auto: pick the Arabic face whose advance fills the
    /// primary cell; set a name to force one face.
    pub arabic_family: String,
    pub size: f32,
    pub line_height: f32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: "Cascadia Mono".into(),
            arabic_family: String::new(),
            size: 16.0,
            line_height: 1.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct BidiConfig {
    pub enabled: bool,
    pub base_direction: BaseDirection,
}

impl Default for BidiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_direction: BaseDirection::Ltr,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BaseDirection {
    #[default]
    Ltr,
    Rtl,
    Auto,
}

impl<'de> Deserialize<'de> for BaseDirection {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "rtl" => Self::Rtl,
            "auto" => Self::Auto,
            "ltr" => Self::Ltr,
            _ => {
                tracing::warn!("unknown base_direction {value:?}; using ltr");
                Self::Ltr
            }
        })
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
    pub padding: u32,
    /// When false, OSC 0/2 title changes are ignored and the window keeps
    /// its default title.
    pub dynamic_title: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1440,
            height: 800,
            padding: 8,
            dynamic_title: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct ScrollbackConfig {
    pub lines: usize,
}

impl Default for ScrollbackConfig {
    fn default() -> Self {
        Self { lines: 10_000 }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct SelectionConfig {
    pub trim_trailing: bool,
    pub copy_on_select: bool,
}

impl Default for SelectionConfig {
    fn default() -> Self {
        Self {
            trim_trailing: true,
            copy_on_select: true,
        }
    }
}

/// The `[spellcheck]` section: whether to decorate misspellings and whether the
/// child shell gets the autocorrect environment.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct SpellcheckConfig {
    /// Underline misspelled words on the shell's live input line. A scrolled-back
    /// viewport shows none, and "Hide underlines" in the right-click menu turns
    /// them off for the rest of the session without rewriting this setting.
    pub enabled: bool,
    /// Export `TERMYX_AUTOCORRECT=1` (or `0`) to every child shell. The
    /// PowerShell module reads it to enable or disable its autocorrect on the
    /// same line. The prompt marks are independent: even with `false`, the
    /// module still wraps the prompt when `TERMYX=1`, so the underlines and the
    /// right-click menu keep working while the shell does not autocorrect.
    pub autocorrect: bool,
    /// `#rrggbb` for the underline band. Invalid values fall back to
    /// `#d19a66`; the field is a string so the parser stays in [`crate::theme`].
    pub underline_color: String,
}

impl Default for SpellcheckConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            autocorrect: true,
            underline_color: "#d19a66".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct ThemeConfig {
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    pub selection: String,
    pub black: String,
    pub red: String,
    pub green: String,
    pub yellow: String,
    pub blue: String,
    pub magenta: String,
    pub cyan: String,
    pub white: String,
    pub bright_black: String,
    pub bright_red: String,
    pub bright_green: String,
    pub bright_yellow: String,
    pub bright_blue: String,
    pub bright_magenta: String,
    pub bright_cyan: String,
    pub bright_white: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            background: "#0c0c0c".into(),
            foreground: "#cccccc".into(),
            cursor: "#ffffff".into(),
            selection: "#ffffff".into(),
            black: "#0c0c0c".into(),
            red: "#c50f1f".into(),
            green: "#13a10e".into(),
            yellow: "#c19c00".into(),
            blue: "#0037da".into(),
            magenta: "#881798".into(),
            cyan: "#3a96dd".into(),
            white: "#cccccc".into(),
            bright_black: "#767676".into(),
            bright_red: "#e74856".into(),
            bright_green: "#16c60c".into(),
            bright_yellow: "#f9f1a5".into(),
            bright_blue: "#3b78ff".into(),
            bright_magenta: "#b4009e".into(),
            bright_cyan: "#61d6d6".into(),
            bright_white: "#f2f2f2".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ShellConfig {
    pub program: String,
    pub args: Vec<String>,
}

/// Section names and field names accepted by the schema. Mirrors the struct
/// fields above; anything else in the file is logged and ignored.
const KNOWN_KEYS: &[(&str, &[&str])] = &[
    ("font", &["family", "arabic_family", "size", "line_height"]),
    ("bidi", &["enabled", "base_direction"]),
    ("window", &["width", "height", "padding", "dynamic_title"]),
    ("scrollback", &["lines"]),
    ("selection", &["trim_trailing", "copy_on_select"]),
    ("spellcheck", &["enabled", "autocorrect", "underline_color"]),
    (
        "theme",
        &[
            "background",
            "foreground",
            "cursor",
            "selection",
            "black",
            "red",
            "green",
            "yellow",
            "blue",
            "magenta",
            "cyan",
            "white",
            "bright_black",
            "bright_red",
            "bright_green",
            "bright_yellow",
            "bright_blue",
            "bright_magenta",
            "bright_cyan",
            "bright_white",
        ],
    ),
    ("shell", &["program", "args"]),
    ("profiles", &["name", "program", "args"]),
    ("default_profile", &[]),
];

fn warn_unknown_keys(value: &toml::Value) {
    let Some(table) = value.as_table() else {
        return;
    };
    for section in table.keys() {
        if !KNOWN_KEYS.iter().any(|(name, _)| *name == section.as_str()) {
            tracing::warn!("unknown config section {section:?}; ignoring");
        }
    }
    for (section, keys) in KNOWN_KEYS {
        let Some(entry) = table.get(*section) else {
            continue;
        };
        let value = match entry {
            toml::Value::Array(items) => match items.first() {
                Some(toml::Value::Table(_)) => Some(entry),
                _ => None,
            },
            toml::Value::Table(_) => Some(entry),
            _ => None,
        };
        let Some(value) = value else { continue };
        let tables: Vec<&toml::Table> = match value {
            toml::Value::Array(items) => items.iter().filter_map(toml::Value::as_table).collect(),
            toml::Value::Table(table) => vec![table],
            _ => continue,
        };
        for table in tables {
            for key in table.keys() {
                if !keys.contains(&key.as_str()) {
                    tracing::warn!("unknown key {section}.{key}; ignoring");
                }
            }
        }
    }
}

/// The tab title for a profile rebuilt from the legacy `[shell]` key.
///
/// A `[shell]` has no `name` of its own, but its tab still has to say which
/// shell it runs: "Default" hides that. The file stem identifies the program,
/// and the three shells Windows ships get the names its own terminal gives
/// them. Any other program keeps its stem exactly as typed, because guessing
/// a friendly name from an unknown binary would be wrong more often than not.
fn shell_profile_name(program: &str) -> String {
    let Some(stem) = Path::new(program)
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
    else {
        // An empty program has no stem; the whole string is the best name
        // there is, and an empty one keeps the tab on its fallback title.
        return program.to_string();
    };
    match stem.to_ascii_lowercase().as_str() {
        "powershell" => "Windows PowerShell".to_string(),
        "pwsh" => "PowerShell".to_string(),
        "cmd" => "Command Prompt".to_string(),
        _ => stem.to_string(),
    }
}

impl Config {
    /// Parses configuration from TOML text, warning about unknown sections and
    /// keys, and clamping out-of-range values.
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let value: toml::Value = toml::from_str(text)?;
        warn_unknown_keys(&value);
        Ok(toml::from_str::<Self>(text)?.validate())
    }

    /// Clamps values into their supported ranges, replacing non-finite font
    /// metrics with their defaults.
    pub fn validate(mut self) -> Self {
        if !self.font.size.is_finite() {
            self.font.size = 16.0;
        }
        self.font.size = self.font.size.clamp(6.0, 48.0);
        if !self.font.line_height.is_finite() || self.font.line_height < 0.8 {
            self.font.line_height = 1.0;
        }
        self.scrollback.lines = self.scrollback.lines.clamp(1_000, 200_000);
        self.window.width = self.window.width.clamp(200, 10_000);
        self.window.height = self.window.height.clamp(200, 10_000);
        self.window.padding = self.window.padding.clamp(0, 64);
        if crate::theme::Rgb::from_hex(&self.spellcheck.underline_color).is_none() {
            self.spellcheck.underline_color = SpellcheckConfig::default().underline_color;
        }
        if self.profiles.is_empty() {
            self.profiles = if self.shell == ShellConfig::default() {
                ProfileConfig::platform_defaults()
            } else {
                vec![ProfileConfig {
                    name: shell_profile_name(&self.shell.program),
                    program: self.shell.program.clone(),
                    args: self.shell.args.clone(),
                }]
            };
        }
        if self.default_profile >= self.profiles.len() {
            self.default_profile = 0;
        }
        self
    }

    /// Loads configuration from `path`, or from [`default_config_path`] when
    /// `path` is `None`. A missing default file yields validated defaults; a
    /// missing explicit file is an error.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let explicit = path.is_some();
        let path = path
            .map(Path::to_path_buf)
            .unwrap_or_else(default_config_path);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::from_toml_str(&text),
            Err(source) if !explicit && source.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self::default().validate())
            }
            Err(source) => Err(ConfigError::Io { path, source }),
        }
    }

    /// Writes the editable settings into `path`, keeping every other key.
    ///
    /// The existing file is parsed into a [`toml::Value`] and only the keys the
    /// Settings page can change are replaced, so unknown sections, the shell
    /// profiles, and the startup window size survive a save. The new text is
    /// written to a sibling temporary file and renamed into place, so a
    /// serialization or I/O failure cannot leave a half-written config.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let mut root = match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str::<toml::Value>(&text)?,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                toml::Value::Table(toml::Table::new())
            }
            Err(source) => {
                return Err(ConfigError::Io {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };
        let Some(table) = root.as_table_mut() else {
            return Err(ConfigError::NotATable {
                path: path.to_path_buf(),
            });
        };
        self.write_editable(table)?;
        let text = toml::to_string_pretty(table)?;
        write_atomically(path, &text)
    }

    fn write_editable(&self, root: &mut toml::Table) -> Result<(), ConfigError> {
        let font = editable_section(root, "font")?;
        font.insert(
            "family".to_string(),
            toml::Value::String(self.font.family.clone()),
        );
        font.insert(
            "arabic_family".to_string(),
            toml::Value::String(self.font.arabic_family.clone()),
        );
        font.insert(
            "size".to_string(),
            toml::Value::Float(f64::from(self.font.size)),
        );
        font.insert(
            "line_height".to_string(),
            toml::Value::Float(f64::from(self.font.line_height)),
        );

        let bidi = editable_section(root, "bidi")?;
        bidi.insert(
            "base_direction".to_string(),
            toml::Value::String(
                match self.bidi.base_direction {
                    BaseDirection::Ltr => "ltr",
                    BaseDirection::Rtl => "rtl",
                    BaseDirection::Auto => "auto",
                }
                .to_string(),
            ),
        );

        let window = editable_section(root, "window")?;
        window.insert(
            "padding".to_string(),
            toml::Value::Integer(i64::from(self.window.padding)),
        );
        window.insert(
            "dynamic_title".to_string(),
            toml::Value::Boolean(self.window.dynamic_title),
        );

        let scrollback = editable_section(root, "scrollback")?;
        scrollback.insert(
            "lines".to_string(),
            toml::Value::Integer(self.scrollback.lines as i64),
        );

        let selection = editable_section(root, "selection")?;
        selection.insert(
            "trim_trailing".to_string(),
            toml::Value::Boolean(self.selection.trim_trailing),
        );
        selection.insert(
            "copy_on_select".to_string(),
            toml::Value::Boolean(self.selection.copy_on_select),
        );

        let theme = editable_section(root, "theme")?;
        for (key, value) in [
            ("background", &self.theme.background),
            ("foreground", &self.theme.foreground),
            ("cursor", &self.theme.cursor),
            ("selection", &self.theme.selection),
            ("black", &self.theme.black),
            ("red", &self.theme.red),
            ("green", &self.theme.green),
            ("yellow", &self.theme.yellow),
            ("blue", &self.theme.blue),
            ("magenta", &self.theme.magenta),
            ("cyan", &self.theme.cyan),
            ("white", &self.theme.white),
            ("bright_black", &self.theme.bright_black),
            ("bright_red", &self.theme.bright_red),
            ("bright_green", &self.theme.bright_green),
            ("bright_yellow", &self.theme.bright_yellow),
            ("bright_blue", &self.theme.bright_blue),
            ("bright_magenta", &self.theme.bright_magenta),
            ("bright_cyan", &self.theme.bright_cyan),
            ("bright_white", &self.theme.bright_white),
        ] {
            theme.insert(key.to_string(), toml::Value::String(value.clone()));
        }
        Ok(())
    }
}

/// A mutable table for one editable section, creating it when the file has
/// none and replacing a non-table value that the schema cannot read anyway.
fn editable_section<'a>(
    root: &'a mut toml::Table,
    name: &str,
) -> Result<&'a mut toml::Table, ConfigError> {
    if !root.get(name).is_some_and(toml::Value::is_table) {
        root.insert(name.to_string(), toml::Value::Table(toml::Table::new()));
    }
    root.get_mut(name)
        .and_then(toml::Value::as_table_mut)
        .ok_or_else(|| ConfigError::Section(name.to_string()))
}

/// Writes `text` beside `path` and renames it over the old file, so a failure
/// leaves the previous config untouched.
fn write_atomically(path: &Path, text: &str) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    if let Err(source) = std::fs::write(&temporary, text) {
        let _ = std::fs::remove_file(&temporary);
        return Err(ConfigError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    if let Err(source) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(ConfigError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read config at {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("could not serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("config at {path} is not a table")]
    NotATable { path: PathBuf },
    #[error("config section {0:?} is not a table")]
    Section(String),
}

/// Returns the platform-specific default configuration file path.
pub fn default_config_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("termyx").join("config.toml");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Application Support/termyx/config.toml");
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join("termyx").join("config.toml");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".config/termyx/config.toml");
        }
    }
    PathBuf::from("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = Config::default();
        assert_eq!(c.font.family, "Cascadia Mono");
        assert_eq!(c.font.arabic_family, "");
        assert_eq!(c.font.size, 16.0);
        assert_eq!(c.font.line_height, 1.0);
        assert!(c.bidi.enabled);
        assert_eq!(c.bidi.base_direction, BaseDirection::Ltr);
        assert_eq!(c.window.width, 1440);
        assert_eq!(c.window.height, 800);
        assert_eq!(c.window.padding, 8);
        assert!(!c.window.dynamic_title);
        assert_eq!(c.scrollback.lines, 10_000);
        assert!(c.selection.trim_trailing);
        assert!(c.selection.copy_on_select);
        assert_eq!(c.theme.background, "#0c0c0c");
        assert_eq!(c.theme.foreground, "#cccccc");
        assert_eq!(c.theme.cursor, "#ffffff");
        assert_eq!(c.theme.selection, "#ffffff");
        assert_eq!(c.theme.black, "#0c0c0c");
        assert_eq!(c.theme.red, "#c50f1f");
        assert_eq!(c.theme.green, "#13a10e");
        assert_eq!(c.theme.yellow, "#c19c00");
        assert_eq!(c.theme.blue, "#0037da");
        assert_eq!(c.theme.magenta, "#881798");
        assert_eq!(c.theme.cyan, "#3a96dd");
        assert_eq!(c.theme.white, "#cccccc");
        assert_eq!(c.theme.bright_black, "#767676");
        assert_eq!(c.theme.bright_red, "#e74856");
        assert_eq!(c.theme.bright_green, "#16c60c");
        assert_eq!(c.theme.bright_yellow, "#f9f1a5");
        assert_eq!(c.theme.bright_blue, "#3b78ff");
        assert_eq!(c.theme.bright_magenta, "#b4009e");
        assert_eq!(c.theme.bright_cyan, "#61d6d6");
        assert_eq!(c.theme.bright_white, "#f2f2f2");
        assert_eq!(c.shell.program, "");
        assert!(c.shell.args.is_empty());
    }

    #[test]
    fn font_size_is_clamped() {
        let c = Config::from_toml_str("[font]\nsize = 100.0\n")
            .unwrap()
            .validate();
        assert_eq!(c.font.size, 48.0);
        let c = Config::from_toml_str("[font]\nsize = 1.0\n")
            .unwrap()
            .validate();
        assert_eq!(c.font.size, 6.0);
    }

    #[test]
    fn nan_font_size_falls_back_to_default() {
        let c = Config::from_toml_str("[font]\nsize = nan\n").unwrap();
        assert_eq!(c.font.size, 16.0);
    }

    #[test]
    fn line_height_falls_back_to_default() {
        let c = Config::from_toml_str("[font]\nline_height = 0.5\n").unwrap();
        assert_eq!(c.font.line_height, 1.0);
        let c = Config::from_toml_str("[font]\nline_height = nan\n").unwrap();
        assert_eq!(c.font.line_height, 1.0);
    }

    #[test]
    fn scrollback_is_clamped() {
        let c = Config::from_toml_str("[scrollback]\nlines = 10\n")
            .unwrap()
            .validate();
        assert_eq!(c.scrollback.lines, 1_000);
        let c = Config::from_toml_str("[scrollback]\nlines = 999999\n")
            .unwrap()
            .validate();
        assert_eq!(c.scrollback.lines, 200_000);
    }

    #[test]
    fn dynamic_title_reads_from_window_table() {
        let c = Config::from_toml_str("[window]\ndynamic_title = true\n").unwrap();
        assert!(c.window.dynamic_title);
    }

    #[test]
    fn window_geometry_and_padding_are_clamped() {
        let c =
            Config::from_toml_str("[window]\nwidth = 0\nheight = 99999\npadding = 500\n").unwrap();
        assert_eq!(c.window.width, 200);
        assert_eq!(c.window.height, 10_000);
        assert_eq!(c.window.padding, 64);

        let c =
            Config::from_toml_str("[window]\nwidth = 1200\nheight = 800\npadding = 0\n").unwrap();
        assert_eq!(c.window.width, 1200);
        assert_eq!(c.window.height, 800);
        assert_eq!(c.window.padding, 0);
    }

    #[test]
    fn invalid_base_direction_falls_back_to_ltr() {
        let c = Config::from_toml_str("[bidi]\nbase_direction = \"sideways\"\n").unwrap();
        assert_eq!(c.bidi.base_direction, BaseDirection::Ltr);
        let c = Config::from_toml_str("[bidi]\nbase_direction = \"rtl\"\n").unwrap();
        assert_eq!(c.bidi.base_direction, BaseDirection::Rtl);
    }

    #[test]
    fn spellcheck_defaults_are_on() {
        let config = Config::default();
        assert!(config.spellcheck.enabled);
        assert!(config.spellcheck.autocorrect);
        assert_eq!(config.spellcheck.underline_color, "#d19a66");
    }

    #[test]
    fn spellcheck_reads_from_table() {
        let config = Config::from_toml_str(
            "[spellcheck]\nenabled = false\nautocorrect = false\nunderline_color = \"#ff0000\"\n",
        )
        .expect("parse");
        assert!(!config.spellcheck.enabled);
        assert!(!config.spellcheck.autocorrect);
        assert_eq!(config.spellcheck.underline_color, "#ff0000");

        // A partial table leaves the other two keys at their defaults.
        let config =
            Config::from_toml_str("[spellcheck]\nenabled = false\n").expect("partial parse");
        assert!(!config.spellcheck.enabled);
        assert!(config.spellcheck.autocorrect);
        assert_eq!(config.spellcheck.underline_color, "#d19a66");
    }

    #[test]
    fn invalid_underline_color_falls_back() {
        let config = Config::from_toml_str("[spellcheck]\nunderline_color = \"not-a-colour\"\n")
            .expect("parse");
        assert_eq!(config.spellcheck.underline_color, "#d19a66");

        let config =
            Config::from_toml_str("[spellcheck]\nunderline_color = \"\"\n").expect("empty parse");
        assert_eq!(config.spellcheck.underline_color, "#d19a66");
    }

    #[test]
    fn spellcheck_keys_are_known() {
        // A warning-only unknown-key path cannot fail a parse, so pin the
        // schema itself: every key the table documents must be listed.
        let keys = KNOWN_KEYS
            .iter()
            .find(|(section, _)| *section == "spellcheck")
            .map(|(_, keys)| *keys)
            .expect("spellcheck must be a known section");
        assert_eq!(keys, ["enabled", "autocorrect", "underline_color"]);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let c = Config::from_toml_str("[font]\nwat = 1\n").unwrap();
        assert_eq!(c.font.size, 16.0);
    }

    #[test]
    fn default_config_path_contains_app_name() {
        let p = default_config_path();
        assert!(p.to_string_lossy().contains("termyx"));
        assert!(p.to_string_lossy().ends_with("config.toml"));
    }

    #[test]
    fn profiles_default_to_the_platform_shells() {
        let config = Config::from_toml_str("").unwrap();
        let names: Vec<&str> = config.profiles.iter().map(|p| p.name.as_str()).collect();
        #[cfg(windows)]
        {
            assert_eq!(names, ["Windows PowerShell", "Command Prompt"]);
            assert_eq!(config.profiles[0].program, "powershell.exe");
            assert_eq!(config.profiles[1].program, "cmd.exe");
        }
        #[cfg(not(windows))]
        {
            // An empty program is what sends `build_command` to `$SHELL`; a
            // hard-coded name here is the reason the app could not start on
            // Linux or macOS.
            assert_eq!(names, ["Default"]);
            assert_eq!(config.profiles[0].program, "");
            assert!(config.profiles[0].args.is_empty());
        }
        assert_eq!(config.default_profile, 0);
    }

    #[test]
    fn profiles_are_read_from_the_config_file() {
        let config = Config::from_toml_str(
            r#"
            default_profile = 1

            [[profiles]]
            name = "Build shell"
            program = "pwsh.exe"
            args = ["-NoLogo"]

            [[profiles]]
            name = "Admin"
            program = "cmd.exe"
            args = ["/K", "echo hi"]
            "#,
        )
        .unwrap();
        assert_eq!(config.profiles.len(), 2);
        assert_eq!(config.profiles[0].name, "Build shell");
        assert_eq!(config.profiles[0].args, ["-NoLogo"]);
        assert_eq!(config.default_profile, 1);
    }

    #[test]
    fn default_profile_is_clamped_to_the_list() {
        let config = Config::from_toml_str("default_profile = 9").unwrap();
        assert_eq!(config.default_profile, 0);
    }

    #[test]
    fn custom_shell_becomes_the_only_profile() {
        let config = Config::from_toml_str("[shell]\nprogram = \"pwsh.exe\"\n").unwrap();
        assert_eq!(config.profiles.len(), 1);
        assert_eq!(config.profiles[0].name, "PowerShell");
        assert_eq!(config.profiles[0].program, "pwsh.exe");
        assert_eq!(config.default_profile, 0);
    }

    #[test]
    fn a_legacy_shell_profile_is_named_after_its_program() {
        assert_eq!(shell_profile_name("powershell.exe"), "Windows PowerShell");
        assert_eq!(shell_profile_name("pwsh.exe"), "PowerShell");
        assert_eq!(shell_profile_name("cmd.exe"), "Command Prompt");
        // A program the helper does not know keeps its own stem.
        assert_eq!(shell_profile_name("nu.exe"), "nu");
        // The lookup ignores case but the name keeps the stem's.
        assert_eq!(shell_profile_name("MyShell.EXE"), "MyShell");
        // A name with no extension is its own stem.
        assert_eq!(shell_profile_name("busybox"), "busybox");
        // No stem at all falls back to the whole string.
        assert_eq!(shell_profile_name(""), "");
    }

    fn save_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "termyx-config-save-{}-{name}.toml",
            std::process::id()
        ))
    }

    #[test]
    fn saving_updates_only_editable_keys_and_preserves_the_rest() {
        let path = save_path("preserve");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            r#"
[font]
family = "Old"
size = 10.0
line_height = 1.0
unrelated = "kept"

[window]
width = 1200
height = 700
padding = 8

[shell]
program = "pwsh.exe"
args = ["-NoLogo"]

[[profiles]]
name = "Dev"
program = "pwsh.exe"

[junk]
keep = true
"#,
        )
        .expect("write the fixture");

        let mut config = Config::from_toml_str(&std::fs::read_to_string(&path).expect("read"))
            .expect("parse the fixture");
        config.font.family = "Fira Code".into();
        config.font.size = 18.0;
        config.font.line_height = 1.25;
        config.font.arabic_family = "Arial".into();
        config.bidi.base_direction = BaseDirection::Rtl;
        config.window.padding = 14;
        config.window.dynamic_title = true;
        config.scrollback.lines = 50_000;
        config.selection.copy_on_select = false;
        config.selection.trim_trailing = false;
        config.theme.background = "#112233".into();
        config.save(&path).expect("save the config");

        let saved = std::fs::read_to_string(&path).expect("read the save");
        let value: toml::Value = toml::from_str(&saved).expect("parse the save");
        assert_eq!(value["font"]["family"].as_str(), Some("Fira Code"));
        assert_eq!(value["font"]["size"].as_float(), Some(18.0));
        assert_eq!(value["font"]["line_height"].as_float(), Some(1.25));
        assert_eq!(value["font"]["arabic_family"].as_str(), Some("Arial"));
        assert_eq!(value["font"]["unrelated"].as_str(), Some("kept"));
        assert_eq!(value["bidi"]["base_direction"].as_str(), Some("rtl"));
        assert_eq!(value["window"]["padding"].as_integer(), Some(14));
        assert_eq!(value["window"]["dynamic_title"].as_bool(), Some(true));
        assert_eq!(
            value["window"]["width"].as_integer(),
            Some(1200),
            "the Settings page must not touch the startup width"
        );
        assert_eq!(value["window"]["height"].as_integer(), Some(700));
        assert_eq!(value["scrollback"]["lines"].as_integer(), Some(50_000));
        assert_eq!(value["selection"]["copy_on_select"].as_bool(), Some(false));
        assert_eq!(value["selection"]["trim_trailing"].as_bool(), Some(false));
        assert_eq!(value["theme"]["background"].as_str(), Some("#112233"));
        assert_eq!(value["shell"]["program"].as_str(), Some("pwsh.exe"));
        assert_eq!(value["shell"]["args"][0].as_str(), Some("-NoLogo"));
        assert_eq!(value["profiles"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["profiles"][0]["name"].as_str(), Some("Dev"));
        assert_eq!(value["junk"]["keep"].as_bool(), Some(true));

        let reloaded = Config::from_toml_str(&saved).expect("reload the save");
        assert_eq!(reloaded.font.size, 18.0);
        assert_eq!(reloaded.bidi.base_direction, BaseDirection::Rtl);
        assert_eq!(reloaded.scrollback.lines, 50_000);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn saving_creates_the_file_and_its_parent_directory() {
        let root =
            std::env::temp_dir().join(format!("termyx-config-save-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("nested").join("config.toml");
        Config::default()
            .validate()
            .save(&path)
            .expect("create the config");
        assert!(path.is_file());
        let reloaded = Config::from_toml_str(&std::fs::read_to_string(&path).expect("read"))
            .expect("parse the new file");
        assert_eq!(reloaded.font.size, 16.0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_parse_error_leaves_the_original_file_untouched() {
        let path = save_path("invalid");
        let original = "[font]\nsize = [oops\n";
        std::fs::write(&path, original).expect("write the invalid fixture");
        let result = Config::default().validate().save(&path);
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(&path).expect("read the unchanged file"),
            original
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_keys_inside_profile_entries_are_ignored() {
        let config = Config::from_toml_str(
            r#"
            [[profiles]]
            name = "Build shell"
            program = "pwsh.exe"
            args = ["-NoLogo"]
            junk = true
            "#,
        )
        .unwrap();
        assert_eq!(config.profiles.len(), 1);
        assert_eq!(config.profiles[0].name, "Build shell");
        assert_eq!(config.profiles[0].program, "pwsh.exe");
        assert_eq!(config.profiles[0].args, ["-NoLogo"]);
    }
}
