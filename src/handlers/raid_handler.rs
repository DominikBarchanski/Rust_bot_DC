use serenity::async_trait;
use serenity::model::prelude::*;
use serenity::prelude::*;

pub struct Handler {
    pub pool: sqlx::PgPool,
}

impl Handler {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
        // Register commands using your existing registration
        if let Err(e) = crate::commands::raid::register(&ctx).await {
            eprintln!("Failed to register raid command: {}", e);
        }
        if let Err(e) = crate::commands::ping::register(&ctx).await {
            eprintln!("Failed to register ping command: {}", e);
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(cmd) => {
                match cmd.data.name.as_str() {
                    "ping" => {
                        let _ = crate::commands::ping::run(&ctx, &cmd).await;
                    }
                    "raid" => {
                        // Pass the database pool to your raid handler
                        println!("Raid command triggered");
                        let _ = crate::commands::raid::handle_with_db(&ctx, &cmd, &self.pool).await;
                    }
                    _ => {}
                }
            }
            Interaction::Component(component) => {
                // Handle your modal button interactions
                match component.data.custom_id.as_str() {
                    "open_raid_modal" => {
                        let _ = crate::commands::raid_modal::handle_raid_modal_button(&ctx, &component).await;
                    }
                    "finish" => {
                        // Handle finish button if needed
                    }
                    _ => {}
                }
            }
            Interaction::Modal(modal) => {
                // Handle modal submissions
                match modal.data.custom_id.as_str() {
                    "raid_modal" => {
                        let _ = crate::commands::raid_modal::handle_modal_submit(&ctx, &modal).await;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
