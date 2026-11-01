//! `ryu-decks` — the control-deck sidecar Core spawns, health-checks and proxies to.
//!
//! Loopback-only, behind the per-plugin bearer Core injects as `RYU_EXT_TOKEN`
//! (fail-closed: with no token every `/api/decks/*` route answers 401). `/health`
//! is the one open route. `{"hid": false}` in the app's `settings.json` keeps the
//! device loop off, for headless runs and machines where no deck will ever be
//! attached (see `ryu_decks::settings`).
//!
//! On a Mac with a Touch Bar the server runs on its own thread, because AppKit
//! needs the main thread to drive the Touch Bar. `{"touch_bar": false}` keeps the
//! main thread out of it.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use tracing_subscriber::EnvFilter;

use ryu_decks::dispatch::{self, Ctx};
use ryu_decks::driver::touchbar;
use ryu_decks::host::CoreHost;
use ryu_decks::manager::Manager;
use ryu_decks::settings::DriverSettings;
use ryu_decks::store::Store;
use ryu_decks::{api, paths, DEFAULT_PORT, PLUGIN_ID};

const MOUNT: &str = "/api/decks";

#[derive(Clone)]
struct Gate {
    token: Option<Arc<str>>,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let runtime = tokio::runtime::Runtime::new().context("starting the async runtime")?;
    let drivers = DriverSettings::load(&paths::data_dir());
    let hid_off = !drivers.hid;

    #[cfg(target_os = "macos")]
    if !hid_off && drivers.touch_bar && touchbar::available() {
        let (link, main_end) = touchbar::link();
        std::thread::Builder::new()
            .name("ryu-decks-server".to_owned())
            .spawn(move || {
                let outcome = runtime.block_on(serve(hid_off, Some(link)));
                if let Err(error) = &outcome {
                    tracing::error!(error = %format!("{error:#}"), "ryu-decks: stopped");
                }
                std::process::exit(i32::from(outcome.is_err()));
            })
            .context("starting the server thread")?;
        touchbar::run_main_thread(main_end);
        // AppKit could not start (or the link closed first); the server thread
        // keeps running and ends the process itself.
        loop {
            std::thread::park();
        }
    }

    runtime.block_on(serve(hid_off, None))
}

async fn serve(hid_off: bool, touch_bar: Option<touchbar::Link>) -> Result<()> {
    let port = std::env::var("RYU_DECKS_PORT")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let db_path = paths::db_path(&paths::data_dir());
    let store = Arc::new(Store::open(&db_path)?);

    let (presses, inbox) = tokio::sync::mpsc::unbounded_channel();
    let manager = if hid_off {
        tracing::info!("ryu-decks: hid is off in settings.json; not opening any devices");
        Manager::disabled()
    } else {
        Manager::spawn(Arc::clone(&store), presses, touch_bar)
    };
    let ctx = Arc::new(Ctx::new(
        store,
        manager,
        Arc::new(CoreHost::from_env(PLUGIN_ID)),
    ));
    tokio::spawn(dispatch::run(Arc::clone(&ctx), inbox));

    let token = std::env::var("RYU_EXT_TOKEN")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if token.is_none() {
        tracing::warn!(
            "ryu-decks: no RYU_EXT_TOKEN set; /api/decks/* is FAIL-CLOSED. Core injects it at spawn."
        );
    }
    let gate = Gate {
        token: token.map(Arc::from),
    };
    let gated = Router::new()
        .nest(MOUNT, api::routes(ctx))
        .layer(from_fn_with_state(gate, bearer_gate));
    let app = Router::new().route("/health", get(health)).merge(gated);

    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("binding {address}"))?;
    tracing::info!(port, mount = MOUNT, db = %db_path.display(), "ryu-decks: listening");
    axum::serve(listener, app)
        .await
        .context("decks server stopped")?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true }))
}

async fn bearer_gate(
    State(gate): State<Gate>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let provided = bearer_of(request.headers());
    if ryu_sidecar_runtime::token_ok(provided.as_deref(), gate.token.as_deref()) {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

fn bearer_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_owned)
}
