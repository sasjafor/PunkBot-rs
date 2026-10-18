use super::common::*;
use crate::{PContext, PError};

#[doc = "Skip the current song."]
#[poise::command(slash_command, guild_only)]
pub async fn skip(ctx: PContext<'_>) -> Result<(), PError> {
    let player = state(&ctx).await?;
    let (title, next_title) = player
        .lock()
        .await
        .skip(&ctx)
        .await
        .map_err(command_error)?;
    let mut embed = serenity::CreateEmbed::new()
        .title("⏭️ Song skipped")
        .field("Skipped", title, false)
        .colour(serenity::Colour::from_rgb(0, 86, 191));
    embed = match next_title {
        Some(next_title) => embed.field("Now playing", next_title, false),
        None => embed.description("The queue is empty."),
    };
    ctx.send(CreateReply::default().embed(embed)).await?;
    Ok(())
}
