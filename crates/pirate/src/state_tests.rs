//! What the plugin does, without a terminal to do it in.
//!
//! Every import is stubbed (see `sys`), so a whole cycle — a click, ask for
//! the file, be handed it, ask for the reading, be handed that — runs here in
//! microseconds and is asserted rather than watched.
//!
//! The rule most of these are about: **only a click asks.** A build asks for
//! nothing, a tick asks for nothing, and the one thing that can refuse a click
//! is a rate limit that has not finished.

use super::*;

use crate::sys::stub;

/// A credentials file with a session in it that does not expire.
const CREDENTIALS: &[u8] = br#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-x"}}"#;

/// What the endpoint answers for an account about half way through its window.
const USAGE: &[u8] = br#"{"five_hour":{"utilization":47.4,"resets_at":"2026-09-04T18:30:00Z"},
                          "seven_day":{"utilization":62.0}}"#;

/// A plugin that has built and been clicked once, with the ticket the
/// credentials read will answer with.
///
/// The transcripts it also asked for are left unanswered. No test below is
/// about the chart, and a scan still walking is what one looks like.
fn opened() -> (Pirate, i32) {
    stub::forget();
    let mut pirate = Pirate::new();
    pirate.build();
    pirate.run("panel");
    let asked = stub::taken();
    (pirate, asked.requests[0].0)
}

/// The same, carried all the way to a reading and then put away.
///
/// Which is the plugin's ordinary state: a number on the chip, a shut panel,
/// nothing in flight, and — after the tick the last cycle booked has been
/// delivered — no wake-up booked either. A plugin at rest.
fn reading() -> Pirate {
    let (mut pirate, credentials) = opened();
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
    pirate.run("panel");
    // And the bite the click bought is let run out, so that what comes back
    // has nothing outstanding at all: no request, no answer, no burst, and no
    // wake-up booked to carry any of them on.
    stub::advance(CHOMP_FOR);
    for _ in 0..CHOMP_CYCLE.len() {
        pirate.tick();
    }
    let _ = stub::taken();
    pirate
}

#[test]
fn building_registers_the_chip_and_asks_for_nothing() {
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
            (String::from("panel"), String::from("Show the usage panel")),
            // Reachable and not offered: a zero-length title.
            (String::from("dismiss"), String::new()),
        ]
    );
    assert!(
        asked.requests.is_empty(),
        "a plugin nobody has clicked must not spend a request: {:?}",
        asked.requests
    );
    assert!(
        asked.timers.is_empty(),
        "and must not wake up to do it later either: {:?}",
        asked.timers
    );
}

#[test]
fn the_session_it_reads_is_what_it_asks_claude_with() {
    let (mut pirate, credentials) = opened();

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
fn opening_the_panel_asks_and_putting_it_away_does_not() {
    stub::forget();
    let mut pirate = Pirate::new();
    pirate.build();

    pirate.run("panel");
    let opening = stub::taken().requests.len();
    pirate.run("panel");
    let closing = stub::taken().requests.len();

    // Two: the reading, and the first page of the transcripts behind it.
    assert_eq!(
        opening, 2,
        "opening it asks for a fresh reading and the week"
    );
    assert_eq!(
        closing, 0,
        "and putting it away asks for nothing: the figures must not move under a person dismissing them"
    );
    assert!(!pirate.panel_open());
}

#[test]
fn a_shut_panel_asks_for_nothing_however_long_it_is_left() {
    // The whole point of the change: a chip sitting in a header is not a
    // reason to spend somebody's rate limit, and the endpoint is shared with
    // Claude Code itself.
    let mut pirate = reading();

    for _ in 0..100 {
        stub::advance(GIVE_UP_WAITING_AFTER);
        pirate.tick();
    }

    let asked = stub::taken();
    assert!(
        asked.requests.is_empty(),
        "something polled in the background: {:?}",
        asked.requests
    );
    assert!(
        asked.timers.is_empty(),
        "and booked a wake-up to do it again: {:?}",
        asked.timers
    );
}

#[test]
fn an_open_panel_is_redrawn_so_the_countdowns_stay_honest() {
    // A redraw and nothing else. "resets in 40m" is worked out from the clock
    // at the moment it is drawn, so a panel nothing wakes is a panel whose
    // countdown stopped when it opened.
    let mut pirate = reading();

    pirate.run("panel");
    let fetch = stub::taken().requests[0].0;
    pirate.deliver(
        fetch,
        Answer::Fetched {
            status: 200,
            body: USAGE.to_vec(),
        },
    );
    let _ = stub::taken();
    // Past the bite the click bought, which books its own faster tick.
    stub::advance(CHOMP_FOR);
    pirate.tick();

    let asked = stub::taken();
    assert_eq!(asked.timers, vec![COUNTDOWN_MILLIS as i32]);
    assert!(
        asked.requests.is_empty(),
        "a redraw is not a reason to ask Anthropic anything: {:?}",
        asked.requests
    );
}

#[test]
fn a_second_reading_reuses_the_token_rather_than_the_file() {
    // Opening the panel again is a request; opening the credentials file again
    // would be a second one, and one that shows up in somebody's audit log for
    // nothing.
    let mut pirate = reading();

    pirate.run("panel");

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
    pirate.run("panel");
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
    pirate.run("dismiss");
    let _ = stub::taken();

    pirate.run("panel");

    assert!(
        matches!(
            stub::taken().requests.first().map(|(_, request)| request),
            Some(Request::ReadFile { .. })
        ),
        "a token inside its last minute has to be read again, not sent"
    );
}

#[test]
fn a_machine_that_has_never_run_claude_code_says_so_and_then_rests() {
    let (mut pirate, credentials) = opened();

    pirate.deliver(
        credentials,
        Answer::Failed(String::from("No such file or directory")),
    );

    assert_eq!(pirate.problem(), Some(&Problem::NoSession));
    assert_eq!(
        pirate.problem().expect("a problem").chip_label(),
        "no session"
    );

    // And nothing is scheduled to find out otherwise. There is nothing to poll
    // until somebody runs Claude Code, and the click that opens the panel next
    // is what will notice they have.
    pirate.run("dismiss");
    stub::advance(CHOMP_FOR);
    for _ in 0..CHOMP_CYCLE.len() {
        pirate.tick();
    }
    let _ = stub::taken();
    pirate.tick();
    let asked = stub::taken();
    assert!(
        asked.requests.is_empty() && asked.timers.is_empty(),
        "{asked:?}"
    );
}

#[test]
fn a_plugin_nobody_has_allowed_says_what_to_allow() {
    let (mut pirate, credentials) = opened();

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

    pirate.run("panel");
    let fetch = stub::taken().requests[0].0;
    pirate.deliver(fetch, Answer::Failed(String::from("connection reset")));

    assert!(
        pirate.usable_reading().is_some(),
        "a network blip is not a reason to throw away the number the last opening got"
    );

    pirate.run("panel");
    pirate.run("panel");
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
fn the_mouth_moves_only_while_somebody_is_waiting() {
    let mut pirate = reading();
    assert_eq!(pirate.mark(), PIRATE);

    // A tick with nothing in flight is a redraw, not a bite.
    pirate.tick();
    assert_eq!(pirate.mark(), PIRATE, "nobody is waiting on anything");

    pirate.run("panel");
    assert_eq!(pirate.mark(), CHOMP_CYCLE[0]);
    pirate.tick();
    assert_eq!(pirate.mark(), CHOMP_CYCLE[1]);
    pirate.tick();
    assert_eq!(pirate.mark(), CHOMP_CYCLE[2]);
}

#[test]
fn a_bite_outlasts_an_answer_that_comes_straight_back() {
    // The click is the whole of what the mouth acknowledges, and the request
    // behind it is usually over before a frame has been drawn. A mouth that
    // shut with the answer would be a click that drew nothing.
    let mut pirate = reading();
    pirate.run("panel");
    let fetch = stub::taken().requests[0].0;

    pirate.deliver(
        fetch,
        Answer::Fetched {
            status: 200,
            body: USAGE.to_vec(),
        },
    );

    let frames: Vec<&str> = (0..CHOMP_CYCLE.len())
        .map(|_| {
            stub::advance(CHOMP_MILLIS);
            pirate.tick();
            pirate.mark()
        })
        .collect();
    assert_eq!(
        frames,
        vec!["pirate-open", "pirate-wide", "pirate-open", "pirate"],
        "the mouth stopped the moment nobody was waiting on anything"
    );

    // And it does end. A bite that outlived its click would be an animation on
    // a timer nobody is watching.
    stub::advance(CHOMP_FOR);
    let _ = stub::taken();
    pirate.tick();
    assert_eq!(pirate.mark(), PIRATE);
    assert_eq!(
        stub::taken().timers,
        vec![COUNTDOWN_MILLIS as i32],
        "the bite is over, so the only thing left to wake up for is the panel"
    );
}

#[test]
fn a_burst_that_runs_out_mid_bite_still_ends_on_a_shut_mouth() {
    // The frame the burst happens to run out on is arbitrary — it is a wall
    // clock against a request — and a pirate left frozen with his mouth wide
    // open reads as a plugin that has crashed.
    let mut pirate = reading();
    pirate.run("panel");
    let fetch = stub::taken().requests[0].0;
    pirate.deliver(
        fetch,
        Answer::Fetched {
            status: 200,
            body: USAGE.to_vec(),
        },
    );

    pirate.tick();
    pirate.tick();
    assert_eq!(pirate.mark(), "pirate-wide", "two frames in, as set up");
    stub::advance(CHOMP_FOR);

    pirate.tick();
    assert_eq!(pirate.mark(), "pirate-open", "it stopped on the wide face");
    pirate.tick();
    assert_eq!(pirate.mark(), PIRATE);
}

#[test]
fn a_click_the_back_off_refuses_does_not_animate_either() {
    // The mouth means somebody is waiting on an answer. Nothing went out, so
    // nobody is, and a bite here would be the chip saying it is doing
    // something it is not.
    let mut pirate = reading();
    pirate.run("panel");
    let fetch = stub::taken().requests[0].0;
    pirate.deliver(
        fetch,
        Answer::Fetched {
            status: 429,
            body: Vec::new(),
        },
    );
    stub::advance(CHOMP_FOR);
    for _ in 0..CHOMP_CYCLE.len() {
        pirate.tick();
    }
    pirate.run("dismiss");
    let _ = stub::taken();

    pirate.run("panel");

    assert_eq!(pirate.mark(), PIRATE);
    assert!(
        stub::taken().timers.is_empty(),
        "a click that asked for nothing booked an animation anyway"
    );
}

#[test]
fn the_bite_comes_back_round_to_a_whole_face() {
    let mut pirate = reading();
    pirate.run("panel");

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
    // The bug that shipped first: a plugin already waiting for its next
    // wake-up never asked again, so the mark stood still with a person
    // watching it. And the bug on the other side of it: asking again for a
    // moment already booked is a heartbeat that doubles, then quadruples.
    let mut pirate = reading();

    pirate.run("panel");
    assert_eq!(
        stub::taken().timers,
        vec![CHOMP_MILLIS as i32],
        "a click wants the mark redrawn a frame from now"
    );

    pirate.run("dismiss");
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
    // Coming back the other way: the answer lands, the mouth shuts, and what
    // is left to wake up for is the countdowns — which are further off than
    // the frame already booked, so nothing is asked for.
    let mut pirate = reading();
    pirate.run("panel");
    let fetch = stub::taken().requests[0].0;

    pirate.deliver(
        fetch,
        Answer::Fetched {
            status: 200,
            body: USAGE.to_vec(),
        },
    );

    assert!(
        stub::taken().timers.is_empty(),
        "a wait that is further off than the one already booked is not worth asking for"
    );
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
fn building_again_forgets_everything_and_asks_for_nothing() {
    // The host builds a plugin again when it is switched back on, when a
    // person answers what it asked to be allowed, and every time it starts.
    // Whatever the previous life was waiting on — a timer nobody will fire
    // again, a ticket nobody will answer — has to go. What must *not* replace
    // it is a request: a build that asked would be a plugin spending one of a
    // small budget on every restart, which is how it came to say "asked too
    // often" for a living.
    let mut pirate = reading();
    pirate.run("panel");
    let _ = stub::taken();

    pirate = Pirate::new();
    pirate.build();

    let asked = stub::taken();
    assert!(
        asked.requests.is_empty(),
        "a rebuild asked Anthropic something nobody clicked for: {:?}",
        asked.requests
    );
    assert!(
        asked.timers.is_empty(),
        "and booked a tick with nothing to do on it: {:?}",
        asked.timers
    );
    assert_eq!(pirate.usable_reading(), None);
    assert_eq!(pirate.mark(), PIRATE);
}

#[test]
fn an_answer_that_never_comes_does_not_stop_the_plugin_for_good() {
    // Nothing in the ABI promises an answer, and a plugin that waited on one
    // forever would be a chip drawing an hour-old number that looks exactly as
    // current as a fresh one — with every click returning early because a
    // cycle nobody will ever finish is still "in flight".
    let (mut pirate, _credentials) = opened();

    // Nobody answers. Ticking short of the watchdog changes nothing.
    stub::advance(GIVE_UP_WAITING_AFTER - 1);
    pirate.tick();
    assert_eq!(
        pirate.problem(),
        None,
        "it gave up on a cycle that was still within its time"
    );

    stub::advance(2);
    pirate.tick();
    assert_eq!(pirate.problem(), Some(&Problem::Unreachable));
    let _ = stub::taken();

    // Giving up ends the cycle rather than starting one, and the next opening
    // of the panel is free to ask again.
    pirate.run("panel");
    pirate.run("panel");

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
    pirate.run("panel");
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
fn a_rate_limit_is_left_alone_however_often_the_panel_is_opened() {
    // The one thing a click may not argue with. Asking again because somebody
    // opened the panel is how a rate limit becomes a longer one — and the
    // endpoint is shared with Claude Code itself.
    let mut pirate = reading();
    pirate.run("panel");
    let fetch = stub::taken().requests[0].0;
    pirate.deliver(
        fetch,
        Answer::Fetched {
            status: 429,
            body: Vec::new(),
        },
    );
    let _ = stub::taken();

    pirate.run("dismiss");
    for _ in 0..5 {
        pirate.run("panel");
        pirate.run("dismiss");
    }
    assert!(
        stub::taken().requests.is_empty(),
        "opening the panel broke the back-off"
    );

    stub::advance(BACK_OFF_FOR);
    pirate.run("panel");
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
    pirate.run("panel");
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
