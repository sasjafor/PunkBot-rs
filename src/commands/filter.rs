use super::common::*;
use crate::{PContext, PError};

#[doc = "Set FFmpeg audio filters."]
#[poise::command(slash_command, guild_only)]
pub async fn filter(
    ctx: PContext<'_>,
    #[description = "Audio filter expression passed to FFmpeg"] filters: String,
) -> Result<(), PError> {
    let player = state(&ctx).await?;
    match player.lock().await.set_filters(&ctx, filters.clone()).await {
        Ok(()) => reply(ctx, format!("🎛️ Set audio filters to `{filters}`."), false).await,
        Err(error) => reply(ctx, error, false).await,
    }
}
