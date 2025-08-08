use serenity::builder::{CreateInteractionResponse, CreateInteractionResponseMessage};
use serenity::model::application::CommandInteraction;
use serenity::prelude::*;

/// Handle `/ping` invocations.
pub async fn run(
    ctx: &Context,
    command: &CommandInteraction,
) -> anyhow::Result<()> {
    let data = CreateInteractionResponseMessage::new().content("Pong!");
    let builder = CreateInteractionResponse::Message(data);

    command.create_response(&ctx.http, builder).await?;
    Ok(())
}