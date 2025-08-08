use serenity::builder::{
    CreateActionRow, CreateButton, CreateSelectMenu, CreateSelectMenuKind, CreateSelectMenuOption,
};
use serenity::all::ButtonStyle;
use uuid::Uuid;

/* Main buttons row */
pub fn main_buttons_row(raid_id: Uuid) -> CreateActionRow {
    CreateActionRow::Buttons(vec![
        CreateButton::new(format!("r:j:m:{raid_id}"))
            .label("Join (Main)")
            .style(ButtonStyle::Success),
        CreateButton::new(format!("r:j:a:{raid_id}"))
            .label("Sign Up (Alt)")
            .style(ButtonStyle::Primary),
        CreateButton::new(format!("r:l:{raid_id}"))
            .label("Sign Out (All)")
            .style(ButtonStyle::Danger),
        CreateButton::new(format!("r:la:{raid_id}"))
            .label("Leave (Alts)")
            .style(ButtonStyle::Secondary),
        CreateButton::new(format!("r:mg:{raid_id}"))
            .label("Manage")
            .style(ButtonStyle::Secondary),
    ])
}

/* Ephemeral class select row */
pub fn class_menu_row(raid_id: Uuid) -> CreateActionRow {
    let menu = CreateSelectMenu::new(
        format!("r:pc:{raid_id}"),
        CreateSelectMenuKind::String {
            options: vec![
                CreateSelectMenuOption::new("MSW", "MSW"),
                CreateSelectMenuOption::new("MAG", "MAG"),
                CreateSelectMenuOption::new("ARCH", "ARCH"),
                CreateSelectMenuOption::new("SWORD", "SWORD"),
            ],
        },
    )
        .placeholder("Choose your class")
        .min_values(1)
        .max_values(1);

    CreateActionRow::SelectMenu(menu)
}

/* Ephemeral SP select row (dynamic per class) */
pub fn sp_menu_row(raid_id: Uuid, class: Option<&str>) -> CreateActionRow {
    let opts = match class.map(|s| s.to_ascii_uppercase()) {
        Some(ref c) if c == "MSW" => vec![1,2,3,4,9,10,11],
        _ => (1..=11).collect::<Vec<_>>(),
    };
    let options = opts
        .into_iter()
        .map(|i| {
            let name = format!("SP{}", i);
            CreateSelectMenuOption::new(&name, &name)
        })
        .collect::<Vec<_>>();

    let menu = CreateSelectMenu::new(
        format!("r:ps:{raid_id}"),
        CreateSelectMenuKind::String { options },
    )
        .placeholder("Choose your SP")
        .min_values(1)
        .max_values(1);

    CreateActionRow::SelectMenu(menu)
}

/* Ephemeral confirm/cancel row */
pub fn confirm_row(raid_id: Uuid, main: bool) -> CreateActionRow {
    CreateActionRow::Buttons(vec![
        CreateButton::new(format!("r:ok:{raid_id}"))
            .label(if main { "Confirm (Main)" } else { "Confirm (Alt)" })
            .style(ButtonStyle::Success),
        CreateButton::new(format!("r:x:{raid_id}"))
            .label("Cancel")
            .style(ButtonStyle::Secondary),
    ])
}

/* Owner manage rows built from string options (reserves only / all users) */
pub fn user_select_row(custom_id: String, placeholder: &str, options: Vec<(String, String)>) -> CreateActionRow {
    let menu = CreateSelectMenu::new(
        custom_id,
        CreateSelectMenuKind::String {
            options: options.into_iter().map(|(label, value)| {
                CreateSelectMenuOption::new(label, value)
            }).collect()
        }
    ).placeholder(placeholder).min_values(1).max_values(1);

    CreateActionRow::SelectMenu(menu)
}
pub fn class_menu_row_selected(raid_id: Uuid, selected: Option<&str>) -> CreateActionRow {
    let classes = ["MSW", "MAG", "ARCH", "SWORD"];
    let options = classes
        .into_iter()
        .map(|label| {
            let mut opt = CreateSelectMenuOption::new(label, label);
            if selected.map(|s| s.eq_ignore_ascii_case(label)).unwrap_or(false) {
                opt = opt.default_selection(true);
            }
            opt
        })
        .collect::<Vec<_>>();

    let menu = CreateSelectMenu::new(
        format!("r:pc:{raid_id}"),
        CreateSelectMenuKind::String { options },
    )
        .placeholder("Choose your class")
        .min_values(1)
        .max_values(1);

    CreateActionRow::SelectMenu(menu)
}

pub fn sp_menu_row_selected(raid_id: Uuid, class: Option<&str>, selected_sp: Option<&str>) -> CreateActionRow {
    let sp_list = match class.map(|s| s.to_ascii_uppercase()) {
        Some(ref c) if c == "MSW" => vec![1,2,3,4,9,10,11],
        _ => (1..=11).collect::<Vec<_>>(),
    };

    let options = sp_list
        .into_iter()
        .map(|i| {
            let label = format!("SP{}", i);
            let mut opt = CreateSelectMenuOption::new(&label, &label);
            if selected_sp
                .as_deref()
                .map(|v| v.eq_ignore_ascii_case(&label))
                .unwrap_or(false)
            {
                opt = opt.default_selection(true);
            }
            opt
        })
        .collect::<Vec<_>>();

    let menu = CreateSelectMenu::new(
        format!("r:ps:{raid_id}"),
        CreateSelectMenuKind::String { options },
    )
        .placeholder("Choose your SP")
        .min_values(1)
        .max_values(1);

    CreateActionRow::SelectMenu(menu)
}
