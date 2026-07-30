//! The server.
//!
//! Configuration comes from the environment and is read once, at start-up, so a missing variable is
//! a refusal to boot rather than a request that fails at three in the morning.

use std::net::SocketAddr;

use anyhow::{Context, Result};
use pustol_api::{AppState, Assets, Clock, router, worker};
use pustol_db::Store;
use pustol_telegram::{Bot, BotToken};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,pustol_api=debug".into()),
        )
        .init();

    let database_url = required("DATABASE_URL")?;
    let bot_token = BotToken::new(required("TELEGRAM_BOT_TOKEN")?);
    let bind: SocketAddr = std::env::var("BIND")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_owned())
        .parse()
        .context("BIND must be an address like 0.0.0.0:8080")?;
    let pool_size: u32 = std::env::var("DATABASE_MAX_CONNECTIONS")
        .unwrap_or_else(|_| "16".to_owned())
        .parse()
        .context("DATABASE_MAX_CONNECTIONS must be a number")?;
    // Absent means the app is served elsewhere — `next dev` during local work. Present and wrong
    // means somebody deployed a build that is not there, and that has to fail here rather than on
    // a guest's phone.
    let assets = std::env::var("PUSTOL_ASSETS_DIR")
        .ok()
        .map(|dir| Assets::open(&dir).with_context(|| format!("PUSTOL_ASSETS_DIR={dir}")))
        .transpose()?;

    let store = Store::connect(&database_url, pool_size)
        .await
        .context("could not reach the database")?;
    store.migrate().await.context("migrations failed")?;

    // A deployment serves one bar. Resolving it here means a misconfigured database is a start-up
    // failure with a clear message rather than a 500 on the first request.
    let bar = store
        .sole_bar()
        .await
        .context("no bar is configured; seed one before starting the API")?;

    let bot = Bot::new(bot_token.clone(), reqwest::Client::new());
    let state = AppState::new(store.clone(), bot.clone(), bar, bot_token, Clock::System);

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let outbox = tokio::spawn(worker::run(store, bot, Clock::System, shutdown_rx));

    // Routes win over a fallback, so `/health` and everything under `/api` keep answering as the
    // API however the app's build is laid out.
    let serving_app = assets.is_some();
    let app = match assets {
        Some(assets) => router(state).fallback_service(assets.into_router()),
        None => router(state),
    };

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("could not bind {bind}"))?;
    tracing::info!(%bind, %bar, serving_app, "pustol is listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutting down");
        })
        .await
        .context("the server stopped unexpectedly")?;

    // Let the outbox finish the batch it is on rather than dropping a message mid-flight.
    let _ = shutdown_tx.send(true);
    let _ = outbox.await;
    Ok(())
}

fn required(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("{name} must be set"))
}
