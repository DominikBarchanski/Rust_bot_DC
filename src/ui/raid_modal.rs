use serenity::all::{
    Context, CommandInteraction, CreateInteractionResponse, CreateInteractionResponseMessage,
    CreateActionRow, CreateButton, ButtonStyle, CreateModal, CreateInputText, InputTextStyle,
    ComponentInteraction, MessageId, ChannelId,
};
use std::collections::HashMap;
use std::sync::Mutex;
use lazy_static::lazy_static;

// Store user_id -> (channel_id, message_id)
lazy_static! {
    static ref MSG_MAP: Mutex<HashMap<u64, (ChannelId, MessageId)>> = Mutex::new(HashMap::new());
}

pub async fn show_raid_modal(
    ctx: &Context,
    cmd: &CommandInteraction,
    _raid_name: &str,
    _player_count: i64,
) -> serenity::Result<()> {
    let buttons = CreateActionRow::Buttons(vec![
        CreateButton::new("open_raid_modal")
            .label("Open Raid Modal")
            .style(ButtonStyle::Primary),
    ]);

    let response = CreateInteractionResponseMessage::new()
        .content("Click the button below to log a raid:")
        .components(vec![buttons]);

    // Initial response
    cmd.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await?;

    // Fetch message (for later editing)
    let msg = cmd.get_response(&ctx.http).await?;
    // Store (user_id, channel_id, message_id)
    MSG_MAP.lock().unwrap().insert(
        cmd.user.id.get(),
        (cmd.channel_id, msg.id),
    );

    Ok(())
}

/// Handle a component interaction for the "Open Raid Modal" button
pub async fn handle_raid_modal_button(
    ctx: &Context,
    component: &ComponentInteraction,
) -> serenity::Result<()> {
    // Build the modal
    let modal = CreateModal::new("raid_modal", "Log Raid Details")
        .components(vec![
            CreateActionRow::InputText(
                CreateInputText::new(InputTextStyle::Paragraph, "description", "Description")
                    .required(true)
            ),
            CreateActionRow::InputText(
                CreateInputText::new(InputTextStyle::Short, "difficulty", "Difficulty")
                    .placeholder("Easy/Normal/Hard?")
                    .required(true)
            ),
            CreateActionRow::InputText(
                CreateInputText::new(InputTextStyle::Short, "notes", "Notes")
                    .required(false)
            ),
        ]);

    component.create_response(
        &ctx.http,
        CreateInteractionResponse::Modal(modal),
    ).await?;
    Ok(())
}

/// After the user submits the modal, echo their values and edit the original message.
pub async fn handle_modal_submit(
    ctx: &Context,
    modal: &serenity::all::ModalInteraction,
) -> serenity::Result<()> {
    use serenity::all::ActionRowComponent;

    fn get_component_value(component: &ActionRowComponent) -> String {
        match component {
            ActionRowComponent::InputText(input) => input.value.clone().unwrap_or_default(),
            _ => String::new(),
        }
    }

    let description = modal
        .data
        .components
        .get(0)
        .and_then(|row| row.components.get(0))
        .map(get_component_value)
        .unwrap_or_default();

    let difficulty = modal
        .data
        .components
        .get(1)
        .and_then(|row| row.components.get(0))
        .map(get_component_value)
        .unwrap_or_default();

    let notes = modal
        .data
        .components
        .get(2)
        .and_then(|row| row.components.get(0))
        .map(get_component_value)
        .unwrap_or_default();

    // Try to edit the original message (if tracked)
    let message_info = {
        MSG_MAP.lock().unwrap().get(&modal.user.id.get()).copied()
    }; // MutexGuard is dropped here

    if let Some((channel_id, message_id)) = message_info {
        // Add a new row with an "Add Another Raid" button
        let new_buttons = CreateActionRow::Buttons(vec![
            CreateButton::new("open_raid_modal")
                .label("Add Another Raid")
                .style(ButtonStyle::Primary),
            CreateButton::new("finish")
                .label("Finish")
                .style(ButtonStyle::Danger),
        ]);

        let edit = serenity::all::EditMessage::new()
            .content(format!(
                "✅ Raid logged:\n• Desc: `{}`\n• Difficulty: `{}`\n• Notes: `{}`",
                description, difficulty, notes
            ))
            .components(vec![new_buttons]);

        ctx.http.edit_message(channel_id, message_id, &edit, vec![]).await?;
    }

    // Ephemeral response to acknowledge modal
    let response = CreateInteractionResponseMessage::new()
        .content("Raid saved! (original message updated)")
        .ephemeral(true);

    modal.create_response(
        &ctx.http,
        CreateInteractionResponse::Message(response),
    ).await?;

    Ok(())
}