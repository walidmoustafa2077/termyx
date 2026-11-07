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

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let config = config::Config::load(None)?;
    tracing::debug!("loaded config: {config:?}");
    Ok(())
}
