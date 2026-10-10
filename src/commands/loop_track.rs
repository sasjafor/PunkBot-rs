use super::common::*;
use crate::{PContext, PError};

#[doc = "Toggle repeat for the current track."]
#[poise::command(slash_command, guild_only, rename = "loop")]
pub async fn loop_track(ctx: PContext<'_>) -> Result<(), PError> {
    let player = state(&ctx).await?;
    let enabled = player.lock().await.set_loop().await;
    reply(
        ctx,
        if enabled {
            "🔁 Loop enabled."
        } else {
            "🔁 Loop disabled."
        },
        false,
    )
    .await
}
