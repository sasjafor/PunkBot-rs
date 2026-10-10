use super::common::*;
use crate::{PContext, PError};

#[doc = "Disconnect from the voice channel."]
#[poise::command(slash_command, guild_only)]
pub async fn disconnect(ctx: PContext<'_>) -> Result<(), PError> {
    let player = state(&ctx).await?;
    player
        .lock()
        .await
        .disconnect(&ctx)
        .await
        .map_err(command_error)?;
    reply(ctx, "Successfully disconnected.", false).await
}
