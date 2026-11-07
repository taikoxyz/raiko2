//! Offline synthetic proposal producer; fixture feature only.
use anyhow::Result;
use clap::Parser;
fn main() -> Result<()> {
    let result = zkgas::fixture::run(&zkgas::fixture::FixtureArgs::parse())?;
    println!("{result}");
    Ok(())
}
