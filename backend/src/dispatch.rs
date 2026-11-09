//! What a press does: raise the app event, run the key's action, keep a record.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::catalog::DeviceModel;
use crate::driver::Input;
use crate::host::{Host, TurnOutcome, EVENT_DIAL_TURNED, EVENT_KEY_PRESSED, EVENT_STRIP_SWIPED};
use crate::manager::{Manager, Press};
use crate::store::{Action, Store};

const ACTIVITY_LIMIT: usize = 50;
/// A second agent request from the same key inside this window is dropped, so a
/// bouncing switch or an impatient double press cannot queue duplicate turns.
const ASK_COOLDOWN: Duration = Duration::from_millis(1500);

pub struct Ctx {
    pub store: Arc<Store>,
    pub manager: Manager,
    pub host: Arc<dyn Host>,
    activity: Mutex<VecDeque<Activity>>,
    last_ask: Mutex<HashMap<(String, u8), Instant>>,
}

impl Ctx {
    pub fn new(store: Arc<Store>, manager: Manager, host: Arc<dyn Host>) -> Self {
        Self {
            store,
            manager,
            host,
            activity: Mutex::new(VecDeque::new()),
            last_ask: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn activity(&self) -> Vec<Activity> {
        self.activity
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    fn record(&self, entry: Activity) {
        let mut activity = self
            .activity
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        activity.push_front(entry);
        activity.truncate(ACTIVITY_LIMIT);
    }

    fn cooling_down(&self, device_id: &str, key: u8) -> bool {
        let mut last = self
            .last_ask
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        let slot = (device_id.to_owned(), key);
        if last
            .get(&slot)
            .is_some_and(|at| now.duration_since(*at) < ASK_COOLDOWN)
        {
            return true;
        }
        last.insert(slot, now);
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Device,
    /// The Companion's "Test" button.
    Test,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The app event was raised; the key has no other action.
    EventRaised,
    Sent,
    PendingApproval,
    /// Accepted, but Ryu had not answered when the key stopped waiting; the
    /// agent may still be working on it.
    Unconfirmed,
    /// Dropped by the double-press guard.
    Ignored,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Activity {
    pub at: String,
    pub device_id: String,
    pub model_name: &'static str,
    pub control: &'static str,
    pub key: u8,
    pub label: Option<String>,
    pub action: &'static str,
    pub source: Source,
    pub outcome: Outcome,
    pub detail: Option<String>,
}

/// Consume presses from the device loop until it goes away.
///
/// Each press runs in its own task: an agent request can take minutes (with
/// approvals off, Core answers only after the turn), and other keys and dials
/// must not queue behind it.
pub async fn run(ctx: Arc<Ctx>, mut presses: UnboundedReceiver<Press>) {
    let mut running = tokio::task::JoinSet::new();
    while let Some(press) = presses.recv().await {
        while running.try_join_next().is_some() {}
        running.spawn(handle(Arc::clone(&ctx), press));
    }
    while running.join_next().await.is_some() {}
}

async fn handle(ctx: Arc<Ctx>, press: Press) {
    match press.input {
        Input::Key {
            index,
            pressed: true,
        } => {
            trigger(&ctx, &press.device_id, press.model, index, Source::Device).await;
        }
        Input::DialPress {
            index,
            pressed: true,
        } => {
            ctx.host
                .emit(
                    EVENT_KEY_PRESSED,
                    json!({
                        "device_id": press.device_id,
                        "model_id": press.model.id,
                        "model_name": press.model.name,
                        "control": "dial",
                        "key": index,
                        "label": null,
                        "action": "none",
                        "source": Source::Device,
                    }),
                )
                .await;
        }
        Input::DialTurn { index, delta } => {
            ctx.host
                .emit(
                    EVENT_DIAL_TURNED,
                    json!({
                        "device_id": press.device_id,
                        "model_id": press.model.id,
                        "model_name": press.model.name,
                        "dial": index,
                        "delta": delta,
                    }),
                )
                .await;
        }
        Input::StripSwipe {
            from,
            to,
            rightward,
        } => {
            ctx.host
                .emit(
                    EVENT_STRIP_SWIPED,
                    json!({
                        "device_id": press.device_id,
                        "model_id": press.model.id,
                        "model_name": press.model.name,
                        "from_key": from,
                        "to_key": to,
                        "direction": if rightward { "right" } else { "left" },
                    }),
                )
                .await;
        }
        // Releases carry nothing a binding acts on.
        Input::Key { .. } | Input::DialPress { .. } => {}
    }
}

/// Run one key exactly as a press would. Shared by the device loop and the
/// Companion's test button.
pub async fn trigger(
    ctx: &Ctx,
    device_id: &str,
    model: &'static DeviceModel,
    key: u8,
    source: Source,
) -> Activity {
    let binding = ctx.store.binding(device_id, key).unwrap_or_else(|error| {
        tracing::warn!(device = %device_id, key, %error, "ryu-decks: could not read the binding");
        None
    });
    let action = binding
        .as_ref()
        .map_or(Action::None, |binding| binding.action.clone());
    let label = binding.as_ref().map(|binding| binding.label.clone());
    let control = if model.strip_segment(key).is_some() {
        "strip"
    } else {
        "key"
    };

    ctx.host
        .emit(
            EVENT_KEY_PRESSED,
            json!({
                "device_id": device_id,
                "model_id": model.id,
                "model_name": model.name,
                "control": control,
                "key": key,
                "label": label,
                "action": action.kind(),
                "source": source,
            }),
        )
        .await;

    let (outcome, detail) = match &action {
        Action::None => (Outcome::EventRaised, None),
        Action::AskAgent { prompt, agent_id } => {
            if ctx.cooling_down(device_id, key) {
                (
                    Outcome::Ignored,
                    Some("Pressed again too quickly".to_owned()),
                )
            } else {
                match ctx.host.start_turn(prompt, agent_id.as_deref()).await {
                    Ok(TurnOutcome::PendingApproval) => (Outcome::PendingApproval, None),
                    Ok(TurnOutcome::Unconfirmed) => (Outcome::Unconfirmed, None),
                    Ok(TurnOutcome::Sent { conversation_id }) => (Outcome::Sent, conversation_id),
                    Err(message) => (Outcome::Failed, Some(message)),
                }
            }
        }
    };

    let entry = Activity {
        at: chrono::Utc::now().to_rfc3339(),
        device_id: device_id.to_owned(),
        model_name: model.name,
        control,
        key,
        label,
        action: action.kind(),
        source,
        outcome,
        detail,
    };
    ctx.record(entry.clone());
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::host::testing::FakeHost;
    use crate::store::Binding;

    fn ctx_with(host: Arc<FakeHost>, action: Action) -> Ctx {
        let store = Arc::new(Store::in_memory().unwrap());
        store
            .put_binding(&Binding {
                device_id: "elgato-mk2".to_owned(),
                key: 4,
                label: "Standup".to_owned(),
                color: "#000000".to_owned(),
                face_png: None,
                action,
                updated_at: 0,
            })
            .unwrap();
        Ctx::new(store, Manager::disabled(), host)
    }

    #[tokio::test]
    async fn every_press_raises_the_event() {
        let host = Arc::new(FakeHost::new(Ok(TurnOutcome::PendingApproval)));
        let ctx = ctx_with(host.clone(), Action::None);
        let model = by_id("elgato-mk2").unwrap();
        let entry = trigger(&ctx, "elgato-mk2", model, 4, Source::Device).await;
        assert_eq!(entry.outcome, Outcome::EventRaised);
        let unbound = trigger(&ctx, "elgato-mk2", model, 9, Source::Device).await;
        assert_eq!(unbound.label, None);

        let events = host.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, EVENT_KEY_PRESSED);
        assert_eq!(events[0].1["label"], "Standup");
        assert_eq!(events[0].1["key"], 4);
        assert_eq!(events[0].1["source"], "device");
        assert!(host.turns.lock().unwrap().is_empty());
        assert_eq!(ctx.activity().len(), 2);
    }

    #[tokio::test]
    async fn ask_agent_keys_start_one_turn_per_deliberate_press() {
        let host = Arc::new(FakeHost::new(Ok(TurnOutcome::PendingApproval)));
        let ctx = ctx_with(
            host.clone(),
            Action::AskAgent {
                prompt: "Draft my standup".to_owned(),
                agent_id: Some("writer".to_owned()),
            },
        );
        let model = by_id("elgato-mk2").unwrap();
        let first = trigger(&ctx, "elgato-mk2", model, 4, Source::Test).await;
        let bounce = trigger(&ctx, "elgato-mk2", model, 4, Source::Device).await;
        assert_eq!(first.outcome, Outcome::PendingApproval);
        assert_eq!(bounce.outcome, Outcome::Ignored);
        assert_eq!(
            *host.turns.lock().unwrap(),
            vec![("Draft my standup".to_owned(), Some("writer".to_owned()))]
        );
        // The newest entry is first.
        assert_eq!(ctx.activity()[0].outcome, Outcome::Ignored);
    }

    #[tokio::test]
    async fn a_refused_turn_is_recorded_with_its_reason() {
        let host = Arc::new(FakeHost::new(Err("not connected".to_owned())));
        let ctx = ctx_with(
            host,
            Action::AskAgent {
                prompt: "x".to_owned(),
                agent_id: None,
            },
        );
        let entry = trigger(
            &ctx,
            "elgato-mk2",
            by_id("elgato-mk2").unwrap(),
            4,
            Source::Test,
        )
        .await;
        assert_eq!(entry.outcome, Outcome::Failed);
        assert_eq!(entry.detail.as_deref(), Some("not connected"));
    }

    #[tokio::test]
    async fn dial_input_raises_its_own_events() {
        let host = Arc::new(FakeHost::new(Ok(TurnOutcome::PendingApproval)));
        let ctx = Arc::new(ctx_with(host.clone(), Action::None));
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let model = by_id("elgato-plus").unwrap();
        for input in [
            Input::DialTurn {
                index: 1,
                delta: -3,
            },
            Input::DialPress {
                index: 0,
                pressed: true,
            },
            Input::DialPress {
                index: 0,
                pressed: false,
            },
        ] {
            sender
                .send(Press {
                    device_id: "elgato-plus".to_owned(),
                    model,
                    input,
                })
                .unwrap();
        }
        drop(sender);
        run(ctx, receiver).await;
        let events = host.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        let turn = events
            .iter()
            .find(|(id, _)| id == EVENT_DIAL_TURNED)
            .unwrap();
        assert_eq!(turn.1["delta"], -3);
        let push = events
            .iter()
            .find(|(id, _)| id == EVENT_KEY_PRESSED)
            .unwrap();
        assert_eq!(push.1["control"], "dial");
    }

    #[tokio::test]
    async fn strip_taps_and_swipes_say_what_they_were() {
        let host = Arc::new(FakeHost::new(Ok(TurnOutcome::PendingApproval)));
        let ctx = Arc::new(ctx_with(host.clone(), Action::None));
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let model = by_id("elgato-plus").unwrap();
        for input in [
            Input::Key {
                index: 9,
                pressed: true,
            },
            Input::Key {
                index: 3,
                pressed: true,
            },
            Input::StripSwipe {
                from: 8,
                to: 10,
                rightward: true,
            },
        ] {
            sender
                .send(Press {
                    device_id: "elgato-plus".to_owned(),
                    model,
                    input,
                })
                .unwrap();
        }
        drop(sender);
        run(ctx, receiver).await;
        let events = host.events.lock().unwrap();
        let control_of = |key: u64| {
            events
                .iter()
                .find(|(id, payload)| id == EVENT_KEY_PRESSED && payload["key"] == key)
                .map(|(_, payload)| payload["control"].clone())
                .unwrap()
        };
        assert_eq!(control_of(9), "strip");
        assert_eq!(control_of(3), "key");
        let swipe = events
            .iter()
            .find(|(id, _)| id == EVENT_STRIP_SWIPED)
            .unwrap();
        assert_eq!(
            (
                &swipe.1["from_key"],
                &swipe.1["to_key"],
                &swipe.1["direction"]
            ),
            (
                &serde_json::json!(8),
                &serde_json::json!(10),
                &serde_json::json!("right")
            )
        );
    }
}
