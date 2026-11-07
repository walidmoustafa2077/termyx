mod app;
mod config;
mod event;
mod fonts;
mod input;
mod pty;
mod render;
mod selection;
mod term;
mod theme;
mod visual;

use anyhow::Context;
use std::path::PathBuf;
use winit::event_loop::{ControlFlow, EventLoop};

fn config_path_from_args() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            return args.next().map(PathBuf::from);
        }
    }
    None
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let config = config::Config::load(config_path_from_args().as_deref())
        .context("loading configuration")?;
    let event_loop = EventLoop::<event::UserEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let mut app = app::App::new(config, proxy);
    event_loop.run_app(&mut app)?;
    Ok(())
}
