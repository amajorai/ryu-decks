//! `/api/decks/*` — the Companion's surface. Paths here are relative to the mount.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::catalog::{self, DeviceModel, ModelView, Protocol, MODELS};
use crate::dispatch::{self, Ctx, Source};
use crate::driver::LockState;
use crate::host::{EVENT_DIAL_TURNED, EVENT_KEY_PRESSED, EVENT_STRIP_SWIPED};
use crate::manager::parse_hex_color;
use crate::store::{Action, Binding};

const MAX_LABEL_CHARS: usize = 48;
const MAX_PROMPT_CHARS: usize = 4000;
const MAX_AGENT_ID_CHARS: usize = 100;
const MAX_FACE_BYTES: usize = 512 * 1024;
const MAX_FACE_SIDE: u32 = 1024;
/// A 1024 x 1024 RGBA face, with headroom for the decoder's own buffers.
const MAX_FACE_DECODE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DEVICE_ID_CHARS: usize = 120;
const PNG_DATA_URL: &str = "data:image/png;base64,";

pub fn routes(ctx: Arc<Ctx>) -> Router {
    Router::new()
        .route("/devices", get(list_devices))
        .route("/models", get(list_models))
        .route("/activity", get(list_activity))
        .route("/devices/:id/keys", get(list_keys))
        .route("/devices/:id/keys/:key", put(put_key).delete(delete_key))
        .route("/devices/:id/keys/:key/press", post(press_key))
        .route("/devices/:id/brightness", put(set_brightness))
        .route("/devices/:id/dials/:dial/light", put(set_dial_light))
        .route("/devices/:id/light", put(set_deck_light))
        .route("/devices/:id/unlock", post(unlock))
        .with_state(ctx)
}

#[derive(Debug)]
pub struct ApiError(StatusCode, String);

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self(StatusCode::NOT_FOUND, message.into())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        tracing::error!(%error, "ryu-decks: request failed");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "The deck store could not be read or written".to_owned(),
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

/// The model behind a device id from a URL, refusing anything that is not one.
fn model_for(id: &str) -> Result<&'static DeviceModel, ApiError> {
    let well_formed = !id.is_empty()
        && id.len() <= MAX_DEVICE_ID_CHARS
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    well_formed
        .then(|| catalog::model_for_device_id(id))
        .flatten()
        .ok_or_else(|| ApiError::not_found("No such deck"))
}

fn key_for(model: &DeviceModel, key: u8) -> Result<u8, ApiError> {
    if key < model.total_keys() {
        Ok(key)
    } else {
        Err(ApiError::not_found(format!(
            "{} has keys 0 to {}",
            model.name,
            model.total_keys() - 1
        )))
    }
}

#[derive(Serialize)]
struct DeviceView {
    id: String,
    model: ModelView,
    serial: Option<String>,
    connected: bool,
    error: Option<String>,
    brightness: Option<u8>,
    bound_keys: usize,
    /// One entry per colour the deck's dial lights take (`null` = off).
    dial_colors: Vec<Option<String>>,
    /// The deck's own lights (`null` = off or none).
    deck_light: Option<String>,
    /// Whether its keys are unlocked, on a connected deck that locks them.
    lock: Option<LockState>,
}

async fn list_devices(State(ctx): State<Arc<Ctx>>) -> ApiResult<Value> {
    let (live, hid_error, running) = ctx.manager.read(|snapshot| {
        (
            snapshot.devices.clone(),
            snapshot.hid_error.clone(),
            snapshot.running,
        )
    });
    let mut ids: Vec<String> = live.keys().cloned().collect();
    for id in ctx.store.known_devices()? {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let mut devices = Vec::new();
    for id in ids {
        let status = live.get(&id);
        let Some(model) = status
            .map(|status| status.model)
            .or_else(|| catalog::model_for_device_id(&id))
        else {
            continue;
        };
        devices.push(DeviceView {
            model: ModelView::from(model),
            serial: status.and_then(|status| status.serial.clone()),
            connected: status.is_some_and(|status| status.connected),
            error: status.and_then(|status| status.error.clone()),
            brightness: ctx.store.brightness(&id)?,
            bound_keys: ctx.store.bindings(&id)?.len(),
            deck_light: ctx.store.deck_light(&id)?,
            lock: status.and_then(|status| status.lock.clone()),
            dial_colors: {
                let stored = ctx.store.dial_colors(&id)?;
                (0..model.dial_light_colors())
                    .map(|dial| stored.get(&dial).cloned())
                    .collect()
            },
            id,
        });
    }
    // Connected decks first, then by name.
    devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| a.model.name.cmp(b.model.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(Json(json!({
        "devices": devices,
        "hid": { "running": running, "error": hid_error },
        "events": {
            "key_pressed": EVENT_KEY_PRESSED,
            "dial_turned": EVENT_DIAL_TURNED,
            "strip_swiped": EVENT_STRIP_SWIPED,
        },
    })))
}

/// Starts unlocking a Vial board's keys. The user must then hold its unlock
/// keys; an unlock cannot be cancelled, so this is sent only when they ask.
async fn unlock(State(ctx): State<Arc<Ctx>>, Path(id): Path<String>) -> ApiResult<Value> {
    let model = model_for(&id)?;
    if model.protocol != Protocol::VialRawHid {
        return Err(ApiError::bad_request(format!("{} has no lock", model.name)));
    }
    let lock = ctx.manager.read(|snapshot| {
        snapshot
            .devices
            .get(&id)
            .filter(|status| status.connected)
            .map(|status| status.lock.clone())
    });
    let Some(Some(lock)) = lock else {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Plug the keyboard in first".to_owned(),
        ));
    };
    if lock.unlocked || lock.unlocking {
        return Ok(Json(json!({ "started": false })));
    }
    if lock.keys_off_layout {
        return Err(ApiError::bad_request(
            crate::driver::vial::UNLOCK_KEYS_OFF_LAYOUT,
        ));
    }
    if lock.keys.is_empty() {
        return Err(ApiError::bad_request(
            "This keyboard names no unlock keys. Unlock it in the Vial app instead.",
        ));
    }
    ctx.manager.unlock(&id);
    Ok(Json(json!({ "started": true })))
}

async fn list_models() -> Json<Value> {
    let models: Vec<ModelView> = MODELS.iter().map(ModelView::from).collect();
    Json(json!({ "models": models }))
}

async fn list_activity(State(ctx): State<Arc<Ctx>>) -> Json<Value> {
    Json(json!({ "activity": ctx.activity() }))
}

#[derive(Serialize)]
struct KeyView {
    key: u8,
    label: String,
    color: String,
    /// The stored face as a PNG data URL.
    face: Option<String>,
    action: Action,
    updated_at: i64,
}

impl From<Binding> for KeyView {
    fn from(binding: Binding) -> Self {
        Self {
            key: binding.key,
            label: binding.label,
            color: binding.color,
            face: binding.face_png.map(|png| {
                format!(
                    "{PNG_DATA_URL}{}",
                    base64::engine::general_purpose::STANDARD.encode(png)
                )
            }),
            action: binding.action,
            updated_at: binding.updated_at,
        }
    }
}

async fn list_keys(State(ctx): State<Arc<Ctx>>, Path(id): Path<String>) -> ApiResult<Value> {
    model_for(&id)?;
    let keys: Vec<KeyView> = ctx
        .store
        .bindings(&id)?
        .into_iter()
        .map(KeyView::from)
        .collect();
    Ok(Json(json!({ "keys": keys })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyInput {
    #[serde(default)]
    label: String,
    color: String,
    #[serde(default)]
    face: Option<String>,
    action: Action,
}

fn validated_face(face: Option<String>) -> Result<Option<Vec<u8>>, ApiError> {
    let Some(face) = face.filter(|face| !face.is_empty()) else {
        return Ok(None);
    };
    let encoded = face
        .strip_prefix(PNG_DATA_URL)
        .ok_or_else(|| ApiError::bad_request("A key face must be a PNG data URL"))?;
    let png = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| ApiError::bad_request("The key face is not valid base64"))?;
    if png.len() > MAX_FACE_BYTES {
        return Err(ApiError::bad_request("The key face is larger than 512 KB"));
    }
    // Read the header first so an image that is small on the wire but huge
    // when inflated is refused before any pixel is decoded.
    let reader = || {
        image::ImageReader::with_format(
            std::io::Cursor::new(png.as_slice()),
            image::ImageFormat::Png,
        )
    };
    let (width, height) = reader()
        .into_dimensions()
        .map_err(|_| ApiError::bad_request("The key face is not a readable PNG"))?;
    if width > MAX_FACE_SIDE || height > MAX_FACE_SIDE {
        return Err(ApiError::bad_request(
            "A key face may be at most 1024 pixels on a side",
        ));
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_FACE_SIDE);
    limits.max_image_height = Some(MAX_FACE_SIDE);
    limits.max_alloc = Some(MAX_FACE_DECODE_BYTES);
    let mut decoder = reader();
    decoder.limits(limits);
    decoder
        .decode()
        .map_err(|_| ApiError::bad_request("The key face is not a readable PNG"))?;
    Ok(Some(png))
}

fn validated_action(action: Action) -> Result<Action, ApiError> {
    match action {
        Action::None => Ok(Action::None),
        Action::AskAgent { prompt, agent_id } => {
            let prompt = prompt.trim().to_owned();
            if prompt.is_empty() {
                return Err(ApiError::bad_request(
                    "Write what the agent should be asked",
                ));
            }
            if prompt.chars().count() > MAX_PROMPT_CHARS {
                return Err(ApiError::bad_request(
                    "The prompt is longer than 4000 characters",
                ));
            }
            let agent_id = agent_id
                .map(|id| id.trim().to_owned())
                .filter(|id| !id.is_empty());
            if agent_id.as_ref().is_some_and(|id| {
                id.chars().count() > MAX_AGENT_ID_CHARS || id.chars().any(char::is_control)
            }) {
                return Err(ApiError::bad_request("That agent id is not valid"));
            }
            Ok(Action::AskAgent { prompt, agent_id })
        }
    }
}

async fn put_key(
    State(ctx): State<Arc<Ctx>>,
    Path((id, key)): Path<(String, u8)>,
    Json(input): Json<KeyInput>,
) -> ApiResult<KeyView> {
    let model = model_for(&id)?;
    let key = key_for(model, key)?;
    let label = input.label.trim().to_owned();
    if label.chars().count() > MAX_LABEL_CHARS || label.chars().any(char::is_control) {
        return Err(ApiError::bad_request(
            "A key label is at most 48 characters on one line",
        ));
    }
    if parse_hex_color(&input.color).is_none() {
        return Err(ApiError::bad_request("The colour must look like #2d7ff9"));
    }
    let binding = Binding {
        device_id: id.clone(),
        key,
        label,
        color: input.color.to_ascii_lowercase(),
        // PNG decoding is CPU work; keep it off the async workers.
        face_png: tokio::task::spawn_blocking(move || validated_face(input.face))
            .await
            .map_err(|_| ApiError::bad_request("The key face could not be checked"))??,
        action: validated_action(input.action)?,
        updated_at: chrono::Utc::now().timestamp_millis(),
    };
    ctx.store.put_binding(&binding)?;
    ctx.manager.repaint(&id);
    Ok(Json(KeyView::from(binding)))
}

async fn delete_key(
    State(ctx): State<Arc<Ctx>>,
    Path((id, key)): Path<(String, u8)>,
) -> ApiResult<Value> {
    let model = model_for(&id)?;
    let key = key_for(model, key)?;
    let removed = ctx.store.delete_binding(&id, key)?;
    if removed {
        ctx.manager.repaint(&id);
    }
    Ok(Json(json!({ "removed": removed })))
}

async fn press_key(
    State(ctx): State<Arc<Ctx>>,
    Path((id, key)): Path<(String, u8)>,
) -> ApiResult<dispatch::Activity> {
    let model = model_for(&id)?;
    let key = key_for(model, key)?;
    Ok(Json(
        dispatch::trigger(&ctx, &id, model, key, Source::Test).await,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrightnessInput {
    percent: u8,
}

async fn set_brightness(
    State(ctx): State<Arc<Ctx>>,
    Path(id): Path<String>,
    Json(input): Json<BrightnessInput>,
) -> ApiResult<Value> {
    model_for(&id)?;
    if input.percent > 100 {
        return Err(ApiError::bad_request(
            "Brightness is a percentage from 0 to 100",
        ));
    }
    ctx.store.set_brightness(&id, input.percent)?;
    ctx.manager.set_brightness(&id, input.percent);
    Ok(Json(json!({ "brightness": input.percent })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DialLightInput {
    /// `#rrggbb`, or `null` to turn the light off.
    color: Option<String>,
}

async fn set_dial_light(
    State(ctx): State<Arc<Ctx>>,
    Path((id, dial)): Path<(String, u8)>,
    Json(input): Json<DialLightInput>,
) -> ApiResult<Value> {
    let model = model_for(&id)?;
    let colors = model.dial_light_colors();
    if dial >= colors {
        return Err(ApiError::not_found(if colors == 0 {
            format!("{} has no dial lights", model.name)
        } else {
            format!("{} has dial lights 0 to {}", model.name, colors - 1)
        }));
    }
    let color = input.color.map(|color| color.to_ascii_lowercase());
    if let Some(color) = &color {
        if parse_hex_color(color).is_none() {
            return Err(ApiError::bad_request("A dial light colour is #rrggbb"));
        }
    }
    ctx.store.set_dial_color(&id, dial, color.as_deref())?;
    ctx.manager.repaint(&id);
    Ok(Json(json!({ "dial": dial, "color": color })))
}

async fn set_deck_light(
    State(ctx): State<Arc<Ctx>>,
    Path(id): Path<String>,
    Json(input): Json<DialLightInput>,
) -> ApiResult<Value> {
    let model = model_for(&id)?;
    if model.deck_lights == 0 {
        return Err(ApiError::not_found(format!(
            "{} has no lights of its own",
            model.name
        )));
    }
    let color = input.color.map(|color| color.to_ascii_lowercase());
    if let Some(color) = &color {
        if parse_hex_color(color).is_none() {
            return Err(ApiError::bad_request("A light colour is #rrggbb"));
        }
    }
    ctx.store.set_deck_light(&id, color.as_deref())?;
    ctx.manager.repaint(&id);
    Ok(Json(json!({ "color": color })))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;
    use crate::catalog::by_id;
    use crate::host::testing::FakeHost;
    use crate::host::TurnOutcome;
    use crate::manager::{DeviceStatus, Manager, Snapshot};
    use crate::store::Store;

    fn app_with(manager: Manager) -> (Router, Arc<Ctx>, Arc<FakeHost>) {
        let host = Arc::new(FakeHost::new(Ok(TurnOutcome::PendingApproval)));
        let ctx = Arc::new(Ctx::new(
            Arc::new(Store::in_memory().unwrap()),
            manager,
            host.clone(),
        ));
        (routes(ctx.clone()), ctx, host)
    }

    async fn call(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn tiny_png_data_url() -> String {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        format!(
            "{PNG_DATA_URL}{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        )
    }

    #[tokio::test]
    async fn binding_a_key_round_trips_and_lists_the_deck() {
        let (app, _, _) = app_with(Manager::disabled());
        let face = tiny_png_data_url();
        let (status, saved) = call(
            &app,
            "PUT",
            "/devices/elgato-mk2-abc/keys/3",
            Some(json!({
                "label": "  Standup ",
                "color": "#2D7FF9",
                "face": face,
                "action": { "type": "ask_agent", "prompt": " Draft my standup ", "agent_id": " " },
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["label"], "Standup");
        assert_eq!(saved["color"], "#2d7ff9");
        assert_eq!(saved["face"], face);
        assert_eq!(
            saved["action"],
            json!({ "type": "ask_agent", "prompt": "Draft my standup", "agent_id": null })
        );

        let (_, keys) = call(&app, "GET", "/devices/elgato-mk2-abc/keys", None).await;
        assert_eq!(keys["keys"].as_array().unwrap().len(), 1);

        let (_, devices) = call(&app, "GET", "/devices", None).await;
        let device = &devices["devices"][0];
        assert_eq!(device["id"], "elgato-mk2-abc");
        assert_eq!(device["model"]["name"], "Stream Deck MK.2");
        assert_eq!(device["connected"], false);
        assert_eq!(device["bound_keys"], 1);
        assert_eq!(devices["events"]["key_pressed"], EVENT_KEY_PRESSED);

        let (status, removed) = call(&app, "DELETE", "/devices/elgato-mk2-abc/keys/3", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(removed["removed"], true);
    }

    #[tokio::test]
    async fn dial_lights_are_set_per_dial_or_shared() {
        let (app, _, _) = app_with(Manager::disabled());
        let red = json!({ "color": "#FF0000" });
        let (status, body) = call(
            &app,
            "PUT",
            "/devices/elgato-studio/dials/1/light",
            Some(red.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["color"], "#ff0000");
        // The Studio has two dials; the XL's lights share one colour; the + has none.
        for uri in [
            "/devices/elgato-studio/dials/2/light",
            "/devices/mirabox-stream-dock-xl/dials/1/light",
            "/devices/elgato-plus/dials/0/light",
            // The X-Touch Mini's fader (dial 9) has no light.
            "/devices/behringer-x-touch-mini/dials/8/light",
        ] {
            let (status, _) = call(&app, "PUT", uri, Some(red.clone())).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        }
        let (status, _) = call(
            &app,
            "PUT",
            "/devices/elgato-studio/dials/0/light",
            Some(json!({ "color": "red" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = call(
            &app,
            "PUT",
            "/devices/behringer-x-touch-mini/dials/7/light",
            Some(red.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (_, devices) = call(&app, "GET", "/devices", None).await;
        let mini = devices["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|device| device["id"] == "behringer-x-touch-mini")
            .unwrap();
        assert_eq!(mini["dial_colors"].as_array().unwrap().len(), 8);
        let studio = devices["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|device| device["id"] == "elgato-studio")
            .unwrap();
        assert_eq!(studio["dial_colors"], json!([null, "#ff0000"]));
        assert_eq!(studio["model"]["dial_lights"], "per_dial");

        // Turning it off clears it.
        let (status, _) = call(
            &app,
            "PUT",
            "/devices/elgato-studio/dials/1/light",
            Some(json!({ "color": null })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, devices) = call(&app, "GET", "/devices", None).await;
        assert!(devices["devices"]
            .as_array()
            .unwrap()
            .iter()
            .all(|device| device["id"] != "elgato-studio"));
    }

    #[tokio::test]
    async fn deck_lights_only_on_decks_that_have_them() {
        let (app, _, _) = app_with(Manager::disabled());
        let blue = json!({ "color": "#0000FF" });
        let (status, body) = call(
            &app,
            "PUT",
            "/devices/mirabox-m18/light",
            Some(blue.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, _) = call(&app, "PUT", "/devices/elgato-mk2/light", Some(blue)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, devices) = call(&app, "GET", "/devices", None).await;
        assert_eq!(devices["devices"][0]["deck_light"], "#0000ff");
        assert_eq!(devices["devices"][0]["model"]["deck_lights"], 24);
    }

    #[tokio::test]
    async fn invalid_requests_are_refused() {
        let (app, _, _) = app_with(Manager::disabled());
        let good = json!({ "label": "x", "color": "#000000", "action": { "type": "none" } });
        for (uri, body) in [
            ("/devices/not-a-deck/keys/0", good.clone()),
            ("/devices/elgato-mini/keys/6", good.clone()),
            ("/devices/Elgato-MK2/keys/0", good.clone()),
        ] {
            let (status, _) = call(&app, "PUT", uri, Some(body)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        }
        for body in [
            json!({ "label": "x", "color": "blue", "action": { "type": "none" } }),
            json!({ "label": "x".repeat(49), "color": "#000000", "action": { "type": "none" } }),
            json!({ "label": "x", "color": "#000000", "action": { "type": "ask_agent", "prompt": "  " } }),
            json!({ "label": "x", "color": "#000000", "face": "data:image/jpeg;base64,AAAA", "action": { "type": "none" } }),
            json!({ "label": "x", "color": "#000000", "face": "data:image/png;base64,AAAA", "action": { "type": "none" } }),
        ] {
            let (status, reply) = call(&app, "PUT", "/devices/elgato-mk2/keys/0", Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
            assert!(reply["error"].is_string());
        }
        let (status, _) = call(
            &app,
            "PUT",
            "/devices/elgato-mk2/keys/0",
            Some(json!({ "color": "#000000", "action": { "type": "run_shell" } })),
        )
        .await;
        assert!(status.is_client_error());
        let (status, _) = call(
            &app,
            "PUT",
            "/devices/elgato-mk2/brightness",
            Some(json!({ "percent": 101 })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn oversized_faces_are_refused_from_the_header() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(1100, 4, image::Rgba([0, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let url = format!(
            "{PNG_DATA_URL}{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        );
        let error = validated_face(Some(url)).unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert!(error.1.contains("1024"));
        assert!(validated_face(Some(tiny_png_data_url())).unwrap().is_some());
    }

    #[tokio::test]
    async fn test_presses_run_the_key_and_show_in_activity() {
        let (app, _, host) = app_with(Manager::disabled());
        call(
            &app,
            "PUT",
            "/devices/openai-codex-micro/keys/0",
            Some(json!({
                "label": "Review",
                "color": "#00ff00",
                "action": { "type": "ask_agent", "prompt": "Review my open PRs" },
            })),
        )
        .await;
        let (status, entry) = call(
            &app,
            "POST",
            "/devices/openai-codex-micro/keys/0/press",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(entry["outcome"], "pending_approval");
        assert_eq!(entry["source"], "test");
        assert_eq!(host.turns.lock().unwrap().len(), 1);
        let (_, activity) = call(&app, "GET", "/activity", None).await;
        assert_eq!(activity["activity"][0]["label"], "Review");
    }

    #[tokio::test]
    async fn connected_decks_list_first_with_their_status() {
        let mut snapshot = Snapshot {
            running: true,
            ..Snapshot::default()
        };
        snapshot.devices.insert(
            "ulanzi-d200-x1".to_owned(),
            DeviceStatus {
                model: by_id("ulanzi-d200").unwrap(),
                serial: Some("X1".to_owned()),
                connected: true,
                error: None,
                lock: None,
            },
        );
        let (app, ctx, _) = app_with(Manager::with_snapshot(snapshot));
        ctx.store.set_brightness("elgato-xl", 50).unwrap();
        let (_, devices) = call(&app, "GET", "/devices", None).await;
        let list = devices["devices"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["id"], "ulanzi-d200-x1");
        assert_eq!(list[0]["connected"], true);
        assert_eq!(list[0]["model"]["keys"], 13);
        assert_eq!(list[1]["brightness"], 50);
        assert_eq!(devices["hid"]["running"], true);

        let (_, models) = call(&app, "GET", "/models", None).await;
        assert!(models["models"].as_array().unwrap().len() >= 10);
    }
}
