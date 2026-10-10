use super::common::*;
use crate::{PContext, PError};

#[doc = "Clear waiting tracks from the queue."]
#[poise::command(slash_command, guild_only)]
pub async fn clear(ctx: PContext<'_>) -> Result<(), PError> {
    let player = state(&ctx).await?;
    player
        .lock()
        .await
        .clear_waiting(&ctx)
        .await
        .map_err(command_error)?;
    reply(ctx, "💥 Cleared the song queue.", false).await
}
