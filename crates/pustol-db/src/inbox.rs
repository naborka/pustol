//! Claims on bot updates, so each update is handled once.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::Store;
use crate::error::Result;

impl Store {
    /// Claims `update_id` of `bot` for `owner`; returns `owner`'s attempt count, or `None` when
    /// another owner holds it.
    ///
    /// Claim older than `forgotten_before` is taken over, count restarts at one: Telegram keeps
    /// updates one day and after a week idle restarts ids from random number, so old claim may name
    /// unseen update. Same owner reclaims with count plus one: claim may have committed while reply
    /// was lost. Racing calls settled by shared row; exactly one wins.
    ///
    /// # Errors
    ///
    /// Database failure.
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

    /// Deletes claims of `bot` older than `forgotten_before`.
    ///
    /// # Errors
    ///
    /// Database failure.
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
