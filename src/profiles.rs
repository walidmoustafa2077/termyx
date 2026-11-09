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

/// Profiles in menu order, with the configured shell as the last resort.
///
/// The `[shell]` section stays authoritative for the first tab and for
/// `default_profile` pointing past the end of the list, so a config that only
/// knows about `[shell]` keeps working.
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
        profiles.push(Profile {
            name: config.shell.program.clone(),
            shell: config.shell.clone(),
        });
    }
    profiles
}

/// Index of the profile a bare `+` opens.
#[allow(dead_code)]
pub fn default_profile_index(config: &Config) -> usize {
    if config.default_profile < config.profiles.len() {
        config.default_profile
    } else {
        0
    }
}
