pub mod raid;

use serenity::prelude::Context;

pub async fn register_commands(ctx: &Context) -> anyhow::Result<()> {
    raid::register(ctx).await?;
    raid::register_kick(ctx).await?;
    raid::register_transfer(ctx).await?;
    raid::register_role_add(ctx).await?;
    raid::register_all_raid_list(ctx).await?;
    Ok(())
}
