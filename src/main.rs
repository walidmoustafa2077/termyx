#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bidi;
mod config;
mod event;
mod find;
mod fonts;
mod icon;
mod input;
mod page;
mod palette;
mod platform;
mod profiles;
mod pty;
mod render;
mod scrollbar;
mod selection;
mod session;
mod settings;
mod shape;
mod spellcheck;
mod term;
mod theme;
mod ui;
mod visual;

use std::path::PathBuf;
use tracing_subscriber::filter::LevelFilter;
use winit::event_loop::{ControlFlow, EventLoop};

/// Options the window takes from the command line.
///
/// Explorer's context menu launches the executable with the selected folder as
/// a bare argument, so a positional path is treated exactly like `--directory`.
#[derive(Debug, Default, PartialEq, Eq)]
struct LaunchOptions {
    /// The config file to load and save, from `--config <path>`.
    config_path: Option<PathBuf>,
    /// The directory the shell starts in.
    directory: Option<PathBuf>,
}

fn launch_options_from_args<I>(args: I) -> anyhow::Result<LaunchOptions>
where
    I: IntoIterator<Item = String>,
{
    let mut options = LaunchOptions::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                let Some(path) = args.next() else {
                    anyhow::bail!("--config requires a path");
                };
                options.config_path = Some(PathBuf::from(path));
            }
            "--directory" | "--working-directory" => {
                let Some(path) = args.next() else {
                    anyhow::bail!("{arg} requires a path");
                };
                options.directory = Some(PathBuf::from(path));
            }
            _ if arg.starts_with('-') => {
                tracing::warn!(argument = %arg, "ignoring an unknown option");
            }
            _ => {
                if options.directory.is_none() {
                    options.directory = Some(PathBuf::from(arg));
                } else {
                    tracing::warn!(
                        argument = %arg,
                        "ignoring an extra path; only one start directory is used"
                    );
                }
            }
        }
    }
    Ok(options)
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(LevelFilter::WARN.into())
                .with_env_var("TERMYX_LOG")
                .from_env_lossy(),
        )
        .with_target(false)
        .init();

    let options = launch_options_from_args(std::env::args().skip(1))?;
    // Settings saves to this path after the working directory may have moved to
    // the folder Explorer selected, so pin a relative `--config` now.
    let requested_path = options
        .config_path
        .map(|path| std::path::absolute(&path).unwrap_or(path));
    let config = match config::Config::load(requested_path.as_deref()) {
        Ok(config) => config,
        Err(err) => {
            tracing::error!("{err:#}");
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Termyx")
                .set_description(format!(
                    "Configuration could not be loaded:\n{err:#}\n\nUsing defaults."
                ))
                .show();
            config::Config::default().validate()
        }
    };
    // A shell started from Explorer's context menu must open in the selected
    // folder. The PTY crate otherwise starts children in the user's home, so
    // the directory is validated here and passed down to `CommandBuilder::cwd`.
    // A missing path is recoverable: keep the shell's default directory.
    let start_directory = options.directory.and_then(|directory| {
        if directory.is_dir() {
            Some(directory)
        } else {
            tracing::warn!(
                path = %directory.display(),
                "start directory does not exist; using the shell default"
            );
            None
        }
    });
    let event_loop = EventLoop::<event::UserEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    // The Settings page and its live poll need the same path that was loaded,
    // including an explicit `--config` override.
    let config_path = requested_path.unwrap_or_else(config::default_config_path);
    let mut app = app::App::new(config, config_path, start_directory, proxy);
    event_loop.run_app(&mut app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> anyhow::Result<LaunchOptions> {
        launch_options_from_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn a_bare_path_is_the_start_directory() {
        let options = parse(&[r"D:\Projects\demo"]).unwrap();
        assert_eq!(options.directory, Some(PathBuf::from(r"D:\Projects\demo")));
        assert_eq!(options.config_path, None);
    }

    #[test]
    fn the_start_directory_can_be_named_explicitly() {
        let options = parse(&["--directory", r"D:\Projects\demo"]).unwrap();
        assert_eq!(options.directory, Some(PathBuf::from(r"D:\Projects\demo")));

        let options = parse(&["--working-directory", r"D:\Projects\demo"]).unwrap();
        assert_eq!(options.directory, Some(PathBuf::from(r"D:\Projects\demo")));
    }

    #[test]
    fn config_and_start_directory_can_be_combined() {
        let options = parse(&["--config", "custom.toml", r"D:\Projects\demo"]).unwrap();
        assert_eq!(options.config_path, Some(PathBuf::from("custom.toml")));
        assert_eq!(options.directory, Some(PathBuf::from(r"D:\Projects\demo")));
    }

    #[test]
    fn only_the_first_bare_path_is_a_start_directory() {
        let options = parse(&[r"D:\one", r"D:\two"]).unwrap();
        assert_eq!(options.directory, Some(PathBuf::from(r"D:\one")));
    }

    #[test]
    fn a_missing_option_value_is_an_error() {
        assert!(parse(&["--config"]).is_err());
        assert!(parse(&["--directory"]).is_err());
    }
}
