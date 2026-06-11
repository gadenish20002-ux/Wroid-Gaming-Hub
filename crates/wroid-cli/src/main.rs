mod backend;
mod cli;
mod commands;
mod device;
mod interactive;
mod output;
mod registry;
mod scaling;

#[cfg(test)]
mod test_support;

use anyhow::Result;
use clap::Parser;

use crate::backend::CommandInputExecutor;
use crate::cli::Cli;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let input_executor = CommandInputExecutor;

    commands::run(cli, &input_executor)
}
