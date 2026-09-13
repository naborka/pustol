//! Which updates sent to the bot have already been taken in hand.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::Store;
use crate::error::Result;

impl Store {
    /// Takes `update_id` of `bot` in hand for `owner` at `now`, answering how many times `owner` has
    /// now taken it, or `None` when this call did not take it.
    ///
    /// A claim made before `forgotten_before` no longer counts and is taken over, counting from one:
    /// Telegram keeps an update for a day, and after a week without updates counts ids afresh from a
    /// random number, so an old claim may name an update nobody has seen. A claim `owner` made itself
    /// is handed back to it, one more time: the claim may have been written while the reply saying so
    /// was lost, and an owner that asks again has not answered the update. Two calls racing for one
    /// update are settled by the row they both write; exactly one of them is told it won.
    pub async fn claim_update(
        &self,
        bot: i64,
        update_id: i64,
        owner: Uuid,
        now: DateTime<Utc>,
        forgotten_before: DateTime<Utc>,
    ) -> Result<Option<i32>> {
        let attempt = sqlx::query_scalar(
            "insert into bot_update (bot_id, update_id, claimed_at, owner, attempts)
             values ($1, $2, $3, $5, 1)
             on conflict (bot_id, update_id) do update
                set claimed_at = excluded.claimed_at, owner = excluded.owner,
                    attempts = case when bot_update.claimed_at < $4 then 1
                                    else bot_update.attempts + 1 end
                where bot_update.claimed_at < $4 or bot_update.owner = excluded.owner
             returning attempts",
        )
        .bind(bot)
        .bind(update_id)
        .bind(now)
        .bind(forgotten_before)
        .bind(owner)
        .fetch_optional(self.pool())
        .await?;
        Ok(attempt)
    }

    /// Clears away the claims of `bot` made before `forgotten_before`, which no longer count.
    pub async fn forget_update_claims(
        &self,
        bot: i64,
        forgotten_before: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query("delete from bot_update where bot_id = $1 and claimed_at < $2")
            .bind(bot)
            .bind(forgotten_before)
            .execute(self.pool())
            .await?;
        Ok(())
    }
}
