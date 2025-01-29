// TODO: make urls and other optional player parameteres truly optional

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

fn main() -> FumResult<()> {
    let config = cli::run()?;

    if config.authorize {
        youtube::authorize();
        return Ok(());
    }

    let mut fum = Fum::new(&config)?;

    fum.run()?;

    Ok(())
}
