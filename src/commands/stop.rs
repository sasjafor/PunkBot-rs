use super::common::*;
use crate::{PContext, PError};

#[doc = "Stop playback and clear the queue."]
#[poise::command(slash_command, guild_only)]
pub async fn stop(ctx: PContext<'_>) -> Result<(), PError> {
    let player = state(&ctx).await?;
    player
        .lock()
        .await
        .stop(&ctx)
        .await
        .map_err(command_error)?;
    reply(ctx, "⏹️ Stopped playback.", false).await
}
