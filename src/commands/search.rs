use super::common::*;
use crate::{PContext, PError};

#[doc = "Search YouTube and select a result to play."]
#[poise::command(slash_command, guild_only)]
pub async fn search(
    ctx: PContext<'_>,
    #[description = "YouTube search text"] query: String,
    #[description = "How many results to show"]
    #[min = 0]
    #[max = 25]
    count: Option<usize>,
) -> Result<(), PError> {
    ctx.defer().await?;
    if author_channel(&ctx).is_none() {
        return reply(ctx, "Join a voice channel first.", false).await;
    }
    let results = ctx
        .data()
        .youtube
        .search(&query, count.unwrap_or(5).clamp(1, 25))
        .await
        .map_err(command_error)?;
    if results.is_empty() {
        return reply(ctx, "❌ No matches.", false).await;
    }
    let options = results
        .iter()
        .map(|item| {
            let label = item.title.chars().take(100).collect::<String>();
            let channel = item
                .channel
                .as_deref()
                .unwrap_or("Unknown channel")
                .chars()
                .take(100)
                .collect::<String>();
            CreateSelectMenuOption::new(label, item.id.clone()).description(channel)
        })
        .collect();
    let menu = CreateSelectMenu::new("punkbot:search", CreateSelectMenuKind::String { options })
        .placeholder("Choose a video to play")
        .min_values(1)
        .max_values(1);
    let embed = serenity::CreateEmbed::new()
        .title("YouTube search results")
        .description("Select a result to add it to the player.");
    let message = ctx
        .send(
            CreateReply::default()
                .embed(embed)
                .components(vec![CreateActionRow::SelectMenu(menu)]),
        )
        .await?
        .into_message()
        .await?;
    let mut collector = message
        .await_component_interactions(ctx)
        .timeout(Duration::from_secs(120))
        .stream();
    let mut selected = false;
    while let Some(interaction) = collector.next().await {
        if interaction.user.id != ctx.author().id {
            let deny = CreateInteractionResponseMessage::new()
                .content("Only the person who ran this search can select a result.")
                .ephemeral(true);
            let _ = interaction
                .create_response(ctx, CreateInteractionResponse::Message(deny))
                .await;
            continue;
        }
        let Some(id) = (match &interaction.data.kind {
            ComponentInteractionDataKind::StringSelect { values } => values.first().cloned(),
            _ => None,
        }) else {
            continue;
        };
        let Some(channel_id) = author_channel(&ctx) else {
            let response = CreateInteractionResponseMessage::default()
                .content("Join a voice channel first.")
                .components(Vec::new());
            interaction
                .create_response(ctx, CreateInteractionResponse::UpdateMessage(response))
                .await?;
            break;
        };
        let video = ctx.data().youtube.video(&id).await.map_err(command_error)?;
        if video.age_restricted && ctx.data().media.cookies.is_none() {
            let response = CreateInteractionResponseMessage::default()
                .content("This video is age restricted. Configure YOUTUBE_COOKIES_FILE to play it.")
                .components(Vec::new());
            interaction
                .create_response(ctx, CreateInteractionResponse::UpdateMessage(response))
                .await?;
            break;
        }
        let player = state(&ctx).await?;
        let item = queued(&ctx, video.clone(), 0);
        let position = Player::enqueue(&ctx, player.clone(), item.clone(), channel_id, false)
            .await
            .map_err(command_error)?;
        ctx.data().youtube.remember_search_choice(&query, &id).await;
        let embed = {
            let player = player.lock().await;
            playback_embed(&item, position, &player).await
        };
        let response = CreateInteractionResponseMessage::default()
            .embed(embed)
            .components(Vec::new());
        interaction
            .create_response(ctx, CreateInteractionResponse::UpdateMessage(response))
            .await?;
        selected = true;
        break;
    }
    if !selected {
        let _ = message.delete(ctx).await;
    }
    Ok(())
}
