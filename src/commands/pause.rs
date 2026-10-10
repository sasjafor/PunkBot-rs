use super::common::*;
use crate::{PContext, PError};

#[doc = "Pause playback."]
#[poise::command(slash_command, guild_only)]
pub async fn pause(ctx: PContext<'_>) -> Result<(), PError> {
    let player = state(&ctx).await?;
    match player.lock().await.pause().await {
        Ok(()) => reply(ctx, "⏸️ Paused.", false).await,
        Err(error) if error == "Already paused" => reply(ctx, "Already paused.", false).await,
        Err(error) => reply(ctx, error, false).await,
    }
}
