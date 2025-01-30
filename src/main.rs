mod action;
mod cli;
mod config;
mod fum;
mod meta;
mod regexes;
mod state;
mod text;
mod ui;
mod utils;
mod widget;
mod youtube;

use fum::{Fum, FumResult};

use tracing::level_filters::LevelFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

fn main() -> FumResult<()> {
    let config = cli::run()?;

    if config.authorize {
        youtube::authorize();
        return Ok(());
    }

    let (writer, _guard);
    if config.log {
        let config_path = expanduser::expanduser("~/.config/fum").unwrap();

        if !std::fs::exists(&config_path).expect("failed to look up config directory existance") {
            std::fs::create_dir_all(&config_path).expect("failed to create config directory");
        }

        let log_path = config_path.join("logs");
        let _ = std::fs::File::create(&log_path).expect("failed to create log file");

        (writer, _guard) = tracing_appender::non_blocking(
            std::fs::File::options()
                .append(true)
                .open(&log_path)
                .unwrap(),
        );

        let filter = EnvFilter::builder()
            .with_default_directive(LevelFilter::TRACE.into())
            .from_env_lossy();

        tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(writer)
                    .pretty()
                    .compact(),
            )
            .with(filter)
            .init();
    }

    let mut fum = Fum::new(&config)?;

    fum.run()?;

    Ok(())
}
