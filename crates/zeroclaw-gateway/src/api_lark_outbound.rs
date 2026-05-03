//! Lark/Feishu outbound proxy used by local sidecars such as lark-codex-ninja.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use serde::Deserialize;
use zeroclaw_channels::lark::LarkChannel;

use crate::{AppState, api};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LarkOutboundAction {
    SendCard,
    ReplyCard,
    ReplyText,
}

#[derive(Debug, Deserialize)]
pub struct LarkOutboundRequest {
    action: LarkOutboundAction,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    chat_id: Option<String>,
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    card: Option<serde_json::Value>,
    #[serde(default)]
    reply_in_thread: Option<bool>,
}

pub async fn handle_lark_outbound(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LarkOutboundRequest>,
) -> impl IntoResponse {
    if let Err(e) = api::require_auth(&state, &headers) {
        return e.into_response();
    }

    match send_lark_outbound(&state, request).await {
        Ok(message_id) => Json(serde_json::json!({
            "ok": true,
            "message_id": message_id,
        }))
        .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "ok": false,
                "error": error.to_string(),
            })),
        )
            .into_response(),
    }
}

async fn send_lark_outbound(
    state: &AppState,
    request: LarkOutboundRequest,
) -> anyhow::Result<String> {
    let channel = lark_channel_from_state(state, request.channel.as_deref())?;
    match request.action {
        LarkOutboundAction::SendCard => {
            let chat_id = required_field(request.chat_id.as_deref(), "chat_id")?;
            let card = request
                .card
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("card is required"))?;
            channel.send_raw_card_message(chat_id, card).await
        }
        LarkOutboundAction::ReplyCard => {
            let message_id = required_field(request.message_id.as_deref(), "message_id")?;
            let card = request
                .card
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("card is required"))?;
            channel
                .reply_raw_card_message(message_id, card, request.reply_in_thread.unwrap_or(true))
                .await
        }
        LarkOutboundAction::ReplyText => {
            let message_id = required_field(request.message_id.as_deref(), "message_id")?;
            let text = required_field(request.text.as_deref(), "text")?;
            channel
                .reply_text_message(message_id, text, request.reply_in_thread.unwrap_or(true))
                .await
        }
    }
}

fn lark_channel_from_state(
    state: &AppState,
    requested_channel: Option<&str>,
) -> anyhow::Result<LarkChannel> {
    let config = state.config.lock().clone();
    match requested_channel
        .unwrap_or("auto")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "feishu" => config
            .channels
            .feishu
            .as_ref()
            .map(LarkChannel::from_feishu_config)
            .ok_or_else(|| anyhow::anyhow!("channels.feishu is not configured")),
        "lark" => config
            .channels
            .lark
            .as_ref()
            .map(LarkChannel::from_lark_config)
            .ok_or_else(|| anyhow::anyhow!("channels.lark is not configured")),
        "auto" | "" => config
            .channels
            .feishu
            .as_ref()
            .map(LarkChannel::from_feishu_config)
            .or_else(|| {
                config
                    .channels
                    .lark
                    .as_ref()
                    .map(LarkChannel::from_lark_config)
            })
            .ok_or_else(|| anyhow::anyhow!("no Lark/Feishu channel is configured")),
        other => anyhow::bail!("unsupported Lark outbound channel: {other}"),
    }
}

fn required_field<'a>(value: Option<&'a str>, name: &str) -> anyhow::Result<&'a str> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_deserializes_send_card_action() {
        let request: LarkOutboundRequest = serde_json::from_value(serde_json::json!({
            "action": "send_card",
            "chat_id": "oc_chat",
            "card": {"elements": []}
        }))
        .expect("request");

        assert!(matches!(request.action, LarkOutboundAction::SendCard));
        assert_eq!(request.chat_id.as_deref(), Some("oc_chat"));
        assert_eq!(
            request
                .card
                .as_ref()
                .and_then(|card| card.get("elements"))
                .and_then(|elements| elements.as_array())
                .map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn required_field_rejects_blank_values() {
        assert_eq!(
            required_field(Some("  om_123  "), "message_id").unwrap(),
            "om_123"
        );
        assert!(required_field(Some("  "), "message_id").is_err());
        assert!(required_field(None, "message_id").is_err());
    }
}
