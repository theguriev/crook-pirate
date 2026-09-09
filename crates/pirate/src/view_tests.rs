//! What is on screen, asserted as the tree the host is handed.
//!
//! A describing plugin has one enormous advantage over a painting one: what it
//! puts on screen is a value. There is no window here, no theme and no
//! renderer — the assertions below are the actual thing Crook receives.

use super::*;

use crook_plugin_api::{Answer, Cell, Request, Tallied};

use crate::sys::stub;

/// A credentials file with a session that does not expire.
const CREDENTIALS: &[u8] = br#"{"claudeAiOauth":{"accessToken":"x"}}"#;

/// A plugin carried to whatever `answer` says the endpoint replied.
///
/// By way of a click, because a click is the only thing that asks: a plugin
/// that has merely built has asked nobody anything.
fn answered(status: u16, body: &[u8]) -> Pirate {
    stub::forget();
    let mut pirate = Pirate::new();
    pirate.build();
    pirate.run("panel");
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
    // Put away again, so that every test below opens the panel itself and the
    // tree it asserts is the one that opening put there.
    pirate.run("dismiss");
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
        Node::Explained {
            content,
            explanation,
        } => {
            walk(content, seen);
            walk(explanation, seen);
        }
        _ => {}
    }
}

/// Every run of text with the size and tone it is set in, in the order it is
/// drawn. A [`Node::Note`] has no size of its own; it is reported at the
/// small one, which is what the host draws it at.
fn styled(node: &Node) -> Vec<(String, Size, Tone)> {
    let mut found = Vec::new();
    walk(node, &mut |node| match node {
        Node::Text { text, size, tone } => found.push((text.clone(), *size, *tone)),
        Node::Note { text, tone } => found.push((text.clone(), Size::Small, *tone)),
        _ => {}
    });
    found
}

/// The rows of the panel, top to bottom.
fn panel_rows(node: &Node) -> Vec<Node> {
    let Node::Anchored {
        panel: Some(panel), ..
    } = node
    else {
        panic!("the panel is open");
    };
    let Node::Column(rows) = panel.as_ref() else {
        panic!("a panel is a column");
    };
    rows.clone()
}

/// A plugin whose panel is open on a reading and on a week the host counted.
///
/// The week is a small one in the shape `examples/fixture.rs` draws: a heavy
/// model and a light one, a busy day six days back and a quieter today, and
/// two projects — one of which has a branch too long to print.
fn with_a_week() -> Pirate {
    with_a_week_counted(tables())
}

/// A plugin whose panel is open on a week the host counted and found nothing
/// in: a fresh machine, or a week off.
fn with_an_empty_week() -> Pirate {
    with_a_week_counted(vec![Vec::new(); 4])
}

/// The four tables `with_a_week` hands over.
fn tables() -> Vec<Vec<Tallied>> {
    let tallied = |key: &[&str], sums: [f64; 4], lines: u64| Tallied {
        key: key
            .iter()
            .map(|part| Cell::Text(String::from(*part)))
            .collect(),
        sums: sums.to_vec(),
        lines,
    };

    vec![
        vec![
            tallied(&["assistant", "claude-opus-5"], [0., 0., 0., 960.], 96),
            tallied(
                &["assistant", "claude-haiku-4-5-20251001"],
                [0., 0., 0., 40.],
                4,
            ),
        ],
        vec![
            tallied(&["assistant", "2026-08-30T12"], [0., 0., 0., 1000.], 1),
            tallied(&["assistant", "2026-09-04T12"], [0., 0., 0., 250.], 1),
        ],
        vec![
            tallied(
                &["assistant", "/home/me/crook", "main"],
                [0., 0., 0., 900.],
                1,
            ),
            tallied(
                &[
                    "assistant",
                    "/home/me/terminal-features",
                    "worktree-tidy-and-the-rest-of-it",
                ],
                [0., 0., 0., 100.],
                1,
            ),
        ],
        vec![tallied(&["assistant", "s1"], [0., 0., 0., 0.], 1)],
    ]
}

/// A plugin whose panel is open on a reading, with the transcripts counted
/// into whatever `tables` says.
fn with_a_week_counted(tables: Vec<Vec<Tallied>>) -> Pirate {
    let mut tables = Some(tables);
    stub::forget();
    let mut pirate = Pirate::new();
    pirate.build();
    pirate.run("panel");

    for (ticket, request) in stub::taken().requests {
        match request {
            Request::ReadFile { .. } => pirate.deliver(
                ticket,
                Answer::Read {
                    bytes: CREDENTIALS.to_vec(),
                },
            ),
            Request::Tally { .. } => pirate.deliver(
                ticket,
                Answer::Counted {
                    tables: tables.take().expect("counted once"),
                    lines: 100,
                },
            ),
            _ => {}
        }
    }
    for (ticket, request) in stub::taken().requests {
        if let Request::Fetch { .. } = request {
            pirate.deliver(
                ticket,
                Answer::Fetched {
                    status: 200,
                    body: br#"{"five_hour":{"utilization":47.0,"resets_at":"2026-09-04T21:30:00Z"},
                              "seven_day":{"utilization":37.0}}"#
                        .to_vec(),
                },
            );
        }
    }
    pirate
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

    let styled = styled(&chip(&pirate));

    // At the size of a name rather than of a reading: a dollar figure has no
    // band, and only the percentages are set large.
    assert!(
        styled.contains(&(String::from("$12 of $50"), Size::Body, Tone::Primary)),
        "{styled:?}"
    );
    assert_eq!(
        styled
            .iter()
            .filter(|(_, size, _)| *size == Size::Large)
            .count(),
        1,
        "{styled:?}"
    );
}

#[test]
fn the_two_readings_are_the_only_large_type_and_the_countdown_sits_beside_the_name() {
    let pirate = with_a_week();

    let styled = styled(&chip(&pirate));

    let large: Vec<&(String, Size, Tone)> = styled
        .iter()
        .filter(|(_, size, _)| *size == Size::Large)
        .collect();
    assert_eq!(
        large,
        vec![
            &(String::from("47%"), Size::Large, Tone::Success),
            &(String::from("37%"), Size::Large, Tone::Success)
        ]
    );

    // Name, then countdown, then the reading: one row, in that order.
    let rows = panel_rows(&chip(&pirate));
    let Node::Row(session) = &rows[0] else {
        panic!("the first row is the session's, {:?}", rows[0]);
    };
    let words: Vec<String> = session
        .iter()
        .filter_map(|node| match node {
            Node::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(words, vec!["Session", "resets in 3h 30m", "47%"]);
    assert!(
        matches!(rows[2], Node::Meter { .. }),
        "the meter sits under its own label, {:?}",
        rows[2]
    );
}

#[test]
fn a_rule_is_never_drawn_straight_onto_what_is_above_it() {
    let pirate = with_a_week();

    let rows = panel_rows(&chip(&pirate));

    let rules: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| matches!(row, Node::Rule))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(rules.len(), 2, "one seam and one colophon, {rows:?}");
    for index in rules {
        assert!(
            matches!(rows[index - 1], Node::Gap(Gap::Medium)),
            "room above the rule at {index}, {:?}",
            rows[index - 1]
        );
    }
}

#[test]
fn the_week_is_never_set_at_the_size_of_a_reading() {
    let pirate = with_a_week();

    let styled = styled(&chip(&pirate));

    let seam = styled
        .iter()
        .position(|(text, ..)| text == "Last 7 days")
        .expect("the local week has a heading");
    assert!(
        styled[seam..]
            .iter()
            .all(|(_, size, _)| *size != Size::Large),
        "{:?}",
        &styled[seam..]
    );
    // The total is a name-sized figure with its unit beside it, so the one
    // number that is not a percentage is the one that says what it is.
    assert!(
        styled.contains(&(String::from("1.0k"), Size::Body, Tone::Primary)),
        "{styled:?}"
    );
    assert!(
        styled.contains(&(String::from("tokens"), Size::Small, Tone::Muted)),
        "{styled:?}"
    );
}

#[test]
fn the_chart_has_a_weekday_under_every_column_and_today_is_the_one_in_the_ordinary_weight() {
    let pirate = with_a_week();

    let rows = panel_rows(&chip(&pirate));

    let bars = rows
        .iter()
        .position(|row| matches!(row, Node::Bars { .. }))
        .expect("a chart");
    let Node::Bars { values, .. } = &rows[bars] else {
        unreachable!();
    };
    assert!(
        matches!(rows[bars + 1], Node::Gap(Gap::Small)),
        "held to the columns, {:?}",
        rows[bars + 1]
    );
    let Node::Row(axis) = &rows[bars + 2] else {
        panic!("the weekdays sit under the bars, {:?}", rows[bars + 2]);
    };
    let letters = weekday_letters(&chip(&pirate));
    assert_eq!(letters.len(), values.len(), "one letter per column");
    // The stub's clock is a Friday; the six days behind it start on a Saturday.
    assert_eq!(
        letters,
        vec![
            (String::from("S"), Tone::Muted),
            (String::from("S"), Tone::Muted),
            (String::from("M"), Tone::Muted),
            (String::from("T"), Tone::Muted),
            (String::from("W"), Tone::Muted),
            (String::from("T"), Tone::Muted),
            (String::from("F"), Tone::Primary),
        ]
    );
    // A share of the width before the first letter, between each pair, and
    // after the last: what lines the letters up with columns spread evenly.
    assert!(matches!(axis.first(), Some(Node::Fill)));
    assert!(matches!(axis.last(), Some(Node::Fill)));
    assert_eq!(
        axis.iter()
            .filter(|node| matches!(node, Node::Fill))
            .count(),
        letters.len() + 1
    );

    // The chart's caption sits under the axis: what the columns add up to, in
    // things counted rather than measured.
    assert!(
        matches!(rows[bars + 3], Node::Gap(Gap::Small)),
        "{:?}",
        rows[bars + 3]
    );
    assert_eq!(
        rows[bars + 4],
        Node::Note {
            text: String::from("100 turns \u{00b7} 1 sessions \u{00b7} 2 models"),
            tone: Tone::Muted,
        }
    );
}

/// The letters under the chart and their tones, in order.
fn weekday_letters(node: &Node) -> Vec<(String, Tone)> {
    let rows = panel_rows(node);
    let bars = rows
        .iter()
        .position(|row| matches!(row, Node::Bars { .. }))
        .expect("a chart");
    let Node::Row(axis) = &rows[bars + 2] else {
        panic!("the weekdays sit under the bars, {:?}", rows[bars + 2]);
    };
    axis.iter()
        .filter_map(|node| match node {
            Node::Text { text, tone, .. } => Some((text.clone(), *tone)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_axis_stays_with_the_columns_when_the_clock_crosses_midnight() {
    let pirate = with_a_week();
    let before = weekday_letters(&chip(&pirate));
    assert_eq!(before.last(), Some(&(String::from("F"), Tone::Primary)));

    // Nine in the evening, three hours east; a quarter past midnight is three
    // and a quarter hours on. Nobody has clicked, so the columns are still
    // Friday's, and the letters under them have to be too — they belong to
    // the week that was counted, not to the clock.
    stub::advance(3 * 3_600_000 + 15 * 60_000);

    assert_eq!(weekday_letters(&chip(&pirate)), before);
}

#[test]
fn an_empty_week_is_a_sentence_rather_than_an_axis_with_nothing_on_it() {
    let pirate = with_an_empty_week();

    let rows = panel_rows(&chip(&pirate));

    assert!(
        !rows.iter().any(|row| matches!(row, Node::Bars { .. })),
        "{rows:?}"
    );
    let heading = rows
        .iter()
        .position(|row| matches!(row, Node::Text { text, .. } if text == "Last 7 days"))
        .expect("the local week has a heading");
    assert!(matches!(rows[heading + 1], Node::Gap(Gap::Small)));
    assert!(
        matches!(&rows[heading + 2], Node::Note { text, .. } if text.starts_with("Nothing in the last 7 days")),
        "{:?}",
        rows[heading + 2]
    );
    // And no "Models" or "Projects" over nothing.
    assert!(!texts(&chip(&pirate)).iter().any(|text| text == "Models"));
}

#[test]
fn a_week_still_being_counted_says_so_under_its_heading() {
    let mut pirate = answered(200, br#"{"five_hour":{"utilization":10.0}}"#);
    pirate.run("panel");

    let rows = panel_rows(&chip(&pirate));

    assert!(
        !rows.iter().any(|row| matches!(row, Node::Bars { .. })),
        "{rows:?}"
    );
    let heading = rows
        .iter()
        .position(|row| matches!(row, Node::Text { text, .. } if text == "Last 7 days"))
        .expect("the local week has a heading");
    assert!(matches!(rows[heading + 1], Node::Gap(Gap::Small)));
    assert_eq!(
        rows[heading + 2],
        Node::Note {
            text: String::from("Reading this machine's transcripts\u{2026}"),
            tone: Tone::Muted,
        }
    );
}

#[test]
fn a_reading_the_refresh_could_not_replace_stays_with_the_failure_named_under_both_limits() {
    let mut pirate = with_a_week();
    pirate.run("dismiss");
    // Ten minutes on, the panel is opened again and the refresh it asks for
    // comes back rate-limited.
    stub::advance(10 * 60_000);
    pirate.run("panel");
    for (ticket, request) in stub::taken().requests {
        if let Request::Fetch { .. } = request {
            pirate.deliver(
                ticket,
                Answer::Fetched {
                    status: 429,
                    body: Vec::new(),
                },
            );
        }
    }

    let chip = chip(&pirate);

    // Both meters are still there, and the sentence sits under the second of
    // them — about the refresh, which is one request for the pair — before
    // the seam.
    assert_eq!(meters(&chip).len(), 2);
    let rows = panel_rows(&chip);
    let last_meter = rows
        .iter()
        .rposition(|row| matches!(row, Node::Meter { .. }))
        .expect("two meters");
    assert!(
        matches!(rows[last_meter + 1], Node::Gap(Gap::Medium)),
        "{:?}",
        rows[last_meter + 1]
    );
    assert!(
        matches!(&rows[last_meter + 2], Node::Note { text, tone: Tone::Muted } if text.contains("rate-limiting")),
        "{:?}",
        rows[last_meter + 2]
    );
    assert!(matches!(rows[last_meter + 3], Node::Gap(Gap::Medium)));
    assert!(matches!(rows[last_meter + 4], Node::Rule));
}

#[test]
fn a_model_has_a_share_and_no_meter() {
    let pirate = with_a_week();

    let chip = chip(&pirate);

    // Two limits, and nothing else: the per-model bars that drew as a dot on
    // an empty track are gone.
    assert_eq!(meters(&chip).len(), 2, "{:?}", meters(&chip));
    let styled = styled(&chip);
    for expected in [
        (String::from("Opus 5"), Size::Small, Tone::Primary),
        (String::from("96%"), Size::Small, Tone::Muted),
        (String::from("Haiku 4.5"), Size::Small, Tone::Primary),
        (String::from("4%"), Size::Small, Tone::Muted),
    ] {
        assert!(styled.contains(&expected), "{expected:?} in {styled:?}");
    }

    // A name with its share, its detail line held to it, and air before the
    // next pair.
    let rows = panel_rows(&chip);
    let models = rows
        .iter()
        .position(|row| matches!(row, Node::Text { text, .. } if text == "Models"))
        .expect("a Models caption");
    assert!(matches!(rows[models + 1], Node::Gap(Gap::Small)));
    assert!(matches!(rows[models + 2], Node::Row(_)));
    assert_eq!(
        rows[models + 3],
        Node::Note {
            text: String::from("0 out \u{00b7} 960 cache \u{00b7} 96 turns"),
            tone: Tone::Muted,
        }
    );
    assert!(
        matches!(rows[models + 4], Node::Gap(Gap::Small)),
        "air between models, {:?}",
        rows[models + 4]
    );
    assert!(matches!(rows[models + 5], Node::Row(_)));
    assert_eq!(
        rows[models + 6],
        Node::Note {
            text: String::from("0 out \u{00b7} 40 cache \u{00b7} 4 turns"),
            tone: Tone::Muted,
        }
    );
}

#[test]
fn a_project_is_its_name_in_the_ordinary_weight_and_its_branch_quietly_cut_to_fit() {
    let pirate = with_a_week();

    let rows = panel_rows(&chip(&pirate));

    let caption = rows
        .iter()
        .position(|row| matches!(row, Node::Text { text, .. } if text == "Projects"))
        .expect("a Projects caption");
    assert!(matches!(rows[caption + 1], Node::Gap(Gap::Small)));
    // Heaviest first, and each row is name, branch, the leftover width, and
    // the figure, in that order — so which project reads before which branch.
    let row = |at: usize| -> Vec<(String, Size, Tone)> {
        let Node::Row(parts) = &rows[at] else {
            panic!("a project is a row, {:?}", rows[at]);
        };
        assert!(
            matches!(parts[3], Node::Fill),
            "the figure at the far end, {parts:?}"
        );
        styled(&rows[at])
    };
    assert_eq!(
        row(caption + 2),
        vec![
            (String::from("crook"), Size::Small, Tone::Primary),
            (String::from("\u{00b7} main"), Size::Small, Tone::Muted),
            (String::from("900"), Size::Small, Tone::Muted),
        ]
    );
    // Seventeen characters of name and three of separator leave ten for the
    // branch: nine of it, the hyphen it would have ended on dropped, and the
    // ellipsis.
    assert_eq!(
        row(caption + 3),
        vec![
            (
                String::from("terminal-features"),
                Size::Small,
                Tone::Primary
            ),
            (
                String::from("\u{00b7} worktree\u{2026}"),
                Size::Small,
                Tone::Muted
            ),
            (String::from("100"), Size::Small, Tone::Muted),
        ]
    );
    // And nothing between the rows: lines under one caption are a table. The
    // second project is the last, and what follows it is the seam.
    assert!(
        matches!(rows[caption + 4], Node::Gap(Gap::Medium)),
        "{:?}",
        rows[caption + 4]
    );
    assert!(matches!(rows[caption + 5], Node::Rule));
}

#[test]
fn a_label_is_cut_where_the_figure_beside_it_would_start_to_move() {
    let project = |name: &str, branch: Option<&str>| Project {
        name: String::from(name),
        branch: branch.map(String::from),
        tokens: 0,
    };

    assert_eq!(
        project_label(&project("crook", Some("main"))),
        (String::from("crook"), Some(String::from("main")))
    );
    assert_eq!(
        project_label(&project("crook", None)),
        (String::from("crook"), None)
    );
    // A name that fills the budget on its own has no room for a branch, and a
    // branch reduced to an ellipsis would be a qualifier that says nothing.
    assert_eq!(
        project_label(&project("a-directory-with-a-long-name-x", Some("main"))),
        (String::from("a-directory-with-a-long-name-x"), None)
    );
    // The boundary: twenty-five and three leave two, which is one letter and
    // the ellipsis; twenty-six leave one, which is nothing worth saying.
    assert_eq!(
        project_label(&project("twenty-five-characters-xx", Some("main"))),
        (
            String::from("twenty-five-characters-xx"),
            Some(String::from("m\u{2026}"))
        )
    );
    assert_eq!(
        project_label(&project("twenty-six-characters-xxxx", Some("main"))),
        (String::from("twenty-six-characters-xxxx"), None)
    );
    // And a name past the budget is cut itself.
    assert_eq!(
        project_label(&project(
            "a-directory-with-an-even-longer-name-than-that",
            None
        )),
        (String::from("a-directory-with-an-even-long\u{2026}"), None)
    );
}

#[test]
fn a_cut_does_not_leave_a_separator_dangling_before_the_ellipsis() {
    assert_eq!(elided("worktree-tidy", 10), "worktree\u{2026}");
    assert_eq!(elided("feature/thing", 9), "feature\u{2026}");
    assert_eq!(elided("a name that is long", 8), "a name\u{2026}");
    assert_eq!(elided("short", 10), "short");
}
