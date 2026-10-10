use super::common::*;
use crate::{PContext, PError};

#[doc = "Resume playback."]
#[poise::command(slash_command, guild_only)]
pub async fn resume(ctx: PContext<'_>) -> Result<(), PError> {
    let player = state(&ctx).await?;
    match player.lock().await.resume().await {
        Ok(()) => reply(ctx, "▶️ Resumed.", false).await,
        Err(error) if error == "Already playing" => reply(ctx, "Already playing.", false).await,
        Err(error) => reply(ctx, error, false).await,
    }
}
