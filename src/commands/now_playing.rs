use super::common::*;
use crate::{PContext, PError};

#[doc = "Show the currently playing song and its progress."]
#[poise::command(slash_command, guild_only, rename = "now-playing")]
pub async fn now_playing(ctx: PContext<'_>) -> Result<(), PError> {
    let player = state(&ctx).await?;
    let player = player.lock().await;
    let Some(current) = player.items.front() else {
        return reply(ctx, "Nothing is playing in this server.", false).await;
    };
    let position = player.current_position().await;
    let total = current.video.duration.unwrap_or(0);
    let mut embed = serenity::CreateEmbed::new()
        .title(current.video.title.clone())
        .url(current.video.url.clone())
        .description(format!(
            "`{}`\n`{} / {}`\nRequested by <@{}>",
            crate::util::progress_bar(position, total),
            format_duration(position),
            format_duration(total),
            current.requester_id
        ))
        .author(
            serenity::CreateEmbedAuthor::new("Now Playing ♪")
                .icon_url(current.requester_avatar.clone()),
        )
        .colour(serenity::Colour::from_rgb(0, 86, 191));
    if let Some(thumbnail) = current.video.thumbnail.as_ref() {
        embed = embed.thumbnail(thumbnail);
    }
    ctx.send(CreateReply::default().embed(embed)).await?;
    Ok(())
}
