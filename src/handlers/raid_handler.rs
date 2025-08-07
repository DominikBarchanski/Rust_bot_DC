use serenity::all::{
    Context, Interaction, Ready, CreateInteractionResponse, CreateInteractionResponseMessage,
};
use serenity::async_trait;
use serenity::prelude::EventHandler;
use tokio_postgres::Client as PgClient;

use crate::commands::raid::{register, handle as handle_slash};
use crate::modal::raid_modal::{handle_raid_modal_button, handle_modal_submit};

pub struct Handler {
    pub db_client: PgClient,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);

        // Register slash command
        if let Err(err) = register(&ctx).await {
            eprintln!("Error registering /raid: {:?}", err);
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(cmd) => {
                if cmd.data.name == "raid" {
                    if let Err(err) = handle_slash(&ctx, &cmd).await {
                        eprintln!("Error handling /raid: {:?}", err);
                    }
                }
            }
            Interaction::Component(component) => {
                match component.data.custom_id.as_str() {
                    "open_raid_modal" => {
                        if let Err(err) = handle_raid_modal_button(&ctx, &component).await {
                            eprintln!("Error showing raid modal: {:?}", err);
                        }
                    }
                    "finish" => {
                        // Handle finish button if needed
                        let response = CreateInteractionResponseMessage::new()
                            .content("✅ Raid setup completed!")
                            .components(vec![]);
                        let _ = component.create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(response)).await;
                    }
                    _ => {}
                }
            }
            Interaction::Modal(modal) => {
                if modal.data.custom_id == "raid_modal" {
                    if let Err(err) = handle_modal_submit(&ctx, &modal).await {
                        eprintln!("Error handling raid_modal submit: {:?}", err);
                    }
                }
            }
            _ => {}
        }
    }
}