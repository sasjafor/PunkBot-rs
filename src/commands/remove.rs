use super::common::*;
use crate::{PContext, PError};

#[doc = "Remove a track from the waiting queue."]
#[poise::command(slash_command, guild_only)]
pub async fn remove(
    ctx: PContext<'_>,
    #[description = "1-based position in the waiting queue"]
    #[min = 1]
    index: usize,
) -> Result<(), PError> {
    let player = state(&ctx).await?;
    match player
        .lock()
        .await
        .remove_waiting(&ctx, index.saturating_sub(1))
        .await
    {
        Ok(item) => reply(ctx, format!("✅ Removed **{}**.", item.video.title), false).await,
        Err(error) => reply(ctx, error, false).await,
    }
}
