//! Who is asking, and whether they are allowed into the admin side.

use chrono::{DateTime, TimeDelta, Utc};
use sqlx::{PgConnection, Row};

use crate::error::Result;
use crate::ids::{BarId, TelegramUserId};
use crate::{Store, lock_bar};

/// When Telegram signed a payload, as far as this server can know it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Signature {
    /// The second Telegram stamped on the payload, by Telegram's clock.
    pub stamped_at: DateTime<Utc>,
    /// How far Telegram's clock may be from this server's.
    ///
    /// Assumed, not measured: nothing in a payload says what time Telegram thought it was. Every
    /// guarantee that a payload was signed after some moment on this server's clock holds only while
    /// the two clocks are within this of each other.
    pub clock_skew: TimeDelta,
}

impl Signature {
    /// The earliest moment on this server's clock the payload can have been signed.
    #[must_use]
    pub fn earliest(self) -> DateTime<Utc> {
        self.stamped_at - self.clock_skew
    }
}

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
    /// Records an account from a payload Telegram signed, and works out what it is allowed to do.
    ///
    /// The profile in a payload is a snapshot from when it was signed, which can be some time before
    /// `now`. It rewrites the stored profile only when no later snapshot is stored, and only a
    /// payload that did rewrite it may claim a seat by its username — see [`bind_staff_seat`]. An
    /// account known only from a session goes to [`Self::recognise`]. The account in the answer is
    /// the one stored.
    pub async fn identify(
        &self,
        bar: BarId,
        account: &TelegramAccount,
        signature: Signature,
        now: DateTime<Utc>,
    ) -> Result<Viewer> {
        let mut transaction = self.pool().begin().await?;
        // The bar's lock, when a seat may be claimed, is taken before any row is: every other
        // transaction that takes it does so first, and one lock order is what keeps two from waiting
        // on each other for ever.
        let offered = seat_offered(&mut transaction, bar, account).await?;
        if offered {
            lock_bar(&mut transaction, bar).await?;
        }
        let profile = record_profile(&mut transaction, account, signature.stamped_at, now).await?;
        if offered && profile.rewritten {
            bind_staff_seat(&mut transaction, bar, account, signature, now).await?;
        }
        let stored = profile.account;
        let is_staff = is_staff(&mut transaction, bar, account.id).await?;
        let reminders = load_reminder_standing(&mut transaction, account.id).await?;
        transaction.commit().await?;

        Ok(Viewer {
            account: stored,
            is_staff,
            reminders,
        })
    }

    /// Works out what an account known from a session is allowed to do.
    ///
    /// A session carries the profile as Telegram signed it, up to a day ago, and its username may
    /// have passed to somebody else since. So it rewrites no profile and claims no seat; it only
    /// records an account never seen before. The account in the answer is the one stored.
    pub async fn recognise(
        &self,
        bar: BarId,
        account: &TelegramAccount,
        now: DateTime<Utc>,
    ) -> Result<Viewer> {
        let mut transaction = self.pool().begin().await?;
        let account = note_account(&mut transaction, account, now).await?;
        let is_staff = is_staff(&mut transaction, bar, account.id).await?;
        let reminders = load_reminder_standing(&mut transaction, account.id).await?;
        transaction.commit().await?;

        Ok(Viewer {
            account,
            is_staff,
            reminders,
        })
    }

    /// Records what the guest decided about reminders and reports where that leaves them.
    ///
    /// An account never seen before is recorded in the same transaction as the choice. Requiring
    /// the caller to have created the row first would make this method correct only when called in
    /// a particular order — a rule no signature expresses and every new caller has to be told. A
    /// known account's profile is left alone: only a fresh payload rewrites it.
    pub async fn choose_reminders(
        &self,
        account: &TelegramAccount,
        choice: ReminderChoice,
        now: DateTime<Utc>,
    ) -> Result<ReminderStanding> {
        let mut transaction = self.pool().begin().await?;
        note_account(&mut transaction, account, now).await?;
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

/// Records an account never seen before and returns the account as stored.
async fn note_account(
    connection: &mut PgConnection,
    account: &TelegramAccount,
    now: DateTime<Utc>,
) -> Result<TelegramAccount> {
    let row = sqlx::query(
        "insert into telegram_user (id, username, first_name, last_name, language_code,
                                    first_seen_at, last_seen_at)
         values ($1, $2, $3, $4, $5, $6, $6)
         on conflict (id) do update set last_seen_at = excluded.last_seen_at
         returning id, username, first_name, last_name, language_code",
    )
    .bind(account.id.0)
    .bind(&account.username)
    .bind(&account.first_name)
    .bind(&account.last_name)
    .bind(&account.language_code)
    .bind(now)
    .fetch_one(connection)
    .await?;
    account_from(&row)
}

/// The account as stored after a payload was recorded, and whether that payload's profile is it.
struct RecordedProfile {
    account: TelegramAccount,
    rewritten: bool,
}

/// Stores the profile a payload signed at `signed_at` carries, unless one signed later is stored
/// already, and returns the account as stored.
///
/// Telegram stamps whole seconds, so two payloads of one second can carry two profiles, and nothing
/// says which is the newer. Neither rewrites the other: a payload stamped the second already stored
/// counts as recorded only when its profile is the one stored, so a username given up in that second
/// cannot claim a seat under it.
async fn record_profile(
    connection: &mut PgConnection,
    account: &TelegramAccount,
    signed_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<RecordedProfile> {
    let stored = note_account(&mut *connection, account, now).await?;
    let rewritten = sqlx::query(
        "update telegram_user
         set username = $2, first_name = $3, last_name = $4, language_code = $5,
             profile_signed_at = $6
         where id = $1
           and (profile_signed_at is null
                or profile_signed_at < $6
                or (profile_signed_at = $6
                    and username is not distinct from $2 and first_name = $3
                    and last_name is not distinct from $4
                    and language_code is not distinct from $5))
         returning id, username, first_name, last_name, language_code",
    )
    .bind(account.id.0)
    .bind(&account.username)
    .bind(&account.first_name)
    .bind(&account.last_name)
    .bind(&account.language_code)
    .bind(signed_at)
    .fetch_optional(connection)
    .await?;
    Ok(match rewritten {
        Some(row) => RecordedProfile {
            account: account_from(&row)?,
            rewritten: true,
        },
        None => RecordedProfile {
            account: stored,
            rewritten: false,
        },
    })
}

fn account_from(row: &sqlx::postgres::PgRow) -> Result<TelegramAccount> {
    Ok(TelegramAccount {
        id: TelegramUserId(row.try_get("id")?),
        username: row.try_get("username")?,
        first_name: row.try_get("first_name")?,
        last_name: row.try_get("last_name")?,
        language_code: row.try_get("language_code")?,
    })
}

/// Whether the roster has an unclaimed seat under the payload's username.
async fn seat_offered(
    connection: &mut PgConnection,
    bar: BarId,
    account: &TelegramAccount,
) -> Result<bool> {
    let Some(username) = account.username.as_ref() else {
        return Ok(false);
    };
    let row = sqlx::query(
        "select exists (
             select 1 from bar_staff
             where bar_id = $1 and telegram_user_id is null and username_lower = lower($2)
         ) as offered",
    )
    .bind(bar)
    .bind(username)
    .fetch_one(connection)
    .await?;
    Ok(row.try_get("offered")?)
}

/// Claims an unclaimed seat on the roster whose username matches.
///
/// Only an *unbound* seat can be claimed. Once a seat carries a numeric id it is that person's,
/// however the username later changes hands — which is the whole reason authorisation is by id.
/// A username released by one member of staff and picked up by a stranger therefore grants the
/// stranger nothing.
///
/// Only by a payload signed after the seat was offered. The username in a payload is what the
/// account was called when Telegram signed it; whoever held the name before the offer is not who was
/// invited. Telegram stamps the second by its own clock, so the offer is compared with the earliest
/// moment the payload can have been signed on this server's clock; a payload stamped within the
/// clock skew of the offer claims the seat on a later visit instead.
///
/// Only by an account that holds no seat yet, so one person is never two members of staff. Called
/// under the bar's lock, so two payloads of one account claiming two seats at once are settled one
/// after the other and the second finds the first.
async fn bind_staff_seat(
    connection: &mut PgConnection,
    bar: BarId,
    account: &TelegramAccount,
    signature: Signature,
    now: DateTime<Utc>,
) -> Result<()> {
    let Some(username) = account.username.as_ref() else {
        return Ok(());
    };
    sqlx::query(
        "update bar_staff set telegram_user_id = $2, bound_at = $4
         where bar_id = $1 and telegram_user_id is null and username_lower = lower($3)
           and invited_at <= $5
           and not exists (
               select 1 from bar_staff held where held.bar_id = $1 and held.telegram_user_id = $2
           )",
    )
    .bind(bar)
    .bind(account.id.0)
    .bind(username)
    .bind(now)
    .bind(signature.earliest())
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
