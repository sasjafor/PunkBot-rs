use super::common::*;
use crate::{PContext, PError};

#[doc = "Join your voice channel."]
#[poise::command(slash_command, guild_only)]
pub async fn summon(ctx: PContext<'_>) -> Result<(), PError> {
    ctx.defer().await?;
    let channel_id =
        author_channel(&ctx).ok_or_else(|| command_error("Join a voice channel first".into()))?;
    let manager = songbird::get(ctx.serenity_context())
        .await
        .ok_or_else(|| command_error("Voice manager is unavailable".into()))?
        .clone();
    manager
        .join(guild_id(&ctx)?, channel_id)
        .await
        .map_err(|e| command_error(e.to_string()))?;
    let player = state(&ctx).await?;
    player.lock().await.channel_id = Some(channel_id);
    reply(ctx, format!("👍 Joined <#{}>.", channel_id.get()), false).await
}
