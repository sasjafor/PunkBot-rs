use super::common::*;
use crate::{PContext, PError};

#[doc = "Set playback volume."]
#[poise::command(slash_command, guild_only)]
pub async fn volume(
    ctx: PContext<'_>,
    #[description = "Volume multiplier"]
    #[min = 0.0]
    value: f32,
) -> Result<(), PError> {
    if !value.is_finite() || value < 0.0 {
        return reply(ctx, "Volume must be a finite number at least 0.", false).await;
    }
    let player = state(&ctx).await?;
    player.lock().await.set_volume(&ctx, value).await;
    reply(ctx, format!("🎚️ Set volume to {value}×."), false).await
}
