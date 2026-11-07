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

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();
}
