//! mpv's JSON IPC: one JSON object per line in each direction.

use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Eof,
    Stop,
    Error,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MpvEvent {
    FileLoaded,
    PlaybackRestart,
    EndFile {
        reason: EndReason,
        error: Option<String>,
    },
    Pause(bool),
    /// Seconds. `None` while nothing is loaded.
    Duration(Option<f64>),
    Buffering(bool),
    Exited,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Reply {
        request_id: u64,
        result: Result<Value, String>,
    },
    Event(MpvEvent),
    Ignored,
}

/// One command as a newline-terminated JSON line.
pub fn encode(request_id: Option<u64>, command: &[Value]) -> String {
    let mut object = json!({ "command": command });

    if let Some(id) = request_id {
        object["request_id"] = id.into();
    }

    format!("{object}\n")
}

pub fn loadfile(url: &str) -> Vec<Value> {
    vec!["loadfile".into(), url.into(), "replace".into()]
}

pub fn stop() -> Vec<Value> {
    vec!["stop".into()]
}

pub fn seek(position_ms: u64) -> Vec<Value> {
    vec![
        "seek".into(),
        (position_ms as f64 / 1000.0).into(),
        "absolute".into(),
    ]
}

pub fn set_property(name: &str, value: impl Into<Value>) -> Vec<Value> {
    vec!["set_property".into(), name.into(), value.into()]
}

pub fn get_property(name: &str) -> Vec<Value> {
    vec!["get_property".into(), name.into()]
}

pub fn observe_property(id: u64, name: &str) -> Vec<Value> {
    vec!["observe_property".into(), id.into(), name.into()]
}

pub fn parse_line(line: &str) -> Incoming {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Incoming::Ignored;
    };

    if let Some(event) = value.get("event").and_then(Value::as_str) {
        return parse_event(event, &value).map_or(Incoming::Ignored, Incoming::Event);
    }

    let Some(request_id) = value.get("request_id").and_then(Value::as_u64) else {
        return Incoming::Ignored;
    };
    let status = value
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("success");
    let result = if status == "success" {
        Ok(value.get("data").cloned().unwrap_or(Value::Null))
    } else {
        Err(status.to_owned())
    };

    Incoming::Reply { request_id, result }
}

fn parse_event(name: &str, value: &Value) -> Option<MpvEvent> {
    match name {
        "file-loaded" => Some(MpvEvent::FileLoaded),
        "playback-restart" => Some(MpvEvent::PlaybackRestart),
        "shutdown" => Some(MpvEvent::Exited),
        "end-file" => {
            let reason = match value.get("reason").and_then(Value::as_str) {
                Some("eof") => EndReason::Eof,
                Some("stop") => EndReason::Stop,
                Some("error") => EndReason::Error,
                _ => EndReason::Other,
            };

            Some(MpvEvent::EndFile {
                reason,
                error: value
                    .get("file_error")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        }
        "property-change" => {
            let data = value.get("data");

            match value.get("name").and_then(Value::as_str)? {
                "pause" => Some(MpvEvent::Pause(data?.as_bool()?)),
                "paused-for-cache" => Some(MpvEvent::Buffering(data?.as_bool()?)),
                "duration" => Some(MpvEvent::Duration(data.and_then(Value::as_f64))),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_single_json_lines() {
        let line = encode(Some(7), &loadfile("https://x.example/a"));

        assert_eq!(
            line,
            "{\"command\":[\"loadfile\",\"https://x.example/a\",\"replace\"],\"request_id\":7}\n"
        );
        assert_eq!(
            encode(None, &observe_property(1, "pause")),
            "{\"command\":[\"observe_property\",1,\"pause\"]}\n"
        );
    }

    #[test]
    fn a_url_with_quotes_stays_one_argument() {
        let line = encode(Some(1), &loadfile("https://x.example/\"a\"\n"));

        assert_eq!(line.matches('\n').count(), 1);
        assert!(line.contains("\\\"a\\\"\\n"));
    }

    #[test]
    fn builds_the_verified_commands() {
        assert_eq!(
            encode(Some(1), &seek(90_500)),
            "{\"command\":[\"seek\",90.5,\"absolute\"],\"request_id\":1}\n"
        );
        assert_eq!(
            encode(Some(2), &set_property("pause", true)),
            "{\"command\":[\"set_property\",\"pause\",true],\"request_id\":2}\n"
        );
        assert_eq!(
            encode(Some(3), &set_property("volume", 40)),
            "{\"command\":[\"set_property\",\"volume\",40],\"request_id\":3}\n"
        );
        assert_eq!(
            encode(Some(4), &get_property("time-pos")),
            "{\"command\":[\"get_property\",\"time-pos\"],\"request_id\":4}\n"
        );
        assert_eq!(
            encode(Some(5), &stop()),
            "{\"command\":[\"stop\"],\"request_id\":5}\n"
        );
    }

    #[test]
    fn parses_replies() {
        assert_eq!(
            parse_line(r#"{"request_id":3,"error":"success","data":12.5}"#),
            Incoming::Reply {
                request_id: 3,
                result: Ok(json!(12.5)),
            }
        );
        assert_eq!(
            parse_line(r#"{"request_id":4,"error":"success"}"#),
            Incoming::Reply {
                request_id: 4,
                result: Ok(Value::Null),
            }
        );
        assert_eq!(
            parse_line(r#"{"request_id":5,"error":"property unavailable"}"#),
            Incoming::Reply {
                request_id: 5,
                result: Err("property unavailable".into()),
            }
        );
    }

    #[test]
    fn parses_the_observed_event_stream() {
        let events = [
            (r#"{"event":"start-file","playlist_entry_id":1}"#, None),
            (r#"{"event":"file-loaded"}"#, Some(MpvEvent::FileLoaded)),
            (
                r#"{"event":"property-change","id":2,"name":"duration","data":276.5}"#,
                Some(MpvEvent::Duration(Some(276.5))),
            ),
            (
                r#"{"event":"playback-restart"}"#,
                Some(MpvEvent::PlaybackRestart),
            ),
            (
                r#"{"event":"property-change","id":1,"name":"pause","data":true}"#,
                Some(MpvEvent::Pause(true)),
            ),
            (
                r#"{"event":"property-change","id":3,"name":"paused-for-cache","data":false}"#,
                Some(MpvEvent::Buffering(false)),
            ),
            (
                r#"{"event":"property-change","id":2,"name":"duration"}"#,
                Some(MpvEvent::Duration(None)),
            ),
            (
                r#"{"event":"end-file","reason":"eof"}"#,
                Some(MpvEvent::EndFile {
                    reason: EndReason::Eof,
                    error: None,
                }),
            ),
            (
                r#"{"event":"end-file","reason":"stop","playlist_entry_id":1}"#,
                Some(MpvEvent::EndFile {
                    reason: EndReason::Stop,
                    error: None,
                }),
            ),
            (
                r#"{"event":"end-file","reason":"error","file_error":"loading failed"}"#,
                Some(MpvEvent::EndFile {
                    reason: EndReason::Error,
                    error: Some("loading failed".into()),
                }),
            ),
            (
                r#"{"event":"end-file","reason":"quit"}"#,
                Some(MpvEvent::EndFile {
                    reason: EndReason::Other,
                    error: None,
                }),
            ),
            (r#"{"event":"shutdown"}"#, Some(MpvEvent::Exited)),
        ];

        for (line, expected) in events {
            let parsed = parse_line(line);

            match expected {
                Some(event) => assert_eq!(parsed, Incoming::Event(event), "{line}"),
                None => assert_eq!(parsed, Incoming::Ignored, "{line}"),
            }
        }
    }

    #[test]
    fn unknown_or_broken_lines_are_ignored() {
        for line in [
            "",
            "not json",
            r#"{"event":"audio-reconfig"}"#,
            r#"{"event":"property-change","id":9,"name":"volume","data":50}"#,
            r#"{"event":"property-change","id":1,"name":"pause","data":"x"}"#,
            r#"{"error":"success"}"#,
        ] {
            assert_eq!(parse_line(line), Incoming::Ignored, "{line}");
        }
    }
}
