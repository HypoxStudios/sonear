use sonear_soloist::protocol::{Command, Event, PlayStatus, RepeatMode};

fn parse(text: &str) -> Event {
    Event::parse(text).expect("fixture parses")
}

#[test]
fn auth_state() {
    let event = parse(include_str!("fixtures/auth_state.json"));

    assert_eq!(
        event,
        Event::AuthState {
            logged_in: true,
            is_active: false,
            device_name: "Sonear Test".into()
        }
    );
}

#[test]
fn playback_state() {
    let Event::PlaybackState(state) = parse(include_str!("fixtures/playback_state.json")) else {
        panic!("wrong variant");
    };
    let item = state.item.expect("item");
    let position = state.position.expect("position");

    assert_eq!(state.status, PlayStatus::Playing);
    assert_eq!(item.name(), "Instant Crush (feat. Julian Casablancas)");
    assert_eq!(item.artist_names(), ["Daft Punk", "Julian Casablancas"]);
    assert_eq!(
        item.album().map(|a| a.name()),
        Some("Random Access Memories")
    );
    assert_eq!(item.duration_ms(), Some(337560));
    assert!(item.cover_url("large").unwrap().contains("0000b273"));
    assert!(item.cover_url("small").unwrap().contains("00004851"));
    assert_eq!(position.position_ms, 94053);
    assert_eq!(position.speed, 1.0);
    assert_eq!(state.volume, Some(30));
    assert_eq!(state.options.expect("options").repeat, RepeatMode::Off);
    assert_eq!(state.available_actions["seek_forward"].step_ms, Some(15000));
    assert!(state.available_actions.contains_key("pause"));
    assert!(!state.available_actions.contains_key("play"));
    assert_eq!(state.context.expect("context").entity_type, "album");
}

#[test]
fn cover_falls_back_to_closest_size() {
    let Event::PlaybackState(state) = parse(include_str!("fixtures/playback_state.json")) else {
        panic!("wrong variant");
    };
    let context = state.context.expect("context");

    assert!(context.cover_url("xlarge").unwrap().contains("00001e02"));
    assert!(context.cover_url("nonsense").is_some());
}

#[test]
fn queue_changed() {
    let Event::QueueChanged { previous, upcoming } =
        parse(include_str!("fixtures/queue_changed.json"))
    else {
        panic!("wrong variant");
    };

    assert_eq!(previous.len(), 3);
    assert_eq!(upcoming.len(), 3);
    assert_eq!(previous[0].item.name(), "Within");
    assert_eq!(previous[0].source, "context");
}

#[test]
fn small_events() {
    assert_eq!(
        parse(include_str!("fixtures/command_result.json")),
        Event::CommandResult {
            command: "set_volume".into()
        }
    );
    assert_eq!(
        parse(include_str!("fixtures/playback_changed.json")),
        Event::PlaybackChanged {
            status: PlayStatus::Paused
        }
    );
    assert_eq!(
        parse(include_str!("fixtures/device_changed.json")),
        Event::DeviceChanged {
            is_active: true,
            device_name: "Sonear Test".into()
        }
    );
    assert_eq!(
        parse(include_str!("fixtures/error.json")),
        Event::Error {
            message: "unknown command: not_a_command".into()
        }
    );

    let Event::PositionSync { position } = parse(include_str!("fixtures/position_sync.json"))
    else {
        panic!("wrong variant");
    };

    assert_eq!(position.position_ms, 102892);
    assert_eq!(position.speed, 0.0);

    let Event::TrackChanged { item } = parse(include_str!("fixtures/track_changed.json")) else {
        panic!("wrong variant");
    };

    assert_eq!(
        item.expect("item").name(),
        "Lose Yourself to Dance (feat. Pharrell Williams)"
    );
}

#[test]
fn documented_but_uncaptured_events() {
    assert_eq!(
        parse(r#"{"type":"volume_changed","volume":55}"#),
        Event::VolumeChanged { volume: 55 }
    );
    assert!(matches!(
        parse(r#"{"type":"options_changed","options":{"shuffle":true,"repeat":"track"}}"#),
        Event::OptionsChanged { options } if options.shuffle && options.repeat == RepeatMode::Track
    ));
    assert!(matches!(
        parse(r#"{"type":"context_changed","context":{"uri":"spotify:album:x"}}"#),
        Event::ContextChanged { context: Some(c) } if c.uri == "spotify:album:x"
    ));
}

#[test]
fn unknown_types_and_extra_fields_do_not_fail() {
    assert_eq!(
        parse(r#"{"type":"something_new","a":1,"b":{"c":2}}"#),
        Event::Unknown
    );
    assert_eq!(
        parse(r#"{"type":"playback_changed","status":"paused","brand_new":true}"#),
        Event::PlaybackChanged {
            status: PlayStatus::Paused
        }
    );
    assert_eq!(
        parse(r#"{"type":"playback_changed","status":"teleporting"}"#),
        Event::PlaybackChanged {
            status: PlayStatus::Stopped
        }
    );
    assert!(matches!(
        parse(r#"{"type":"playback_state","item":{"uri":"x","decorations":{"surprise":1}}}"#),
        Event::PlaybackState(_)
    ));
}

#[test]
fn commands_serialise_to_wire_shape() {
    let cases = [
        (Command::Pause, r#"{"type":"command","command":"pause"}"#),
        (
            Command::Seek { position_ms: 5000 },
            r#"{"type":"command","command":"seek","position_ms":5000}"#,
        ),
        (
            Command::Play { uri: None },
            r#"{"type":"command","command":"play"}"#,
        ),
        (
            Command::Play {
                uri: Some("spotify:track:x".into()),
            },
            r#"{"type":"command","command":"play","uri":"spotify:track:x"}"#,
        ),
        (
            Command::SetRepeatTrack { enabled: true },
            r#"{"type":"command","command":"set_repeat_track","enabled":true}"#,
        ),
        (
            Command::GetQueue { limit: Some(5) },
            r#"{"type":"command","command":"get_queue","limit":5}"#,
        ),
    ];

    for (command, expected) in cases {
        let actual: serde_json::Value = serde_json::from_str(&command.to_json()).unwrap();
        let expected: serde_json::Value = serde_json::from_str(expected).unwrap();

        assert_eq!(actual, expected);
    }
}
