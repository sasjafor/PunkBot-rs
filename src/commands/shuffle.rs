use super::common::*;
use crate::{PContext, PError};

#[doc = "Shuffle waiting tracks."]
#[poise::command(slash_command, guild_only)]
pub async fn shuffle(ctx: PContext<'_>) -> Result<(), PError> {
    let player = state(&ctx).await?;
    player
        .lock()
        .await
        .shuffle_waiting(&ctx)
        .await
        .map_err(command_error)?;
    reply(ctx, "Shuffled queue 👌", false).await
}
