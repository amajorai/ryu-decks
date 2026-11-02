//! Calls back into Core: raising this app's declared events and starting agent
//! turns. Both go to Core's authenticated host-capability broker on loopback with
//! the token Core injected at spawn, so neither works when the sidecar runs
//! standalone; a standalone run still drives decks and records presses.

use std::time::Duration;

use serde_json::{json, Value};

pub const EVENT_KEY_PRESSED: &str = "@ryu/decks#key.pressed";
pub const EVENT_DIAL_TURNED: &str = "@ryu/decks#dial.turned";
pub const EVENT_STRIP_SWIPED: &str = "@ryu/decks#strip.swiped";

/// How long a key waits for Core to accept an agent request. With approvals on,
/// Core answers at once; with them off it answers after the turn, so a timeout
/// means "still running", not "failed".
const TURN_TIMEOUT: Duration = Duration::from_secs(120);

/// What became of a request to start an agent turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    Sent {
        conversation_id: Option<String>,
    },
    /// Core queued the send in the Approvals inbox (the default for app sends).
    PendingApproval,
    /// Core accepted the request but had not answered when we stopped waiting:
    /// with approvals off it answers only after the whole agent turn.
    Unconfirmed,
}

#[async_trait::async_trait]
pub trait Host: Send + Sync {
    /// Best-effort; a missing listener must never fail a key press.
    async fn emit(&self, event: &str, payload: Value);
    async fn start_turn(&self, prompt: &str, agent_id: Option<&str>)
        -> Result<TurnOutcome, String>;
}

pub struct CoreHost {
    events: ryu_app_events::EventEmitter,
    turns: Option<TurnClient>,
}

struct TurnClient {
    http: reqwest::Client,
    base: String,
    plugin_id: String,
    token: String,
}

impl CoreHost {
    #[must_use]
    pub fn from_env(plugin_id: &str) -> Self {
        Self {
            // A slow hook must not hold a key press for long; emits are best-effort.
            events: ryu_app_events::EventEmitter::with_client(
                plugin_id,
                reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .build()
                    .unwrap_or_default(),
            ),
            turns: TurnClient::from_env(),
        }
    }
}

fn non_empty(var: &str) -> Option<String> {
    std::env::var(var)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

impl TurnClient {
    fn from_env() -> Option<Self> {
        let token = non_empty("RYU_EXT_TOKEN")?;
        let plugin_id = non_empty("RYU_EXT_PLUGIN_ID")?;
        let port: u16 = non_empty("RYU_CORE_PORT")?.parse().ok()?;
        Some(Self {
            http: reqwest::Client::builder()
                .timeout(TURN_TIMEOUT)
                .build()
                .ok()?,
            base: format!("http://127.0.0.1:{port}"),
            plugin_id,
            token,
        })
    }
}

#[async_trait::async_trait]
impl Host for CoreHost {
    async fn emit(&self, event: &str, payload: Value) {
        self.events.emit(event, payload).await;
    }

    async fn start_turn(
        &self,
        prompt: &str,
        agent_id: Option<&str>,
    ) -> Result<TurnOutcome, String> {
        let Some(client) = &self.turns else {
            return Err(
                "This deck app is not connected to Ryu, so it cannot reach an agent.".to_owned(),
            );
        };
        let response = client
            .http
            .post(format!(
                "{}/api/host/capability/chat.startTurn",
                client.base
            ))
            .header("x-ryu-plugin-id", &client.plugin_id)
            .bearer_auth(&client.token)
            .json(&json!({ "text": prompt, "agent_id": agent_id }))
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) if error.is_timeout() => return Ok(TurnOutcome::Unconfirmed),
            Err(error) => return Err(format!("Could not reach Ryu: {error}")),
        };
        let status = response.status();
        let body: Value = response.json().await.unwrap_or_else(|_| json!({}));
        turn_outcome(status.as_u16(), &body)
    }
}

/// Read Core's `chat.startTurn` answer: `202 pending_approval`, `200 sent`, or an error.
fn turn_outcome(status: u16, body: &Value) -> Result<TurnOutcome, String> {
    if !(200..300).contains(&status) {
        let message = body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Ryu refused the request");
        return Err(message.to_owned());
    }
    if body.get("status").and_then(Value::as_str) == Some("pending_approval") {
        return Ok(TurnOutcome::PendingApproval);
    }
    Ok(TurnOutcome::Sent {
        conversation_id: body
            .get("conversation_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Mutex;

    use super::*;

    /// Records every call; `start_turn` answers with `reply`.
    pub struct FakeHost {
        pub events: Mutex<Vec<(String, Value)>>,
        pub turns: Mutex<Vec<(String, Option<String>)>>,
        pub reply: Result<TurnOutcome, String>,
    }

    impl FakeHost {
        pub fn new(reply: Result<TurnOutcome, String>) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                turns: Mutex::new(Vec::new()),
                reply,
            }
        }
    }

    #[async_trait::async_trait]
    impl Host for FakeHost {
        async fn emit(&self, event: &str, payload: Value) {
            self.events
                .lock()
                .unwrap()
                .push((event.to_owned(), payload));
        }

        async fn start_turn(
            &self,
            prompt: &str,
            agent_id: Option<&str>,
        ) -> Result<TurnOutcome, String> {
            self.turns
                .lock()
                .unwrap()
                .push((prompt.to_owned(), agent_id.map(str::to_owned)));
            self.reply.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_each_start_turn_answer() {
        assert_eq!(
            turn_outcome(
                202,
                &json!({ "status": "pending_approval", "approval_id": "a1" })
            ),
            Ok(TurnOutcome::PendingApproval)
        );
        assert_eq!(
            turn_outcome(200, &json!({ "status": "sent", "conversation_id": "c9" })),
            Ok(TurnOutcome::Sent {
                conversation_id: Some("c9".to_owned())
            })
        );
        assert_eq!(
            turn_outcome(403, &json!({ "error": "not on a shared node" })),
            Err("not on a shared node".to_owned())
        );
        assert_eq!(
            turn_outcome(500, &json!({})),
            Err("Ryu refused the request".to_owned())
        );
    }
}
