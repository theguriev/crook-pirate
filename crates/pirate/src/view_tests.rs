//! What is on screen, asserted as the tree the host is handed.
//!
//! A describing plugin has one enormous advantage over a painting one: what it
//! puts on screen is a value. There is no window here, no theme and no
//! renderer — the assertions below are the actual thing Crook receives.

use super::*;

use crook_plugin_api::Answer;

use crate::sys::stub;

/// A credentials file with a session that does not expire.
const CREDENTIALS: &[u8] = br#"{"claudeAiOauth":{"accessToken":"x"}}"#;

/// A plugin carried to whatever `answer` says the endpoint replied.
fn answered(status: u16, body: &[u8]) -> Pirate {
    stub::forget();
    let mut pirate = Pirate::new();
    pirate.build();
    let credentials = stub::taken().requests[0].0;
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
            status,
            body: body.to_vec(),
        },
    );
    let _ = stub::taken();
    pirate
}

/// Everything of one kind in a tree, in the order it is drawn.
fn texts(node: &Node) -> Vec<String> {
    let mut found = Vec::new();
    walk(node, &mut |node| {
        if let Node::Text { text, .. } | Node::Note { text, .. } = node {
            found.push(text.clone());
        }
    });
    found
}

fn meters(node: &Node) -> Vec<(f32, Tone)> {
    let mut found = Vec::new();
    walk(node, &mut |node| {
        if let Node::Meter { fraction, tone } = node {
            found.push((*fraction, *tone));
        }
    });
    found
}

fn walk(node: &Node, seen: &mut impl FnMut(&Node)) {
    seen(node);
    match node {
        Node::Row(children) | Node::Column(children) => {
            for child in children {
                walk(child, seen);
            }
        }
        Node::Pressable { content, .. } => walk(content, seen),
        Node::Anchored { content, panel, .. } => {
            walk(content, seen);
            if let Some(panel) = panel {
                walk(panel, seen);
            }
        }
        _ => {}
    }
}

/// The mark and the label out of a chip, which is what a header shows.
fn pill(node: &Node) -> (String, Tone, String, Tone) {
    let Node::Anchored { content, .. } = node else {
        panic!("a chip is anchored, so that it has somewhere to hang a panel");
    };
    let Node::Pressable { content, action } = content.as_ref() else {
        panic!("a chip is pressable: clicking it is how the panel opens");
    };
    assert_eq!(action, "panel");
    let Node::Row(parts) = content.as_ref() else {
        panic!("a chip is a mark beside a number");
    };
    match (&parts[0], &parts[2]) {
        (
            Node::Icon { name, tone: mark },
            Node::Text {
                text, tone: label, ..
            },
        ) => (name.clone(), *mark, text.clone(), *label),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_chip_with_nothing_to_report_yet_says_so_without_looking_broken() {
    stub::forget();
    let pirate = Pirate::new();

    let (mark, mark_tone, label, label_tone) = pill(&chip(&pirate));

    assert_eq!(mark, "pirate");
    assert_eq!(mark_tone, Tone::Primary);
    // An en dash rather than nothing at all.
    assert_eq!(label, "\u{2013}");
    assert_eq!(label_tone, Tone::Success);
}

#[test]
fn a_reading_is_printed_in_the_band_it_falls_in() {
    for (utilization, expected, tone) in [
        (0.0, "0%", Tone::Success),
        (47.4, "47%", Tone::Success),
        (79.9, "80%", Tone::Success),
        (80.0, "80%", Tone::Warning),
        (94.9, "95%", Tone::Warning),
        (95.0, "95%", Tone::Danger),
        (140.0, "100%", Tone::Danger),
    ] {
        let pirate = answered(
            200,
            format!(r#"{{"five_hour":{{"utilization":{utilization}}}}}"#).as_bytes(),
        );

        let (_, _, label, label_tone) = pill(&chip(&pirate));

        assert_eq!(label, expected, "at {utilization}");
        assert_eq!(label_tone, tone, "at {utilization}");
    }
}

#[test]
fn a_session_that_has_gone_replaces_the_number_and_greys_the_mark() {
    let pirate = answered(401, b"");

    let (mark, mark_tone, label, label_tone) = pill(&chip(&pirate));

    assert_eq!(label, "session expired");
    assert_eq!(label_tone, Tone::Muted);
    // The mark goes grey with it: a pirate that stayed bright beside a
    // greyed-out number would be the loudest thing in the row saying the
    // reading is current.
    assert_eq!(mark_tone, Tone::Muted);
    assert_eq!(mark, "pirate");
}

#[test]
fn the_panel_is_not_there_until_somebody_opens_it() {
    let mut pirate = answered(200, br#"{"five_hour":{"utilization":47.0}}"#);

    let Node::Anchored { panel, dismiss, .. } = chip(&pirate) else {
        panic!("a chip is anchored");
    };
    assert_eq!(panel, None);
    assert_eq!(dismiss, "dismiss");

    pirate.run("panel");

    let Node::Anchored { panel, .. } = chip(&pirate) else {
        panic!("a chip is anchored");
    };
    assert!(panel.is_some());
}

#[test]
fn the_panel_draws_a_bar_for_every_limit_claude_reports() {
    let mut pirate = answered(
        200,
        br#"{"five_hour":{"utilization":47.0,"resets_at":"2026-09-04T21:30:00Z"},
             "seven_day":{"utilization":96.0}}"#,
    );
    pirate.run("panel");

    let chip = chip(&pirate);

    assert_eq!(
        meters(&chip),
        vec![(0.47, Tone::Success), (0.96, Tone::Danger)]
    );
    let texts = texts(&chip);
    assert!(texts.iter().any(|text| text == "Session"), "{texts:?}");
    assert!(texts.iter().any(|text| text == "Week"), "{texts:?}");
    // The clock the stub keeps is 2026-09-04T18:00Z, so the session window
    // has three and a half hours left of it.
    assert!(
        texts.iter().any(|text| text == "resets in 3h 30m"),
        "{texts:?}"
    );
}

#[test]
fn a_panel_with_no_reading_says_why_rather_than_drawing_empty_bars() {
    let mut pirate = answered(401, b"");
    pirate.run("panel");

    let chip = chip(&pirate);

    assert!(
        meters(&chip).is_empty(),
        "two empty bars read as nothing used"
    );
    assert!(
        texts(&chip)
            .iter()
            .any(|text| text.contains("run Claude Code to refresh it")),
        "{:?}",
        texts(&chip)
    );
}

#[test]
fn a_panel_says_where_the_number_came_from_and_where_it_does_not_go() {
    let mut pirate = answered(200, br#"{"five_hour":{"utilization":10.0}}"#);
    pirate.run("panel");

    let texts = texts(&chip(&pirate));

    assert!(
        texts
            .iter()
            .any(|text| text.contains("read where they are and sent nowhere")),
        "{texts:?}"
    );
}

#[test]
fn extra_usage_is_drawn_in_money_because_that_is_what_it_is() {
    let mut pirate = answered(
        200,
        br#"{"five_hour":{"utilization":10.0},
             "extra_usage":{"is_enabled":true,"monthly_limit":5000.0,"used_credits":1234.0}}"#,
    );
    pirate.run("panel");

    let texts = texts(&chip(&pirate));

    assert!(texts.iter().any(|text| text == "$12 of $50"), "{texts:?}");
}
