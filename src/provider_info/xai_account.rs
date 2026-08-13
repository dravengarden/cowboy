//! Read-only xAI account metadata and the explicit usage-reset action.
//!
//! Grok Build currently exposes billing over ACP but not subscription identity
//! or usage resets. Cowboy keeps this bridge narrow: it uses the same official
//! OIDC credential file as Grok Build, calls Grok's own account endpoints, and
//! can be deleted once those operations become native Grok ACP methods.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::Value;

const GROK_WEB_BASE: &str = "https://grok.com";
const SUBSCRIPTIONS_PATH: &str = "/rest/subscriptions";
const GET_RESETS_PATH: &str = "/prod_mc_billing.ConsumerUiSvc/GetRemainingResets";
const REDEEM_RESET_PATH: &str = "/prod_mc_billing.ConsumerUiSvc/RedeemReset";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResetCredit {
    pub id: String,
    pub granted_at: Option<i64>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct AccountSnapshot {
    pub plan: Option<String>,
    pub resets: Option<Vec<ResetCredit>>,
}

struct Credential {
    key: String,
    user_id: String,
}

pub(crate) async fn collect() -> Result<AccountSnapshot> {
    let credential = load_credential()?;
    let client = http_client()?;
    let (plan, resets) = tokio::join!(
        fetch_active_plan(&client, &credential),
        fetch_remaining_resets(&client, &credential),
    );
    let plan = match plan {
        Ok(plan) => plan,
        Err(error) => {
            tracing::warn!(provider = "xai", %error, "reading xAI subscription plan");
            None
        }
    };
    let resets = match resets {
        Ok(resets) => Some(resets),
        Err(error) => {
            tracing::warn!(provider = "xai", %error, "reading xAI reset availability");
            None
        }
    };
    Ok(AccountSnapshot { plan, resets })
}

pub(crate) async fn redeem_reset(token_id: &str) -> Result<usize> {
    ensure!(!token_id.trim().is_empty(), "xAI reset token is empty");
    let credential = load_credential()?;
    let client = http_client()?;
    let request = encode_string_field(10, token_id);
    let response = grpc_web_unary(&client, &credential, REDEEM_RESET_PATH, request).await?;
    Ok(decode_reset_tokens(&response, 10)?.len())
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("build xAI account client")
}

async fn fetch_active_plan(
    client: &reqwest::Client,
    credential: &Credential,
) -> Result<Option<String>> {
    let response = client
        .get(format!("{GROK_WEB_BASE}{SUBSCRIPTIONS_PATH}"))
        .headers(auth_headers(credential)?)
        .send()
        .await
        .context("request xAI subscriptions")?;
    ensure!(
        response.status().is_success(),
        "xAI subscriptions returned HTTP {}",
        response.status().as_u16()
    );
    let length = response.content_length().unwrap_or(0);
    ensure!(
        length <= u64::try_from(MAX_RESPONSE_BYTES).unwrap_or(u64::MAX),
        "xAI subscriptions response is too large"
    );
    let bytes = response
        .bytes()
        .await
        .context("read xAI subscriptions response")?;
    ensure!(
        bytes.len() <= MAX_RESPONSE_BYTES,
        "xAI subscriptions response is too large"
    );
    let body: Value = serde_json::from_slice(&bytes).context("decode xAI subscriptions")?;
    Ok(active_plan(&body))
}

async fn fetch_remaining_resets(
    client: &reqwest::Client,
    credential: &Credential,
) -> Result<Vec<ResetCredit>> {
    let message = grpc_web_unary(client, credential, GET_RESETS_PATH, Vec::new()).await?;
    let now_seconds = crate::usage::now_ms().div_euclid(1_000);
    Ok(decode_reset_tokens(&message, 10)?
        .into_iter()
        .filter(|credit| credit.expires_at.is_none_or(|expiry| expiry > now_seconds))
        .collect())
}

async fn grpc_web_unary(
    client: &reqwest::Client,
    credential: &Credential,
    path: &str,
    message: Vec<u8>,
) -> Result<Vec<u8>> {
    ensure!(
        message.len() <= MAX_RESPONSE_BYTES,
        "xAI RPC request is too large"
    );
    let mut headers = auth_headers(credential)?;
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/grpc-web+proto"),
    );
    headers.insert("x-grpc-web", HeaderValue::from_static("1"));
    let response = client
        .post(format!("{GROK_WEB_BASE}{path}"))
        .headers(headers)
        .body(frame_message(&message))
        .send()
        .await
        .with_context(|| format!("request xAI RPC {path}"))?;
    ensure!(
        response.status().is_success(),
        "xAI RPC returned HTTP {}",
        response.status().as_u16()
    );
    let length = response.content_length().unwrap_or(0);
    ensure!(
        length <= u64::try_from(MAX_RESPONSE_BYTES).unwrap_or(u64::MAX),
        "xAI RPC response is too large"
    );
    let body = response.bytes().await.context("read xAI RPC response")?;
    ensure!(
        body.len() <= MAX_RESPONSE_BYTES,
        "xAI RPC response is too large"
    );
    decode_grpc_web_response(&body)
}

fn auth_headers(credential: &Credential) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", credential.key))
            .context("encode xAI authorization header")?,
    );
    headers.insert("x-xai-token-auth", HeaderValue::from_static("xai-grok-cli"));
    headers.insert(
        "x-userid",
        HeaderValue::from_str(&credential.user_id).context("encode xAI user header")?,
    );
    Ok(headers)
}

fn load_credential() -> Result<Credential> {
    let serialized = match std::env::var("GROK_AUTH") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            let path = grok_auth_path().context("Grok auth.json path is unavailable")?;
            std::fs::read_to_string(&path)
                .with_context(|| format!("read Grok credentials from {}", path.display()))?
        }
    };
    let value: Value = serde_json::from_str(&serialized).context("parse Grok credentials")?;
    credential_from_json(&value).context("official Grok OIDC credential is unavailable")
}

fn grok_auth_path() -> Option<PathBuf> {
    std::env::var_os("GROK_AUTH_PATH")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("GROK_HOME").map(|home| PathBuf::from(home).join("auth.json")))
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".grok/auth.json"))
        })
}

fn credential_from_json(value: &Value) -> Option<Credential> {
    fn parse(value: &Value) -> Option<Credential> {
        let object = value.as_object()?;
        let key = object.get("key")?.as_str()?.trim();
        let user_id = object.get("user_id")?.as_str()?.trim();
        let auth_mode = object.get("auth_mode").and_then(Value::as_str);
        if key.is_empty() || user_id.is_empty() || auth_mode != Some("oidc") {
            return None;
        }
        Some(Credential {
            key: key.to_owned(),
            user_id: user_id.to_owned(),
        })
    }

    parse(value).or_else(|| value.as_object()?.values().find_map(parse))
}

fn active_plan(value: &Value) -> Option<String> {
    value
        .get("subscriptions")?
        .as_array()?
        .iter()
        .filter(|subscription| {
            subscription.get("status").and_then(Value::as_str) == Some("SUBSCRIPTION_STATUS_ACTIVE")
        })
        .filter_map(|subscription| subscription.get("tier").and_then(Value::as_str))
        .filter_map(|tier| plan_label(tier).map(|label| (plan_rank(tier), label)))
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, label)| label.to_owned())
}

fn plan_label(tier: &str) -> Option<&'static str> {
    match tier {
        "SUBSCRIPTION_TIER_SUPER_GROK_LITE" => Some("SuperGrok Lite"),
        "SUBSCRIPTION_TIER_GROK_PRO" => Some("SuperGrok"),
        "SUBSCRIPTION_TIER_SUPER_GROK_PLUS" => Some("SuperGrok Plus"),
        "SUBSCRIPTION_TIER_SUPER_GROK_PRO" => Some("SuperGrok Heavy"),
        _ => None,
    }
}

fn plan_rank(tier: &str) -> u8 {
    match tier {
        "SUBSCRIPTION_TIER_SUPER_GROK_LITE" => 1,
        "SUBSCRIPTION_TIER_GROK_PRO" => 2,
        "SUBSCRIPTION_TIER_SUPER_GROK_PLUS" => 3,
        "SUBSCRIPTION_TIER_SUPER_GROK_PRO" => 4,
        _ => 0,
    }
}

fn frame_message(message: &[u8]) -> Vec<u8> {
    let length = u32::try_from(message.len()).expect("bounded xAI RPC request length fits u32");
    let mut framed = Vec::with_capacity(message.len().saturating_add(5));
    framed.push(0);
    framed.extend_from_slice(&length.to_be_bytes());
    framed.extend_from_slice(message);
    framed
}

fn decode_grpc_web_response(body: &[u8]) -> Result<Vec<u8>> {
    let mut offset = 0;
    let mut message = None;
    let mut grpc_status = None;
    while offset < body.len() {
        ensure!(
            body.len().saturating_sub(offset) >= 5,
            "truncated xAI RPC envelope"
        );
        let flags = body[offset];
        let length = usize::try_from(u32::from_be_bytes(
            body[offset + 1..offset + 5]
                .try_into()
                .expect("four-byte envelope length"),
        ))
        .context("xAI RPC envelope length does not fit usize")?;
        offset += 5;
        let end = offset
            .checked_add(length)
            .context("xAI RPC envelope length overflow")?;
        ensure!(end <= body.len(), "truncated xAI RPC envelope payload");
        if flags == 0 {
            ensure!(
                message.is_none(),
                "xAI unary RPC returned multiple messages"
            );
            message = Some(body[offset..end].to_vec());
        } else if flags == 0x80 {
            grpc_status = trailer_status(&body[offset..end]);
        } else {
            bail!("xAI RPC returned unsupported envelope flags {flags}");
        }
        offset = end;
    }
    ensure!(
        grpc_status == Some(0),
        "xAI RPC returned gRPC status {}",
        grpc_status.unwrap_or(-1)
    );
    message.context("xAI unary RPC returned no message")
}

fn trailer_status(trailer: &[u8]) -> Option<i32> {
    std::str::from_utf8(trailer).ok()?.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("grpc-status")
            .then(|| value.trim().parse().ok())
            .flatten()
    })
}

#[derive(Clone, Copy)]
enum WireValue<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed32,
    Fixed64,
}

#[derive(Clone, Copy)]
struct WireField<'a> {
    number: u32,
    value: WireValue<'a>,
}

fn decode_fields(mut bytes: &[u8]) -> Result<Vec<WireField<'_>>> {
    let mut fields = Vec::new();
    while !bytes.is_empty() {
        let (tag, tag_len) = decode_varint(bytes)?;
        bytes = &bytes[tag_len..];
        let number = u32::try_from(tag >> 3).context("protobuf field number overflow")?;
        ensure!(number > 0, "protobuf field number is zero");
        let value = match tag & 7 {
            0 => {
                let (value, length) = decode_varint(bytes)?;
                bytes = &bytes[length..];
                WireValue::Varint(value)
            }
            1 => {
                ensure!(bytes.len() >= 8, "truncated fixed64 field");
                bytes = &bytes[8..];
                WireValue::Fixed64
            }
            2 => {
                let (length, prefix) = decode_varint(bytes)?;
                bytes = &bytes[prefix..];
                let length = usize::try_from(length).context("protobuf length overflow")?;
                ensure!(bytes.len() >= length, "truncated length-delimited field");
                let value = &bytes[..length];
                bytes = &bytes[length..];
                WireValue::Bytes(value)
            }
            5 => {
                ensure!(bytes.len() >= 4, "truncated fixed32 field");
                bytes = &bytes[4..];
                WireValue::Fixed32
            }
            wire => bail!("unsupported protobuf wire type {wire}"),
        };
        fields.push(WireField { number, value });
    }
    Ok(fields)
}

fn decode_varint(bytes: &[u8]) -> Result<(u64, usize)> {
    let mut value = 0_u64;
    for (index, byte) in bytes.iter().copied().take(10).enumerate() {
        let shift = u32::try_from(index * 7).expect("varint shift fits u32");
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    bail!("truncated or oversized protobuf varint")
}

fn decode_reset_tokens(message: &[u8], repeated_field: u32) -> Result<Vec<ResetCredit>> {
    decode_fields(message)?
        .into_iter()
        .filter_map(|field| match field {
            WireField {
                number,
                value: WireValue::Bytes(value),
            } if number == repeated_field => Some(decode_reset_token(value)),
            _ => None,
        })
        .collect()
}

fn decode_reset_token(message: &[u8]) -> Result<ResetCredit> {
    let fields = decode_fields(message)?;
    let id = fields
        .iter()
        .find_map(|field| match field {
            WireField {
                number: 10,
                value: WireValue::Bytes(value),
            } => std::str::from_utf8(value).ok(),
            _ => None,
        })
        .filter(|value| !value.is_empty())
        .context("xAI reset token has no id")?
        .to_owned();
    let timestamp = |number| {
        fields.iter().find_map(|field| match field {
            WireField {
                number: field_number,
                value: WireValue::Bytes(value),
            } if *field_number == number => decode_timestamp(value).ok(),
            _ => None,
        })
    };
    Ok(ResetCredit {
        id,
        granted_at: timestamp(20),
        expires_at: timestamp(30),
    })
}

fn decode_timestamp(message: &[u8]) -> Result<i64> {
    let seconds = decode_fields(message)?
        .into_iter()
        .find_map(|field| match field {
            WireField {
                number: 1,
                value: WireValue::Varint(value),
            } => Some(value),
            _ => None,
        })
        .unwrap_or(0);
    i64::try_from(seconds).context("xAI reset timestamp overflow")
}

fn encode_string_field(number: u32, value: &str) -> Vec<u8> {
    let mut encoded = Vec::new();
    encode_varint(u64::from(number) << 3 | 2, &mut encoded);
    encode_varint(u64::try_from(value.len()).unwrap_or(u64::MAX), &mut encoded);
    encoded.extend_from_slice(value.as_bytes());
    encoded
}

fn encode_varint(mut value: u64, target: &mut Vec<u8>) {
    loop {
        let mut byte = u8::try_from(value & 0x7f).expect("seven bits fit u8");
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        target.push(byte);
        if value == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn timestamp(seconds: u64) -> Vec<u8> {
        let mut encoded = Vec::new();
        encode_varint(1 << 3, &mut encoded);
        encode_varint(seconds, &mut encoded);
        encoded
    }

    fn length_field(number: u32, value: &[u8], target: &mut Vec<u8>) {
        encode_varint(u64::from(number) << 3 | 2, target);
        encode_varint(u64::try_from(value.len()).unwrap(), target);
        target.extend_from_slice(value);
    }

    fn reset_token(id: &str, start: u64, end: u64) -> Vec<u8> {
        let mut token = encode_string_field(10, id);
        length_field(20, &timestamp(start), &mut token);
        length_field(30, &timestamp(end), &mut token);
        token
    }

    #[test]
    fn active_subscription_overrides_the_incorrect_free_billing_tier() {
        let subscriptions = json!({
            "subscriptions": [
                { "tier": "SUBSCRIPTION_TIER_GROK_PRO", "status": "SUBSCRIPTION_STATUS_INACTIVE" },
                { "tier": "SUBSCRIPTION_TIER_GROK_PRO", "status": "SUBSCRIPTION_STATUS_ACTIVE" }
            ]
        });
        assert_eq!(active_plan(&subscriptions).as_deref(), Some("SuperGrok"));
    }

    #[test]
    fn credential_reader_accepts_the_official_scoped_store() {
        let store = json!({
            "https://auth.x.ai::client": {
                "key": "secret",
                "user_id": "user",
                "auth_mode": "oidc"
            }
        });
        let credential = credential_from_json(&store).expect("credential");
        assert_eq!(credential.user_id, "user");
    }

    #[test]
    fn remaining_reset_response_uses_the_published_field_numbers() {
        let mut response = Vec::new();
        length_field(10, &reset_token("reset-a", 100, 200), &mut response);
        length_field(10, &reset_token("reset-b", 300, 400), &mut response);
        let credits = decode_reset_tokens(&response, 10).expect("decode resets");
        assert_eq!(credits.len(), 2);
        assert_eq!(credits[0].id, "reset-a");
        assert_eq!(credits[0].granted_at, Some(100));
        assert_eq!(credits[0].expires_at, Some(200));
    }

    #[test]
    fn redeem_request_encodes_token_id_as_field_ten() {
        assert_eq!(
            encode_string_field(10, "reset-a"),
            [0x52, 0x07, b'r', b'e', b's', b'e', b't', b'-', b'a']
        );
    }

    #[test]
    fn grpc_web_decoder_requires_a_success_trailer() {
        let mut response = frame_message(b"payload");
        let trailer = b"grpc-status: 0\r\n";
        response.push(0x80);
        response.extend_from_slice(&u32::try_from(trailer.len()).unwrap().to_be_bytes());
        response.extend_from_slice(trailer);
        assert_eq!(decode_grpc_web_response(&response).unwrap(), b"payload");
    }

    #[test]
    fn wire_decoder_skips_supported_unknown_scalar_fields() {
        let fields = decode_fields(&[0x0d, 1, 2, 3, 4, 0x11, 1, 2, 3, 4, 5, 6, 7, 8])
            .expect("decode fixed fields");
        assert!(matches!(fields[0].value, WireValue::Fixed32));
        assert!(matches!(fields[1].value, WireValue::Fixed64));
    }
}
