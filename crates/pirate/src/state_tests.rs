//! What the plugin does, without a terminal to do it in.
//!
//! Every import is stubbed (see `sys`), so a whole poll cycle — ask for the
//! file, be handed it, ask for the reading, be handed that, come back in a
//! minute — runs here in microseconds and is asserted rather than watched.

use super::*;

use crate::sys::stub;

/// A credentials file with a session in it that does not expire.
const CREDENTIALS: &[u8] = br#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-x"}}"#;

/// What the endpoint answers for an account about half way through its window.
const USAGE: &[u8] = br#"{"five_hour":{"utilization":47.4,"resets_at":"2026-09-04T18:30:00Z"},
                          "seven_day":{"utilization":62.0}}"#;

/// A plugin that has built, with everything it asked for on the way taken.
fn built() -> (Pirate, i32) {
    stub::forget();
    let mut pirate = Pirate::new();
    pirate.build();
    let asked = stub::taken();
    let ticket = asked.requests[0].0;
    (pirate, ticket)
}

/// The same, carried all the way to a reading.
fn reading() -> Pirate {
    let (mut pirate, credentials) = built();
    pirate.deliver(
        credentials,
        Answer::Read {
            bytes: CREDENTIALS.to_vec(),
        },
    );
    let fetch = stub::taken().requests[0].0;
    pirate.deliver(
        fetch,
        Answer::Fetched {
            status: 200,
            body: USAGE.to_vec(),
        },
    );
    let _ = stub::taken();
    pirate
}

#[test]
fn building_registers_the_chip_and_starts_reading() {
    stub::forget();
    let mut pirate = Pirate::new();

    pirate.build();

    let asked = stub::taken();
    assert_eq!(
        asked.contributions,
        vec![(String::from("header.right"), String::from("chip"), 0)]
    );
    assert_eq!(
        asked.actions,
        vec![
            (
                String::from("refresh"),
                String::from("Refresh the usage reading")
            ),
            (String::from("panel"), String::from("Show the usage panel")),
            // Reachable and not offered: a zero-length title.
            (String::from("dismiss"), String::new()),
        ]
    );
    assert_eq!(
        asked.requests.first().map(|(_, request)| request.clone()),
        Some(Request::ReadFile {
            path: String::from(CREDENTIALS_PATH)
        })
    );
}

#[test]
fn the_session_it_reads_is_what_it_asks_claude_with() {
    let (mut pirate, credentials) = built();

    pirate.deliver(
        credentials,
        Answer::Read {
            bytes: CREDENTIALS.to_vec(),
        },
    );

    let asked = stub::taken();
    assert_eq!(
        asked.requests.first().map(|(_, request)| request.clone()),
        Some(Request::Fetch {
            method: Method::Get,
            url: String::from(USAGE_URL),
            headers: vec![
                (
                    String::from("authorization"),
                    String::from("Bearer sk-ant-oat01-x")
                ),
                (String::from("anthropic-beta"), String::from(OAUTH_BETA)),
            ],
            body: None,
        })
    );
}

#[test]
fn a_reading_that_lands_is_what_the_chip_then_says() {
    let pirate = reading();

    let reading = pirate.usable_reading().expect("a reading landed");
    assert_eq!(reading.session.percent, 47.4);
    assert_eq!(reading.weekly.expect("a weekly limit").percent, 62.0);
    assert_eq!(pirate.problem(), None);
}

#[test]
fn a_second_reading_reuses_the_token_rather_than_the_file() {
    // A poll a minute that opened a credentials file a minute would be a poll
    // that shows up in somebody's audit log.
    let mut pirate = reading();
    stub::advance(POLL_MILLIS);

    pirate.tick();

    let asked = stub::taken();
    assert!(
        matches!(
            asked.requests.first().map(|(_, request)| request),
            Some(Request::Fetch { .. })
        ),
        "{:?}",
        asked.requests
    );
}

#[test]
fn a_session_that_has_expired_is_read_from_the_file_again() {
    stub::forget();
    let mut pirate = Pirate::new();
    pirate.build();
    let credentials = stub::taken().requests[0].0;
    let expires_at = stub::_now_for_tests() + 1_000;
    pirate.deliver(
        credentials,
        Answer::Read {
            bytes: format!(r#"{{"claudeAiOauth":{{"accessToken":"x","expiresAt":{expires_at}}}}}"#)
                .into_bytes(),
        },
    );
    let fetch = stub::taken().requests[0].0;
    pirate.deliver(
        fetch,
        Answer::Fetched {
            status: 200,
            body: USAGE.to_vec(),
        },
    );
    let _ = stub::taken();

    stub::advance(POLL_MILLIS);
    pirate.tick();

    assert!(
        matches!(
            stub::taken().requests.first().map(|(_, request)| request),
            Some(Request::ReadFile { .. })
        ),
        "a token inside its last minute has to be read again, not sent"
    );
}

#[test]
fn a_machine_that_has_never_run_claude_code_says_so_and_asks_rarely() {
    let (mut pirate, credentials) = built();

    pirate.deliver(
        credentials,
        Answer::Failed(String::from("No such file or directory")),
    );

    assert_eq!(pirate.problem(), Some(&Problem::NoSession));
    assert_eq!(
        pirate.problem().expect("a problem").chip_label(),
        "no session"
    );

    // Nothing to poll until somebody runs Claude Code. The wait is asked for
    // by the next tick rather than by the answer, because a tick was already
    // coming and asking again would be asking twice.
    let _ = stub::taken();
    pirate.tick();
    assert_eq!(stub::taken().timers, vec![IDLE_POLL_MILLIS as i32]);
}

#[test]
fn a_plugin_nobody_has_allowed_says_what_to_allow() {
    let (mut pirate, credentials) = built();

    pirate.deliver(
        credentials,
        Answer::Refused(String::from("Read ~/.claude/.credentials.json")),
    );

    let problem = pirate.problem().expect("a refusal is a problem");
    assert_eq!(problem.chip_label(), "not allowed");
    assert!(
        problem
            .message()
            .contains("Read ~/.claude/.credentials.json"),
        "{}",
        problem.message()
    );
    // And it names where to go, because a plugin that cannot say that is a
    // chip that looks broken.
    assert!(
        problem.message().contains("Plugins"),
        "{}",
        problem.message()
    );
}

#[test]
fn a_blip_keeps_the_number_and_a_dead_session_replaces_it() {
    let mut pirate = reading();

    stub::advance(POLL_MILLIS);
    pirate.tick();
    let fetch = stub::taken().requests[0].0;
    pirate.deliver(fetch, Answer::Failed(String::from("connection reset")));

    assert!(
        pirate.usable_reading().is_some(),
        "a network blip is not a reason to throw away a number from a minute ago"
    );

    stub::advance(POLL_MILLIS);
    pirate.tick();
    let fetch = stub::taken().requests[0].0;
    pirate.deliver(
        fetch,
        Answer::Fetched {
            status: 401,
            body: Vec::new(),
        },
    );

    assert_eq!(pirate.problem(), Some(&Problem::SessionExpired));
    assert!(
        pirate.usable_reading().is_none(),
        "a percentage that will never be refreshed has to stop being shown"
    );
}

#[test]
fn a_background_poll_never_animates_and_a_click_always_does() {
    let mut pirate = reading();
    assert_eq!(pirate.mark(), PIRATE);

    stub::advance(POLL_MILLIS);
    pirate.tick();
    assert_eq!(pirate.mark(), PIRATE, "nobody asked for this one");
    let _ = stub::taken();

    pirate.run("refresh");
    assert_eq!(pirate.mark(), CHOMP_CYCLE[0]);
    pirate.tick();
    assert_eq!(pirate.mark(), CHOMP_CYCLE[1]);
    pirate.tick();
    assert_eq!(pirate.mark(), CHOMP_CYCLE[2]);
}

#[test]
fn the_bite_comes_back_round_to_a_whole_face() {
    let mut pirate = reading();
    pirate.run("refresh");

    let cycle: Vec<&str> = (0..CHOMP_CYCLE.len() + 1)
        .map(|_| {
            let frame = pirate.mark();
            pirate.tick();
            frame
        })
        .collect();

    assert_eq!(
        cycle,
        vec![
            "pirate",
            "pirate-open",
            "pirate-wide",
            "pirate-open",
            "pirate"
        ]
    );
}

#[test]
fn a_click_asks_to_be_woken_sooner_and_asks_once() {
    // The bug that shipped first: a plugin already waiting a minute for its
    // next poll never asked again, so the mark stood still for that minute
    // with a person watching it. And the bug on the other side of it: asking
    // again for a moment already booked is a heartbeat that doubles, then
    // quadruples, and a plugin polling Anthropic at a rising rate is an
    // account rate-limited.
    let mut pirate = reading();

    pirate.run("refresh");
    assert_eq!(
        stub::taken().timers,
        vec![CHOMP_MILLIS as i32],
        "a click wants the mark redrawn a frame from now, not a minute from now"
    );

    pirate.run("refresh");
    pirate.run("panel");
    assert!(
        stub::taken().timers.is_empty(),
        "the moment was already booked; asking for it again is asking twice"
    );

    pirate.tick();
    assert_eq!(
        stub::taken().timers,
        vec![CHOMP_MILLIS as i32],
        "and the tick that arrives books the next frame of the bite"
    );
}

#[test]
fn the_bite_ending_does_not_book_a_second_heartbeat() {
    // Coming back the other way: the answer lands, the mouth shuts, and the
    // next poll is a minute off — which is later than the tick already
    // coming, so nothing is asked for and the tick that arrives is the one
    // that books the minute.
    let mut pirate = reading();
    pirate.run("refresh");
    let _ = stub::taken();

    let fetch = stub::taken().requests.first().map(|(ticket, _)| *ticket);
    if let Some(ticket) = fetch {
        pirate.deliver(
            ticket,
            Answer::Fetched {
                status: 200,
                body: USAGE.to_vec(),
            },
        );
    }

    assert!(
        stub::taken().timers.is_empty(),
        "a wait that is further off than the one already booked is not worth asking for"
    );
}

#[test]
fn clicking_the_chip_twice_asks_claude_once() {
    let mut pirate = reading();

    pirate.run("panel");
    assert!(pirate.panel_open());
    let first = stub::taken().requests.len();
    pirate.run("panel");
    let second = stub::taken().requests.len();

    // Two: the reading, and the first page of the transcripts behind it.
    assert_eq!(first, 2, "opening it asks for a fresh reading and the week");
    assert_eq!(
        second, 0,
        "and putting it away asks for nothing: the figures must not move under a person dismissing them"
    );
    assert!(!pirate.panel_open());
}

#[test]
fn a_click_outside_the_panel_puts_it_away() {
    let mut pirate = reading();
    pirate.run("panel");

    pirate.run("dismiss");

    assert!(!pirate.panel_open());
}

#[test]
fn an_answer_nothing_is_waiting_on_changes_nothing() {
    let mut pirate = reading();
    let before = pirate.usable_reading().cloned();

    pirate.deliver(
        9_999,
        Answer::Fetched {
            status: 200,
            body: b"{}".to_vec(),
        },
    );

    assert_eq!(pirate.usable_reading().cloned(), before);
}

#[test]
fn building_again_forgets_everything_the_last_life_was_waiting_on() {
    // The host builds a plugin again when it is switched back on and when a
    // person answers what it asked to be allowed. Whatever the previous life
    // was waiting on — a timer nobody will fire again, a ticket nobody will
    // answer — has to go, or the plugin comes back permanently asleep.
    let mut pirate = reading();
    pirate.run("refresh");
    let _ = stub::taken();

    pirate = Pirate::new();
    pirate.build();

    let asked = stub::taken();
    assert_eq!(asked.requests.len(), 1, "it asks again from the beginning");
    assert_eq!(
        asked.timers.len(),
        1,
        "and asks for a tick, which a plugin still holding a stale one would not"
    );
    assert_eq!(pirate.usable_reading(), None);
    assert_eq!(pirate.mark(), PIRATE);
}

#[test]
fn an_answer_that_never_comes_does_not_stop_the_plugin_for_good() {
    // Nothing in the ABI promises an answer, and a plugin that waited on one
    // forever would be a chip drawing an hour-old number that looks exactly
    // as current as a fresh one — with every click returning early because a
    // cycle nobody will ever finish is still "in flight".
    let (mut pirate, _credentials) = built();

    // Nobody answers. Ticking short of the watchdog changes nothing.
    stub::advance(GIVE_UP_WAITING_AFTER - 1);
    pirate.tick();
    assert!(
        stub::taken().requests.is_empty(),
        "it gave up on a cycle that was still within its time"
    );

    stub::advance(2);
    pirate.tick();
    assert_eq!(pirate.problem(), Some(&Problem::Unreachable));
    let _ = stub::taken();

    // Giving up ends the cycle rather than starting one: the poll it books is
    // an ordinary one, a minute out, because something that has already been
    // silent for three minutes is not worth hurrying back to.
    stub::advance(POLL_MILLIS);
    pirate.tick();

    let asked = stub::taken();
    assert!(
        matches!(
            asked.requests.first().map(|(_, request)| request),
            Some(Request::ReadFile { .. })
        ),
        "the plugin never asked for anything again: {:?}",
        asked.requests
    );
}

#[test]
fn a_cycle_that_is_answered_leaves_the_watchdog_with_nothing_to_do() {
    // The other half: a slow answer that does arrive must not be mistaken for
    // one that never will, and a settled cycle must not leave the watchdog
    // armed against the *next* one.
    let mut pirate = reading();

    stub::advance(GIVE_UP_WAITING_AFTER * 2);
    pirate.tick();

    assert_eq!(pirate.problem(), None, "a finished cycle was given up on");
}

#[test]
fn being_told_to_ask_less_often_is_not_a_network_failure() {
    // The request was right and the session was good; the answer was "later".
    // Reporting that as "couldn't reach Claude" tells somebody their network
    // is broken when it is not.
    let mut pirate = reading();
    stub::advance(POLL_MILLIS);
    pirate.tick();
    let fetch = stub::taken().requests[0].0;

    pirate.deliver(
        fetch,
        Answer::Fetched {
            status: 429,
            body: Vec::new(),
        },
    );

    assert_eq!(pirate.problem(), Some(&Problem::RateLimited));
    assert_eq!(
        pirate.problem().expect("a problem").chip_label(),
        "asked too often"
    );
    // And the number it already had is still the best one there is.
    assert!(pirate.usable_reading().is_some());
}

#[test]
fn a_rate_limit_is_left_alone_even_when_somebody_presses_refresh() {
    // Asking again because a button was pressed is how a rate limit becomes a
    // longer one — and the endpoint is shared with Claude Code itself.
    let mut pirate = reading();
    stub::advance(POLL_MILLIS);
    pirate.tick();
    let fetch = stub::taken().requests[0].0;
    pirate.deliver(
        fetch,
        Answer::Fetched {
            status: 429,
            body: Vec::new(),
        },
    );
    let _ = stub::taken();

    pirate.run("refresh");
    assert!(
        stub::taken().requests.is_empty(),
        "a click broke the back-off"
    );

    // The ordinary poll does not break it either, until the wait is over.
    stub::advance(POLL_MILLIS);
    pirate.tick();
    assert!(
        stub::taken().requests.is_empty(),
        "the poll broke the back-off"
    );

    stub::advance(BACK_OFF_FOR);
    pirate.tick();
    assert!(
        !stub::taken().requests.is_empty(),
        "it never asked again after backing off"
    );
}

#[test]
fn a_status_that_is_none_of_the_known_ones_is_said_with_its_number() {
    // A person who can see 503 knows more than one who is told the network
    // failed.
    let mut pirate = reading();
    stub::advance(POLL_MILLIS);
    pirate.tick();
    let fetch = stub::taken().requests[0].0;

    pirate.deliver(
        fetch,
        Answer::Fetched {
            status: 503,
            body: Vec::new(),
        },
    );

    assert_eq!(pirate.problem(), Some(&Problem::Returned(503)));
    assert!(
        pirate
            .problem()
            .expect("a problem")
            .message()
            .contains("503"),
        "{}",
        pirate.problem().expect("a problem").message()
    );
}
