//! Jellyfin Sessions message validation and conversion to bounded playback commands.

use serde_json::Value;
use std::collections::VecDeque;

const CAPABILITIES_BODY: &[u8] = br#"{
    "PlayableMediaTypes":["Video"],
    "SupportsMediaControl":true,
    "SupportsPersistentIdentifier":true,
    "SupportedCommands":["Play","Playstate"]
}"#;

pub(crate) fn capabilities_body() -> &'static [u8] {
    CAPABILITIES_BODY
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlayRequest {
    pub(super) item_id: String,
    pub(super) media_source_id: Option<String>,
    pub(super) start_position_ns: u64,
    pub(super) audio_stream_index: Option<i32>,
    pub(super) subtitle_stream_index: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Action {
    Play(PlayRequest),
    Playstate(Playstate),
    ForceKeepAlive(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Playstate {
    Pause,
    Unpause,
    PlayPause,
    Stop,
    Seek(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProtocolError {
    Malformed,
    InvalidCommand,
    PositionOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParsedMessage {
    pub(super) message_id: Option<String>,
    pub(super) action: Option<Action>,
}

#[derive(Default)]
pub(super) struct Deduper {
    recent: VecDeque<String>,
}

impl Deduper {
    pub(super) fn accept(&mut self, message: &ParsedMessage) -> bool {
        let Some(id) = message.message_id.as_ref() else {
            return true;
        };
        if self.recent.contains(id) {
            return false;
        }
        self.recent.push_back(id.clone());
        if self.recent.len() > 64 {
            self.recent.pop_front();
        }
        true
    }
}

pub(super) fn parse(bytes: &[u8]) -> Result<Option<Action>, ProtocolError> {
    parse_detailed(bytes).map(|message| message.action)
}

pub(super) fn parse_detailed(bytes: &[u8]) -> Result<ParsedMessage, ProtocolError> {
    let message: Value = serde_json::from_slice(bytes).map_err(|_| ProtocolError::Malformed)?;
    let message_type = message
        .get("MessageType")
        .and_then(Value::as_str)
        .ok_or(ProtocolError::Malformed)?;
    let message_id = match message.get("MessageId") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) => Some(id.clone()),
        Some(Value::Number(id)) => Some(id.to_string()),
        _ => return Err(ProtocolError::Malformed),
    };
    let action = match message_type {
        "Play" => parse_play(message.get("Data")),
        "Playstate" => parse_playstate(message.get("Data")),
        "ForceKeepAlive" => {
            let seconds = message
                .get("Data")
                .and_then(Value::as_u64)
                .filter(|seconds| (1..=3600).contains(seconds))
                .ok_or(ProtocolError::InvalidCommand)?;
            Ok(Some(Action::ForceKeepAlive(seconds)))
        }
        _ => Ok(None),
    }?;
    Ok(ParsedMessage { message_id, action })
}

fn parse_playstate(data: Option<&Value>) -> Result<Option<Action>, ProtocolError> {
    let data = data.and_then(Value::as_object).ok_or(ProtocolError::InvalidCommand)?;
    let command = data
        .get("Command")
        .and_then(Value::as_str)
        .ok_or(ProtocolError::InvalidCommand)?;
    let state = match command {
        "Pause" => Playstate::Pause,
        "Unpause" => Playstate::Unpause,
        "PlayPause" => Playstate::PlayPause,
        "Stop" => Playstate::Stop,
        "Seek" => {
            let position = data
                .get("SeekPositionTicks")
                .and_then(Value::as_u64)
                .and_then(|ticks| ticks.checked_mul(100))
                .ok_or(ProtocolError::PositionOverflow)?;
            Playstate::Seek(position)
        }
        _ => return Ok(None),
    };
    Ok(Some(Action::Playstate(state)))
}

fn parse_play(data: Option<&Value>) -> Result<Option<Action>, ProtocolError> {
    let data = data.and_then(Value::as_object).ok_or(ProtocolError::InvalidCommand)?;
    if data.get("PlayCommand").and_then(Value::as_str) != Some("PlayNow") {
        return Ok(None);
    }
    let items = data
        .get("ItemIds")
        .and_then(Value::as_array)
        .ok_or(ProtocolError::InvalidCommand)?;
    let start_index = match data.get("StartIndex") {
        None | Some(Value::Null) => 0,
        Some(value) => value
            .as_u64()
            .and_then(|index| usize::try_from(index).ok())
            .ok_or(ProtocolError::InvalidCommand)?,
    };
    let item_id = items
        .get(start_index)
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
        .ok_or(ProtocolError::InvalidCommand)?
        .to_owned();
    let media_source_id = match data.get("MediaSourceId") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_str()
                .filter(|id| valid_id(id))
                .ok_or(ProtocolError::InvalidCommand)?
                .to_owned(),
        ),
    };
    let start_position_ns = match data.get("StartPositionTicks") {
        None | Some(Value::Null) => 0,
        Some(value) => value
            .as_u64()
            .and_then(|ticks| ticks.checked_mul(100))
            .ok_or(ProtocolError::PositionOverflow)?,
    };
    let audio_stream_index = optional_i32(data.get("AudioStreamIndex"))?;
    let subtitle_stream_index = optional_i32(data.get("SubtitleStreamIndex"))?;
    Ok(Some(Action::Play(PlayRequest {
        item_id,
        media_source_id,
        start_position_ns,
        audio_stream_index,
        subtitle_stream_index,
    })))
}

fn optional_i32(value: Option<&Value>) -> Result<Option<i32>, ProtocolError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .and_then(|number| i32::try_from(number).ok())
            .map(Some)
            .ok_or(ProtocolError::InvalidCommand),
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}
