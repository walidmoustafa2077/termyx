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
mod platform;
mod profiles;
mod pty;
mod render;
mod scrollbar;
mod selection;
mod session;
mod settings;
mod shape;
mod term;
mod theme;
mod ui;
mod visual;

use std::path::PathBuf;
use tracing_subscriber::filter::LevelFilter;
use winit::event_loop::{ControlFlow, EventLoop};

fn config_path_from_args() -> anyhow::Result<Option<PathBuf>> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            let Some(path) = args.next() else {
                anyhow::bail!("--config requires a path");
            };
            return Ok(Some(PathBuf::from(path)));
        }
    }
    Ok(None)
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

    let requested_path = config_path_from_args()?;
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
    let event_loop = EventLoop::<event::UserEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    // The Settings page and its live poll need the same path that was loaded,
    // including an explicit `--config` override.
    let config_path = requested_path.unwrap_or_else(config::default_config_path);
    let mut app = app::App::new(config, config_path, proxy);
    event_loop.run_app(&mut app)?;
    Ok(())
}
