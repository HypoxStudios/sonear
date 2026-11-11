use serde_json::json;
use tokio::io::{DuplexStream, Lines, ReadHalf, WriteHalf};

use super::*;
use crate::mpv::protocol::EndReason;

struct Server {
    lines: Lines<BufReader<ReadHalf<DuplexStream>>>,
    writer: WriteHalf<DuplexStream>,
}

impl Server {
    async fn next_command(&mut self) -> Value {
        let line = self.lines.next_line().await.unwrap().unwrap();

        serde_json::from_str(&line).unwrap()
    }

    async fn send(&mut self, line: &str) {
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn reply(&mut self, request: &Value, data: Value) {
        let id = request["request_id"].as_u64().unwrap();

        self.send(&json!({ "request_id": id, "error": "success", "data": data }).to_string())
            .await;
    }

    async fn fail(&mut self, request: &Value, error: &str) {
        let id = request["request_id"].as_u64().unwrap();

        self.send(&json!({ "request_id": id, "error": error }).to_string())
            .await;
    }
}

async fn connected() -> (MpvClient, Server) {
    connected_with(COMMAND_TIMEOUT).await
}

async fn connected_with(timeout: Duration) -> (MpvClient, Server) {
    let (ours, theirs) = tokio::io::duplex(8192);
    let client = MpvClient::connect_with_timeout(ours, timeout).await;
    let (reader, writer) = tokio::io::split(theirs);
    let mut server = Server {
        lines: BufReader::new(reader).lines(),
        writer,
    };

    for (id, name) in [(1, "pause"), (2, "duration"), (3, "paused-for-cache")] {
        let command = server.next_command().await;

        assert_eq!(command["command"], json!(["observe_property", id, name]));
        assert!(command.get("request_id").is_none());
    }

    (client, server)
}

#[tokio::test]
async fn commands_get_their_reply() {
    let (client, mut server) = connected().await;
    let call = client.set_pause(true);
    let answer = async {
        let command = server.next_command().await;

        assert_eq!(command["command"], json!(["set_property", "pause", true]));
        server.reply(&command, Value::Null).await;
    };
    let (result, ()) = tokio::join!(call, answer);

    assert_eq!(result, Ok(()));
}

#[tokio::test]
async fn load_seek_stop_and_volume_send_the_verified_commands() {
    let (client, mut server) = connected().await;
    let calls = async {
        client.load("https://x.example/a").await.unwrap();
        client.seek(1500).await.unwrap();
        client.set_volume(40).await.unwrap();
        client.stop().await.unwrap();
        client.set_audio_device("wasapi/{abc}").await.unwrap();
    };
    let answers = async {
        let expected = [
            json!(["loadfile", "https://x.example/a", "replace"]),
            json!(["seek", 1.5, "absolute"]),
            json!(["set_property", "volume", 40]),
            json!(["stop"]),
            json!(["set_property", "audio-device", "wasapi/{abc}"]),
        ];

        for command in expected {
            let received = server.next_command().await;

            assert_eq!(received["command"], command);
            server.reply(&received, Value::Null).await;
        }
    };

    tokio::join!(calls, answers);
}

#[tokio::test]
async fn an_error_reply_becomes_an_mpv_error() {
    let (client, mut server) = connected().await;
    let call = client.stop();
    let answer = async {
        let command = server.next_command().await;

        server.fail(&command, "property unavailable").await;
    };
    let (result, ()) = tokio::join!(call, answer);

    assert_eq!(result, Err(ToolError::Mpv("property unavailable".into())));
}

#[tokio::test]
async fn position_reads_time_pos_in_milliseconds() {
    let (client, mut server) = connected().await;
    let call = client.position_ms();
    let answer = async {
        let command = server.next_command().await;

        assert_eq!(command["command"], json!(["get_property", "time-pos"]));
        server.reply(&command, json!(12.3456)).await;
    };
    let (position, ()) = tokio::join!(call, answer);

    assert_eq!(position, Some(12346));

    let call = client.position_ms();
    let answer = async {
        let command = server.next_command().await;

        server.fail(&command, "property unavailable").await;
    };
    let (position, ()) = tokio::join!(call, answer);

    assert_eq!(position, None);
}

#[tokio::test]
async fn lists_audio_devices() {
    let (client, mut server) = connected().await;
    let call = client.audio_devices();
    let answer = async {
        let command = server.next_command().await;

        server
            .reply(
                &command,
                json!([
                    { "name": "auto", "description": "Autoselect device" },
                    { "name": "wasapi/{guid}", "description": "Speakers" },
                    { "description": "no name" },
                ]),
            )
            .await;
    };
    let (devices, ()) = tokio::join!(call, answer);

    assert_eq!(
        devices.unwrap(),
        vec![
            AudioDevice {
                name: "auto".into(),
                description: "Autoselect device".into(),
            },
            AudioDevice {
                name: "wasapi/{guid}".into(),
                description: "Speakers".into(),
            },
        ]
    );
}

#[tokio::test]
async fn replies_are_matched_by_request_id() {
    let (client, mut server) = connected().await;
    let first = client.request(protocol::get_property("a"));
    let second = client.request(protocol::get_property("b"));
    let answers = async {
        let one = server.next_command().await;
        let two = server.next_command().await;

        server.reply(&two, json!("second")).await;
        server.reply(&one, json!("first")).await;
    };
    let (first, second, ()) = tokio::join!(first, second, answers);

    assert_eq!(first, Ok(json!("first")));
    assert_eq!(second, Ok(json!("second")));
}

#[tokio::test]
async fn events_are_broadcast_in_order() {
    let (client, mut server) = connected().await;
    let mut events = client.events();

    for line in [
        r#"{"event":"start-file","playlist_entry_id":1}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"property-change","id":2,"name":"duration","data":276.5}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"event":"end-file","reason":"eof"}"#,
    ] {
        server.send(line).await;
    }

    assert_eq!(events.recv().await.unwrap(), MpvEvent::FileLoaded);
    assert_eq!(
        events.recv().await.unwrap(),
        MpvEvent::Duration(Some(276.5))
    );
    assert_eq!(events.recv().await.unwrap(), MpvEvent::PlaybackRestart);
    assert_eq!(
        events.recv().await.unwrap(),
        MpvEvent::EndFile {
            reason: EndReason::Eof,
            error: None,
        }
    );
}

#[tokio::test]
async fn hanging_up_fails_pending_and_future_commands() {
    let (client, server) = connected().await;
    let mut events = client.events();
    let pending = client.stop();
    let hang_up = async {
        tokio::task::yield_now().await;
        drop(server);
    };
    let (result, ()) = tokio::join!(pending, hang_up);

    assert_eq!(result, Err(ToolError::Disconnected));
    assert_eq!(events.recv().await.unwrap(), MpvEvent::Exited);
    assert_eq!(client.stop().await, Err(ToolError::Disconnected));
    assert_eq!(client.position_ms().await, None);
}

#[tokio::test(start_paused = true)]
async fn an_unanswered_command_times_out() {
    let (client, _server) = connected_with(Duration::from_secs(2)).await;

    assert_eq!(client.stop().await, Err(ToolError::Timeout));
}

#[tokio::test]
async fn only_http_addresses_are_loaded() {
    let (client, _server) = connected().await;

    for url in ["file:///C:/secret.mp3", "C:\\secret.mp3", "ytdl://abc", ""] {
        assert!(
            matches!(client.load(url).await, Err(ToolError::Mpv(_))),
            "{url}"
        );
    }
}
