//! Who is asking, and whether they are allowed into the admin side.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Row};

use crate::error::Result;
use crate::ids::{BarId, TelegramUserId};
use crate::Store;

/// A Telegram account as the app has just seen it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TelegramAccount {
    pub id: TelegramUserId,
    pub username: Option<String>,
    pub first_name: String,
    pub last_name: Option<String>,
    pub language_code: Option<String>,
}

/// Where this guest stands on being reminded.
///
/// Three facts that are easy to conflate and must not be. Wanting a reminder is consent;
/// being reachable is evidence from the last delivery attempt; having dismissed the prompt is
/// neither, and only says the app should stop asking. A bar that folded them together would
/// either nag people who declined or silently drop what they asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReminderStanding {
    /// The guest asked to be reminded.
    pub opted_in: bool,
    /// The bot has not been found unable to reach them.
    pub deliverable: bool,
    /// The guest said "not now".
    pub prompt_dismissed: bool,
}

impl ReminderStanding {
    /// Whether the reminder prompt is worth showing.
    #[must_use]
    pub const fn should_ask(self) -> bool {
        !self.opted_in && !self.prompt_dismissed
    }
}

/// What the app needs to know about the person holding it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Viewer {
    pub account: TelegramAccount,
    /// Whether this account is on the bar's admin roster.
    pub is_staff: bool,
    pub reminders: ReminderStanding,
}

impl Store {
    /// Records the account and works out what it is allowed to do.
    ///
    /// Called on every authenticated request, because the Telegram payload is the freshest source
    /// for a display name, and because a member of staff invited a minute ago should get in on
    /// their first visit rather than after a cache expires.
    pub async fn identify(
        &self,
        bar: BarId,
        account: &TelegramAccount,
        now: DateTime<Utc>,
    ) -> Result<Viewer> {
        let mut transaction = self.pool().begin().await?;
        upsert_account(&mut transaction, account, now).await?;
        bind_staff_seat(&mut transaction, bar, account).await?;
        let is_staff = is_staff(&mut transaction, bar, account.id).await?;
        let reminders = load_reminder_standing(&mut transaction, account.id).await?;
        transaction.commit().await?;

        Ok(Viewer {
            account: account.clone(),
            is_staff,
            reminders,
        })
    }

    /// Records what the guest decided about reminders and reports where that leaves them.
    ///
    /// The account is recorded in the same transaction as the choice. Requiring the caller to have
    /// created the row first would make this method correct only when called in a particular order —
    /// a rule no signature expresses and every new caller has to be told.
    pub async fn choose_reminders(
        &self,
        account: &TelegramAccount,
        choice: ReminderChoice,
        now: DateTime<Utc>,
    ) -> Result<ReminderStanding> {
        let mut transaction = self.pool().begin().await?;
        upsert_account(&mut transaction, account, now).await?;
        // `returning` rather than a second read: the answer the caller needs is the row just
        // written, and reading it again could observe somebody else's write instead.
        let row = sqlx::query(match choice {
            ReminderChoice::OptIn => {
                "update telegram_user
                 set reminders_opted_in = true, reminder_prompt_dismissed_at = null
                 where id = $1
                 returning reminders_opted_in, can_receive_messages,
                           reminder_prompt_dismissed_at is not null as dismissed"
            }
            ReminderChoice::NotNow => {
                "update telegram_user set reminder_prompt_dismissed_at = $2 where id = $1
                 returning reminders_opted_in, can_receive_messages,
                           reminder_prompt_dismissed_at is not null as dismissed"
            }
        })
        .bind(account.id.0)
        .bind(now)
        .fetch_one(&mut *transaction)
        .await?;
        let standing = standing_from(&row)?;
        transaction.commit().await?;
        Ok(standing)
    }

    /// Records whether the bot can reach this account, learned from what a send actually did.
    ///
    /// There is no Bot API call that asks whether a chat exists, so evidence from a delivery — or a
    /// refusal — is the only honest way to know.
    pub async fn set_reachable(&self, user: TelegramUserId, reachable: bool) -> Result<()> {
        sqlx::query("update telegram_user set can_receive_messages = $2 where id = $1")
            .bind(user.0)
            .bind(reachable)
            .execute(self.pool())
            .await?;
        Ok(())
    }
}

/// What a guest decided when the app offered to remind them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReminderChoice {
    OptIn,
    NotNow,
}

pub(crate) async fn upsert_account(
    connection: &mut PgConnection,
    account: &TelegramAccount,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "insert into telegram_user (id, username, first_name, last_name, language_code,
                                    first_seen_at, last_seen_at)
         values ($1, $2, $3, $4, $5, $6, $6)
         on conflict (id) do update set
            username = excluded.username,
            first_name = excluded.first_name,
            last_name = excluded.last_name,
            language_code = excluded.language_code,
            last_seen_at = excluded.last_seen_at",
    )
    .bind(account.id.0)
    .bind(&account.username)
    .bind(&account.first_name)
    .bind(&account.last_name)
    .bind(&account.language_code)
    .bind(now)
    .execute(connection)
    .await?;
    Ok(())
}

/// Claims an unclaimed seat on the roster whose username matches.
///
/// Only an *unbound* seat can be claimed. Once a seat carries a numeric id it is that person's,
/// however the username later changes hands — which is the whole reason authorisation is by id.
/// A username released by one member of staff and picked up by a stranger therefore grants the
/// stranger nothing.
async fn bind_staff_seat(
    connection: &mut PgConnection,
    bar: BarId,
    account: &TelegramAccount,
) -> Result<()> {
    let Some(username) = account.username.as_ref() else {
        return Ok(());
    };
    sqlx::query(
        "update bar_staff set telegram_user_id = $2, bound_at = now()
         where bar_id = $1 and telegram_user_id is null and username_lower = lower($3)",
    )
    .bind(bar)
    .bind(account.id.0)
    .bind(username)
    .execute(connection)
    .await?;
    Ok(())
}

pub(crate) async fn is_staff(
    connection: &mut PgConnection,
    bar: BarId,
    user: TelegramUserId,
) -> Result<bool> {
    let row = sqlx::query(
        "select exists (
             select 1 from bar_staff where bar_id = $1 and telegram_user_id = $2
         ) as present",
    )
    .bind(bar)
    .bind(user.0)
    .fetch_one(connection)
    .await?;
    Ok(row.try_get("present")?)
}

async fn load_reminder_standing(
    connection: &mut PgConnection,
    user: TelegramUserId,
) -> Result<ReminderStanding> {
    let row = sqlx::query(
        "select reminders_opted_in, can_receive_messages,
                reminder_prompt_dismissed_at is not null as dismissed
         from telegram_user where id = $1",
    )
    .bind(user.0)
    .fetch_one(connection)
    .await?;
    standing_from(&row)
}

/// Reads the three reminder columns, wherever they were selected from.
fn standing_from(row: &sqlx::postgres::PgRow) -> Result<ReminderStanding> {
    Ok(ReminderStanding {
        opted_in: row.try_get("reminders_opted_in")?,
        deliverable: row.try_get("can_receive_messages")?,
        prompt_dismissed: row.try_get("dismissed")?,
    })
}
