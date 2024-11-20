use clap::Parser;

use crate::{build_project, CmdExecutor};

#[derive(Debug, Parser)]
pub struct BuildOpts {}

impl CmdExecutor for BuildOpts {
    async fn execute(self) -> anyhow::Result<()> {
        let cur_dir = std::env::current_dir()?.display().to_string();
        let filename = build_project(&cur_dir)?;
        eprintln!("Built project at {}", filename);
        Ok(())
    }
}
