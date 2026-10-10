use super::common::*;
use crate::{PContext, PError};

#[doc = "Seek to a position in the current track."]
#[poise::command(slash_command, guild_only)]
pub async fn seek(
    ctx: PContext<'_>,
    #[description = "Position, such as 0:30 or 1:02:15"] time: String,
) -> Result<(), PError> {
    let Some(seconds) = parse_time(&time) else {
        return reply(ctx, "Invalid time. Use `0:30`, `1:30`, or `2:15`.", false).await;
    };
    let player = state(&ctx).await?;
    match player.lock().await.seek(&ctx, seconds).await {
        Ok(()) => {
            reply(
                ctx,
                format!("⏩ Set position to `{}`.", format_duration(seconds)),
                false,
            )
            .await
        }
        Err(error) => reply(ctx, error, false).await,
    }
}
