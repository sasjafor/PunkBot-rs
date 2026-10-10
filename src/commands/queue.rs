use super::common::*;
use crate::{PContext, PError};

#[doc = "Show songs in the queue."]
#[poise::command(slash_command, guild_only)]
pub async fn queue(
    ctx: PContext<'_>,
    #[description = "1-based page index"]
    #[min = 1]
    index: Option<usize>,
) -> Result<(), PError> {
    let mut page = index.unwrap_or(1).max(1);
    let player = state(&ctx).await?;
    let (embed, pages) = queue_embed(&ctx, &player, page).await?;
    if page > pages {
        return reply(
            ctx,
            format!("Invalid queue page. Choose a page between 1 and {pages}."),
            false,
        )
        .await;
    }
    let message = ctx
        .send(
            CreateReply::default()
                .embed(embed)
                .components(queue_buttons(page, pages)),
        )
        .await?
        .into_message()
        .await?;
    let mut collector = message
        .await_component_interactions(ctx)
        .timeout(Duration::from_secs(300))
        .stream();
    while let Some(interaction) = collector.next().await {
        if interaction.user.id != ctx.author().id {
            let deny = CreateInteractionResponseMessage::new()
                .content("Only the person who opened this queue can change pages.")
                .ephemeral(true);
            let _ = interaction
                .create_response(ctx, CreateInteractionResponse::Message(deny))
                .await;
            continue;
        }
        match interaction.data.custom_id.as_str() {
            "punkbot:queue:prev" => page = page.saturating_sub(1).max(1),
            "punkbot:queue:next" => page = (page + 1).min(pages),
            _ => continue,
        }
        let (embed, _) = queue_embed(&ctx, &player, page).await?;
        let response = CreateInteractionResponseMessage::default()
            .embed(embed)
            .components(queue_buttons(page, pages));
        interaction
            .create_response(ctx, CreateInteractionResponse::UpdateMessage(response))
            .await?;
    }
    let _ = message.delete(ctx).await;
    Ok(())
}

async fn queue_embed(
    ctx: &PContext<'_>,
    player: &Arc<Mutex<Player>>,
    page: usize,
) -> Result<(serenity::CreateEmbed, usize), PError> {
    let p = player.lock().await;
    let Some(current) = p.items.front() else {
        return Err(command_error("Nothing is playing in this server".into()));
    };
    let waiting = p.queue_len();
    let pages = waiting.div_ceil(10).max(1);
    let page = page.min(pages).max(1);
    let start = 1 + (page - 1) * 10;
    let mut lines = vec![format!(
        "**Now Playing:** [{}]({}) · {} · requested by <@{}>",
        current.video.title,
        current.video.url,
        format_duration(current.video.duration.unwrap_or(0)),
        current.requester_id
    )];
    if page == 1 && waiting > 0 {
        lines.push("⬇️ **Up Next** ⬇️".to_owned());
    }
    for (offset, item) in p.items.iter().skip(start).take(10).enumerate() {
        lines.push(format!(
            "{}. [{}]({}) · {} · requested by <@{}>",
            start + offset,
            item.video.title,
            item.video.url,
            format_duration(item.video.duration.unwrap_or(0)),
            item.requester_id
        ));
    }
    let total_time = p
        .items
        .iter()
        .skip(1)
        .map(|item| item.video.duration.unwrap_or(0))
        .sum::<u64>();
    if waiting > 0 {
        lines.push(format!(
            "\n**{waiting} songs waiting · {} total length · page {page}/{pages}**",
            format_duration(total_time)
        ));
    }
    let embed = serenity::CreateEmbed::new()
        .title(format!(
            "Queue for {}",
            ctx.guild().map(|g| g.name.clone()).unwrap_or_default()
        ))
        .description(lines.join("\n\n"));
    Ok((embed, pages))
}

fn queue_buttons(page: usize, pages: usize) -> Vec<CreateActionRow> {
    if pages <= 1 {
        return Vec::new();
    }
    vec![CreateActionRow::Buttons(vec![
        CreateButton::new("punkbot:queue:prev")
            .label("Prev")
            .style(ButtonStyle::Secondary)
            .disabled(page <= 1),
        CreateButton::new("punkbot:queue:next")
            .label("Next")
            .style(ButtonStyle::Secondary)
            .disabled(page >= pages),
    ])]
}
