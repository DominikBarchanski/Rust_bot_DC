pub mod ping;
pub mod raid;

use serenity::prelude::*;

pub async fn register_commands(ctx: &Context) -> anyhow::Result<()> {
    raid::register(ctx).await?;
    Ok(())
}