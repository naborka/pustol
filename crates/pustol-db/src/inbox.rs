//! Which updates sent to the bot have already been taken in hand.

use chrono::{DateTime, Utc};

use crate::Store;
use crate::error::Result;

impl Store {
    /// Takes `update_id` of `bot` in hand at `now`, answering whether this call is the one that did.
    ///
    /// A claim made before `forgotten_before` no longer counts and is taken over: Telegram keeps an
    /// update for a day, and after a week without updates counts ids afresh from a random number, so
    /// an old claim may name an update nobody has seen. Two calls racing for one update, fresh or
    /// forgotten, are settled by the row they both write; exactly one of them is told it won.
    pub async fn claim_update(
        &self,
        bot: i64,
        update_id: i64,
        now: DateTime<Utc>,
        forgotten_before: DateTime<Utc>,
    ) -> Result<bool> {
        let claimed = sqlx::query(
            "insert into bot_update (bot_id, update_id, claimed_at) values ($1, $2, $3)
             on conflict (bot_id, update_id) do update set claimed_at = excluded.claimed_at
                where bot_update.claimed_at < $4
             returning update_id",
        )
        .bind(bot)
        .bind(update_id)
        .bind(now)
        .bind(forgotten_before)
        .fetch_optional(self.pool())
        .await?;
        Ok(claimed.is_some())
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
