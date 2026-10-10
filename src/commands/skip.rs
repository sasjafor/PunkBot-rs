use super::common::*;
use crate::{PContext, PError};

#[doc = "Skip the current song."]
#[poise::command(slash_command, guild_only)]
pub async fn skip(ctx: PContext<'_>) -> Result<(), PError> {
    let player = state(&ctx).await?;
    let title = player
        .lock()
        .await
        .skip(&ctx)
        .await
        .map_err(command_error)?;
    reply(ctx, format!("⏭️ Skipped **{title}**."), false).await
}
