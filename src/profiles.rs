//! The flat list of shells offered by the `+` and `⌄` menus.

use crate::config::{Config, ShellConfig};

/// A shell the user can open in a new tab.
// Phase 4 Tasks 2+ build the `+` and `⌄` menus from this.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: String,
    pub shell: ShellConfig,
}

/// Profiles in menu order.
///
/// A validated config always has at least one profile: `Config::validate`
/// fills an empty list with the configured `[shell]` (named "Default") or, when
/// `[shell]` is unset, with the Windows pair. The `[shell]` fallback below only
/// applies to a config built or edited without validation.
// Phase 4 Tasks 2+ build the `+` and `⌄` menus from this.
#[allow(dead_code)]
pub fn profiles(config: &Config) -> Vec<Profile> {
    let mut profiles: Vec<Profile> = config
        .profiles
        .iter()
        .map(|profile| Profile {
            name: profile.name.clone(),
            shell: ShellConfig {
                program: profile.program.clone(),
                args: profile.args.clone(),
            },
        })
        .collect();
    if profiles.is_empty() {
        // Unvalidated configs can still have an empty list; keep their shell.
        profiles.push(Profile {
            name: config.shell.program.clone(),
            shell: config.shell.clone(),
        });
    }
    profiles
}

/// Index of the profile a bare `+` opens.
// Phase 4 Tasks 2+ build the `+` and `⌄` menus from this.
#[allow(dead_code)]
pub fn default_profile_index(config: &Config) -> usize {
    if config.default_profile < config.profiles.len() {
        config.default_profile
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_map_each_entry_to_a_name_and_a_shell() {
        let config = Config::from_toml_str(
            r#"
            [[profiles]]
            name = "Build shell"
            program = "pwsh.exe"
            args = ["-NoLogo"]
            "#,
        )
        .unwrap();
        assert_eq!(
            profiles(&config),
            [Profile {
                name: "Build shell".to_string(),
                shell: ShellConfig {
                    program: "pwsh.exe".to_string(),
                    args: vec!["-NoLogo".to_string()],
                },
            }]
        );
    }

    #[test]
    fn default_profile_index_falls_back_to_zero_past_the_end() {
        let mut config = Config::from_toml_str(
            r#"
            [[profiles]]
            name = "Build shell"
            program = "pwsh.exe"
            "#,
        )
        .unwrap();
        config.default_profile = 5;
        assert_eq!(default_profile_index(&config), 0);
    }
}
