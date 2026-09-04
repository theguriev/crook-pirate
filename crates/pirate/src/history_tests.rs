//! What the totals add up to, without a transcript in sight.
//!
//! The host counts; what is tested here is the part that decides what the
//! counting *means* — which rows are turns at all, which hour belongs to which
//! day a person lived, and how the answer is ordered.

use super::*;

use crate::sys::stub;

/// The clock the stub keeps: 2026-09-04T18:00:00Z, three hours east.
const NOW: i64 = 1_788_544_800_000;

/// One row of a table, in the shape the host builds one.
fn tallied(key: &[&str], tokens: [f64; 4], lines: u64) -> Tallied {
    Tallied {
        key: key
            .iter()
            .map(|part| Cell::Text(String::from(*part)))
            .collect(),
        sums: tokens.to_vec(),
        lines,
    }
}

/// A week out of four tables, as the host would answer.
fn week_of(tables: Vec<Vec<Tallied>>) -> Week {
    stub::forget();
    stub::set_now(NOW);
    stub::set_timezone(180);
    let mut reading = Reading::start();
    let _ = stub::taken();
    reading.take(tables)
}

/// A week whose only table that matters is the one named.
fn only(table: usize, rows: Vec<Tallied>) -> Week {
    let mut tables = vec![Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    tables[table] = rows;
    week_of(tables)
}

#[test]
fn a_line_that_is_not_a_turn_is_not_counted() {
    // Both filters are the plugin's because the host has no notion of either:
    // a line carrying a usage object that is not an assistant turn is not a
    // turn, and `<synthetic>` is the placeholder for one produced without a
    // model — naming it in a breakdown of models would be a bug with a
    // straight face.
    let week = only(
        BY_MODEL,
        vec![
            tallied(&["assistant", "claude-opus-5"], [10., 20., 30., 40.], 3),
            tallied(&["user", "claude-opus-5"], [1., 1., 1., 1.], 9),
            tallied(&["assistant", "<synthetic>"], [1., 1., 1., 1.], 9),
        ],
    );

    assert_eq!(week.models.len(), 1);
    assert_eq!(week.models[0].id, "claude-opus-5");
    assert_eq!(week.turns, 3);
    assert_eq!(week.tokens(), 100);
}

#[test]
fn an_hour_belongs_to_the_day_the_person_lived() {
    // Three hours east: 22:00 UTC on the 3rd is one in the morning on the 4th,
    // and 12:00 UTC on the 3rd is the afternoon of the 3rd. UTC says one day;
    // the person who worked those hours lived two.
    let week = only(
        BY_HOUR,
        vec![
            tallied(&["assistant", "2026-09-03T22"], [0., 10., 0., 0.], 1),
            tallied(&["assistant", "2026-09-03T12"], [0., 20., 0., 0.], 1),
        ],
    );

    assert_eq!(week.days.len(), DAYS as usize);
    let busy: Vec<u64> = week.days.iter().copied().filter(|day| *day > 0).collect();
    assert_eq!(busy.len(), 2, "{:?}", week.days);
}

#[test]
fn every_day_of_the_window_is_a_column_even_the_quiet_ones() {
    // A gap in a chart has to be a column of no height rather than a missing
    // one, or the chart says the week was shorter than it was.
    let week = only(
        BY_HOUR,
        vec![tallied(
            &["assistant", "2026-09-04T12"],
            [0., 10., 0., 0.],
            1,
        )],
    );

    assert_eq!(week.days.len(), DAYS as usize);
    assert_eq!(week.days.iter().filter(|tokens| **tokens > 0).count(), 1);
}

#[test]
fn an_hour_older_than_the_first_column_counts_without_a_column_to_stand_in() {
    // The window is seven times twenty-four hours and the chart is seven days,
    // which are not the same thing: the hours between them are real turns with
    // nowhere to be drawn.
    let week = only(
        BY_HOUR,
        vec![tallied(
            &["assistant", "2026-08-28T12"],
            [0., 10., 0., 0.],
            1,
        )],
    );

    assert!(
        week.days.iter().all(|tokens| *tokens == 0),
        "{:?}",
        week.days
    );
}

#[test]
fn a_project_is_the_last_part_of_its_path_and_the_branch_most_of_it_was_on() {
    let week = only(
        BY_PROJECT,
        vec![
            tallied(
                &["assistant", "/home/a/Work/crook/", "pirate"],
                [0., 30., 0., 0.],
                2,
            ),
            tallied(
                &["assistant", "/home/a/Work/crook", "main"],
                [0., 10., 0., 0.],
                1,
            ),
        ],
    );

    assert_eq!(week.projects.len(), 1, "{:?}", week.projects);
    assert_eq!(week.projects[0].name, "crook");
    assert_eq!(week.projects[0].branch.as_deref(), Some("pirate"));
    assert_eq!(week.projects[0].tokens, 40);
}

#[test]
fn a_detached_head_names_nothing_a_person_would_recognise() {
    let week = only(
        BY_PROJECT,
        vec![tallied(
            &["assistant", "/home/a/Work/crook", "HEAD"],
            [0., 10., 0., 0.],
            1,
        )],
    );

    assert_eq!(week.projects[0].branch, None);
}

#[test]
fn sessions_are_counted_by_there_being_one_row_each() {
    let week = only(
        BY_SESSION,
        vec![
            tallied(&["assistant", "one"], [0.; 4], 12),
            tallied(&["assistant", "two"], [0.; 4], 3),
            tallied(&["user", "three"], [0.; 4], 3),
        ],
    );

    assert_eq!(
        week.sessions, 2,
        "a session that answered nothing is not one"
    );
}

#[test]
fn the_heaviest_model_is_first_and_so_is_the_heaviest_project() {
    let mut tables = vec![Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    tables[BY_MODEL] = vec![
        tallied(&["assistant", "claude-haiku-4-5"], [0., 10., 0., 0.], 1),
        tallied(&["assistant", "claude-opus-5"], [0., 90., 0., 0.], 3),
    ];
    tables[BY_PROJECT] = vec![
        tallied(&["assistant", "/home/a/small", ""], [0., 10., 0., 0.], 1),
        tallied(&["assistant", "/home/a/big", ""], [0., 90., 0., 0.], 3),
    ];

    let week = week_of(tables);

    assert_eq!(week.models[0].id, "claude-opus-5");
    assert_eq!(week.projects[0].name, "big");
}

#[test]
fn models_are_named_the_way_a_person_writes_them() {
    for (id, name) in [
        ("claude-opus-5", "Opus 5"),
        ("claude-fable-5-1", "Fable 5.1"),
        // A trailing release date is noise in a panel this size.
        ("claude-haiku-4-5-20251001", "Haiku 4.5"),
        // Anything that does not fit the pattern is the most useful thing
        // there is to say about it, which is what it was called.
        ("gpt-4o", "gpt-4o"),
        ("opus", "Opus"),
    ] {
        let model = Model {
            id: id.into(),
            ..Model::default()
        };
        assert_eq!(model.name(), name, "{id}");
    }
}

#[test]
fn what_it_asks_the_host_for_is_one_walk_and_four_ways_of_looking_at_it() {
    // Several tables rather than several requests, because the walk is the
    // expensive part: asking twice would read three hundred megabytes twice.
    stub::forget();
    stub::set_now(NOW);
    let _ = Reading::start();

    let asked = stub::taken();
    let Some((
        _,
        Request::Tally {
            tables,
            distinct_by,
            at_least,
            containing,
            ..
        },
    )) = asked.requests.first()
    else {
        panic!("it asked for something else: {:?}", asked.requests);
    };

    assert_eq!(tables.len(), 4);
    // Every table keys on the one thing the host cannot filter on.
    for table in tables {
        assert_eq!(table.by[KIND].field, "type");
    }
    // And each on one subject of its own. The model is on the models table and
    // nowhere else: keying the sessions table by it counted a session that
    // used two models twice.
    assert_eq!(tables[BY_MODEL].by[SUBJECT].field, "message.model");
    assert_eq!(tables[BY_SESSION].by[SUBJECT].field, "sessionId");
    // The hour, which is how a plugin asks for a day without the host knowing
    // what a time zone is.
    assert_eq!(tables[BY_HOUR].by[SUBJECT].prefix, Some(HOUR));
    assert_eq!(distinct_by.len(), 2, "a turn is a message and a request");
    assert_eq!(
        at_least.len(),
        1,
        "the window has to be a line-by-line floor"
    );
    assert_eq!(containing, "\"usage\"");
}
