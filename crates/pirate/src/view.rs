//! What the chip and the panel are, said in the host's vocabulary.
//!
//! Nothing here is a colour, a pixel or a font. A [`Node`] says what a thing
//! *is* and Crook decides what that looks like in whatever theme is in force,
//! which is why this plugin looks like part of the application in a theme it
//! has never heard of.
//!
//! # Three bands where the chip had four
//!
//! The version in the box coloured the percentage from a table of four:
//! normal, elevated, high, critical. The vocabulary has three tones that mean
//! "a reading" — [`Tone::Success`], [`Tone::Warning`], [`Tone::Danger`] — and
//! a plugin that wanted a fourth would be a plugin naming a colour, which is
//! the one thing this tier may not do. So elevated and normal are one tone
//! here. That is the honest cost of describing rather than painting, and it is
//! a smaller cost than it looks: the band is a warning system, and it warns at
//! the same two thresholds it always did.
//!
//! # Three sizes, each meaning one thing
//!
//! The panel used to set everything at the small size, so a section's name,
//! a model's name and the caption under it were told apart by tone alone and
//! the eye had nothing to land on. Now each of the host's three sizes means
//! one thing and nothing else. [`Size::Large`] is a *reading*: the two
//! percentages Anthropic reports, and only those, because they are the
//! warning system and the whole reason the panel opens. [`Size::Body`] is
//! what a row is *about*: its name — Session, Week, Extra usage — and, where a
//! row sums to something that has no band, that sum at the same size, so the
//! week's total and the dollars past the limits sit beside a name as facts
//! and are never read as a third percentage. [`Size::Small`] is everything
//! that qualifies one of those: a caption, a countdown, a detail line, the
//! quiet figure at the end of a model's or a project's row. A fact from the
//! transcripts is never set Large, however big the number, so that a week's
//! tokens can never be read as a peer of a limit — different windows,
//! different weights, and nothing here derives one from the other.
//!
//! # Room is grouping
//!
//! [`Gap::Small`] is the room within a line and within a list: a label and
//! its meter, a figure and its unit, a name and its branch, and one model
//! from the next — the detail line under a model's name gets none at all, so
//! that the list reads as pairs. [`Gap::Large`] puts one section apart from
//! the next. [`Gap::Medium`] is the room under a heading, over a rule, above
//! the sentence that qualifies both readings at once, and beside a name on
//! its own line, where a word's worth at the body size is too little for a
//! countdown at the small one. A [`Node::Rule`] is not a grouping device: it is drawn
//! exactly where two things are not the same measurement, which is between
//! what Anthropic says is left and what this machine did, and once more over
//! the sentence that says so. Everything else is told apart by air.

use crook_plugin_api::{Gap, Node, Size, Tone};

use crate::claude::{Limit, Reading};
use crate::history::{Model, Project, Week};
use crate::state::{Pirate, Problem};
use crate::time::{format_countdown, weekday_initial};

/// What the pill prints before the first reading lands.
///
/// An en dash, not a hyphen: a chip that showed nothing at all would read as a
/// bug rather than as a number that has not arrived.
const UNREAD: &str = "\u{2013}";

/// Where the warning band starts, and where the critical one does.
///
/// The same two thresholds the chip used when it was in the box, so that a
/// person who upgrades does not find the colour changing under a number that
/// did not.
const WARNING_AT: f32 = 80.;
/// See [`WARNING_AT`].
const DANGER_AT: f32 = 95.;

/// The whole contribution: the mark, the number, and the panel under them.
pub fn chip(pirate: &Pirate) -> Node {
    Node::Anchored {
        content: Box::new(Node::Pressable {
            content: Box::new(Node::Row(vec![
                Node::Icon {
                    name: String::from(pirate.mark()),
                    tone: mark_tone(pirate),
                },
                Node::Gap(Gap::Small),
                Node::Text {
                    text: label(pirate),
                    size: Size::Small,
                    tone: label_tone(pirate),
                },
            ])),
            action: String::from("panel"),
        }),
        // The plugin's state, not the host's: shut on every frame it is shut,
        // which is nearly all of them.
        panel: pirate.panel_open().then(|| Box::new(panel(pirate))),
        dismiss: String::from("dismiss"),
    }
}

/// What the pill says: a percentage when there is a usable one, and what is
/// wrong when there is not.
fn label(pirate: &Pirate) -> String {
    match (pirate.usable_reading(), pirate.problem()) {
        (Some(reading), _) => format!("{}%", rounded(reading.session.percent)),
        (None, Some(problem)) => String::from(problem.chip_label()),
        (None, None) => String::from(UNREAD),
    }
}

/// What colour to say it in: the band while the reading is current, and the
/// muted grey the unread chip uses the moment it is not.
///
/// A percentage the last cycle failed to refresh is still the best answer
/// available, and it has to be visibly not a fresh one.
fn label_tone(pirate: &Pirate) -> Tone {
    if pirate.problem().is_some() {
        return Tone::Muted;
    }
    match pirate.usable_reading() {
        Some(reading) => band(reading.session.percent),
        None => Tone::Success,
    }
}

/// What the mark is painted in.
///
/// The same rule the percentage follows: a stale chip has to read as stale,
/// and a mark that stayed bright beside a greyed-out number would be the
/// loudest thing in the row saying otherwise. Every tone but
/// [`Tone::Muted`] leaves the pirate his own yellow — the host decides that,
/// because the artwork is the host's.
fn mark_tone(pirate: &Pirate) -> Tone {
    if pirate.problem().is_some() {
        Tone::Muted
    } else {
        Tone::Primary
    }
}

/// Which band a percentage falls in.
fn band(percent: f32) -> Tone {
    if percent >= DANGER_AT {
        Tone::Danger
    } else if percent >= WARNING_AT {
        Tone::Warning
    } else {
        Tone::Success
    }
}

/// The percentage, clamped and rounded the way it is drawn.
fn rounded(percent: f32) -> u32 {
    percent.clamp(0., 100.).round() as u32
}

/// What hangs under the chip: the limits, and where the number came from.
///
/// The chip has room for one number, and one number is not an answer to "am I
/// about to run out, and on what". Drawing this is also the only thing that
/// refreshes it — see [`crate::state`] — which is why there is nothing here to
/// press.
fn panel(pirate: &Pirate) -> Node {
    let mut rows: Vec<Node> = Vec::new();

    match pirate.usable_reading() {
        Some(reading) => rows.extend(limits(reading, crate::sys::now())),
        // No reading at all: say why in a sentence rather than drawing two
        // empty bars, which would read as "nothing used yet".
        None => rows.push(Node::Note {
            text: pirate
                .problem()
                .map(Problem::message)
                .unwrap_or_else(|| String::from("Reading Claude usage\u{2026}")),
            tone: Tone::Muted,
        }),
    }

    // A reading that failed to refresh is still drawn — it is the best number
    // there is — with the failure named underneath, which is the same rule the
    // chip follows when it greys the percentage out.
    if let (Some(problem), Some(_)) = (pirate.problem(), pirate.usable_reading()) {
        // Under both limits rather than under the second: it is about the
        // refresh, which is one request for the pair of them.
        rows.push(Node::Gap(Gap::Medium));
        rows.push(Node::Note {
            text: problem.message(),
            tone: Tone::Muted,
        });
    }

    rows.extend(seam());
    rows.extend(week(pirate));

    rows.extend(seam());
    // And no Refresh button under it. Opening the panel is what refreshes it,
    // so a button here would be an offer to do again, by hand, the thing that
    // has just been done — against an endpoint whose whole problem is being
    // asked twice.
    rows.push(Node::Note {
        text: String::from(
            "The limits come from Anthropic, read when this panel opens; the week comes from \
             the transcripts Claude Code writes on this machine, read where they are and sent \
             nowhere.",
        ),
        tone: Tone::Muted,
    });

    Node::Column(rows)
}

/// The hairline between two things that are not the same measurement.
///
/// With the room above it that the host gives only two pixels of: a rule
/// carries nearly all of its margin underneath, because it belongs to the
/// section it opens, and a meter drawn all but straight onto it reads as
/// underlined rather than as ended. Eight and the host's two make the ten
/// below, so the seam sits in the middle of its room.
fn seam() -> [Node; 2] {
    [Node::Gap(Gap::Medium), Node::Rule]
}

/// What Claude says is left: the session, the week, and any credits past them.
fn limits(reading: &Reading, now: i64) -> Vec<Node> {
    let mut rows = limit("Session", reading.session, now);

    if let Some(weekly) = reading.weekly {
        rows.push(Node::Gap(Gap::Large));
        rows.extend(limit("Week", weekly, now));
    }

    if let Some((used, monthly)) = reading.extra {
        rows.push(Node::Gap(Gap::Large));
        // Money, at the size of a name rather than of a reading: a dollar
        // figure has no band, and a number set as large as the percentages
        // would be asking to be read as a third one.
        rows.push(Node::Row(vec![
            Node::Text {
                text: String::from("Extra usage"),
                size: Size::Body,
                tone: Tone::Primary,
            },
            Node::Fill,
            Node::Text {
                text: format!("${used:.0} of ${monthly:.0}"),
                size: Size::Body,
                tone: Tone::Primary,
            },
        ]));
    }

    rows
}

/// One limit: its name and what is left of its window on one line, the
/// reading at the far end of it, and the bar underneath.
///
/// Two lines where there were three. The countdown sits beside the name
/// rather than under the bar because it qualifies the name — "Session, for
/// another three hours" — and a caption separated from its noun by a meter
/// read as a caption for the meter.
fn limit(name: &str, limit: Limit, now: i64) -> Vec<Node> {
    let percent = limit.percent.clamp(0., 100.);
    let tone = band(percent);

    let mut row = vec![Node::Text {
        text: String::from(name),
        size: Size::Body,
        tone: Tone::Primary,
    }];
    if let Some(resets_at) = limit.resets_at {
        row.push(Node::Gap(Gap::Medium));
        row.push(Node::Text {
            text: format!("resets in {}", format_countdown(resets_at - now)),
            size: Size::Small,
            tone: Tone::Muted,
        });
    }
    row.push(Node::Fill);
    row.push(Node::Text {
        text: format!("{}%", rounded(percent)),
        size: Size::Large,
        tone,
    });

    vec![
        Node::Row(row),
        Node::Gap(Gap::Small),
        Node::Meter {
            fraction: percent / 100.,
            tone,
        },
    ]
}

/// What this machine did with the week: per day, per model, per project.
///
/// Two sources, and the seam between them is drawn rather than hidden: the
/// block above is what Anthropic says is left, this one is what happened here,
/// and a week's tokens do not add up to a percentage of a limit — different
/// windows, different weights, and a cache read is not priced like a token the
/// model wrote. Nothing here pretends to derive one from the other, which is
/// also why nothing below the seam is set at the size of a reading.
fn week(pirate: &Pirate) -> Vec<Node> {
    let Some(week) = pirate.week() else {
        return vec![
            heading("Last 7 days", None),
            Node::Gap(Gap::Small),
            Node::Note {
                text: String::from(if pirate.is_reading_the_week() {
                    "Reading this machine's transcripts\u{2026}"
                } else {
                    "No transcripts read yet"
                }),
                tone: Tone::Muted,
            },
        ];
    };

    if week.is_empty() {
        return vec![
            heading("Last 7 days", None),
            Node::Gap(Gap::Small),
            Node::Note {
                text: String::from(
                    "Nothing in the last 7 days. This counts the turns Claude Code writes to \
                     this machine.",
                ),
                tone: Tone::Muted,
            },
        ];
    }

    let total = week.tokens();
    let busiest = week.days.iter().copied().max().unwrap_or(1).max(1);
    let mut rows = vec![
        heading("Last 7 days", Some(total)),
        Node::Gap(Gap::Medium),
        // Shares of the busiest day rather than of anything absolute: nobody
        // reads the height, they read which day was the busy one.
        Node::Bars {
            values: week
                .days
                .iter()
                .map(|tokens| *tokens as f32 / busiest as f32)
                .collect(),
            tone: Tone::Accent,
        },
        Node::Gap(Gap::Small),
        weekdays(week),
        Node::Gap(Gap::Small),
        // The chart's caption: what the columns add up to, in things counted
        // rather than measured.
        Node::Note {
            text: format!(
                "{} turns \u{00b7} {} sessions \u{00b7} {} models",
                thousands(week.turns),
                thousands(week.sessions),
                week.models.len()
            ),
            tone: Tone::Muted,
        },
        Node::Gap(Gap::Large),
        caption("Models"),
        Node::Gap(Gap::Small),
    ];

    for (index, model) in week.models.iter().enumerate() {
        if index > 0 {
            rows.push(Node::Gap(Gap::Small));
        }
        rows.extend(model_rows(model, total));
    }

    if !week.projects.is_empty() {
        rows.push(Node::Gap(Gap::Large));
        rows.push(caption("Projects"));
        rows.push(Node::Gap(Gap::Small));
        // No air between the rows: each is one line, and four of them under
        // one caption are a table, which reads tight.
        for project in &week.projects {
            rows.push(project_row(project));
        }
    }

    rows
}

/// A section's name, quietly.
fn caption(title: &str) -> Node {
    Node::Text {
        text: String::from(title),
        size: Size::Small,
        tone: Tone::Muted,
    }
}

/// A section's name, with the figure it sums to when there is one.
///
/// The figure is set at the size of a name and carries its unit, so that the
/// one number below the seam set at that size is the one that says what it
/// is — and is never mistaken for a reading.
fn heading(title: &str, total: Option<u64>) -> Node {
    let Some(total) = total else {
        return caption(title);
    };
    Node::Row(vec![
        caption(title),
        Node::Fill,
        Node::Text {
            text: compact(total),
            size: Size::Body,
            tone: Tone::Primary,
        },
        Node::Gap(Gap::Small),
        Node::Text {
            text: String::from("tokens"),
            size: Size::Small,
            tone: Tone::Muted,
        },
    ])
}

/// The weekday under each column of the chart, today's in the ordinary
/// weight and the rest quiet.
///
/// The host spreads the columns evenly, with as much room before the first
/// and after the last as between any two, and there is no node that says
/// "under the third column". What lines up with it is a row of the seven
/// letters with a [`Node::Fill`] between each pair *and* at both ends: eight
/// equal shares of the leftover width around seven glyphs, which is the same
/// arithmetic the columns went through around seven bars. The two differ only
/// by how much narrower a letter is than a bar, and the difference at the
/// outermost column comes to a couple of pixels — under a column thirteen
/// wide, that is under it.
fn weekdays(week: &Week) -> Node {
    let today = week.days.len().saturating_sub(1);
    let mut row = vec![Node::Fill];
    for offset in 0..week.days.len() {
        row.push(Node::Text {
            text: String::from(weekday_initial(week.first_day + offset as i64)),
            size: Size::Small,
            tone: if offset == today {
                Tone::Primary
            } else {
                Tone::Muted
            },
        });
        row.push(Node::Fill);
    }
    Node::Row(row)
}

/// One model: its name and its share of the week, and what the share was made
/// of underneath.
///
/// No meter. Each model used to have one, and on any real week three of the
/// four drew as a dot on an empty track: one model takes nearly everything,
/// and a bar whose whole message is "not this one" is a line spent on
/// nothing. The share says it in two characters, and the muted tone keeps the
/// figures below the seam one column — the total above them is the only
/// number in the local week set in the ordinary weight.
fn model_rows(model: &Model, total: u64) -> Vec<Node> {
    let share = if total == 0 {
        0.
    } else {
        model.tokens() as f32 / total as f32
    };

    vec![
        Node::Row(vec![
            Node::Text {
                text: model.name(),
                size: Size::Small,
                tone: Tone::Primary,
            },
            Node::Fill,
            Node::Text {
                text: format!("{}%", (share * 100.).round()),
                size: Size::Small,
                tone: Tone::Muted,
            },
        ]),
        // What the sum is made of, beside it: cache reads dominate any session
        // long enough to matter, and a reader left to guess which of the four
        // this was would read the total as work the model did.
        Node::Note {
            text: format!(
                "{} out \u{00b7} {} cache \u{00b7} {} turns",
                compact(model.output),
                compact(model.cache_read + model.cache_write),
                thousands(model.turns)
            ),
            tone: Tone::Muted,
        },
    ]
}

/// One project: what it is called, the branch most of it was on, and its
/// tokens.
///
/// The name in the ordinary weight and the branch quiet, as two runs rather
/// than one string, so that which project it is reads before which branch —
/// a branch is a qualifier, and "crook · main" in one tone was a name with
/// the wrong number of words.
fn project_row(project: &Project) -> Node {
    let (name, branch) = project_label(project);

    let mut row = vec![Node::Text {
        text: name,
        size: Size::Small,
        tone: Tone::Primary,
    }];
    if let Some(branch) = branch {
        row.push(Node::Gap(Gap::Small));
        row.push(Node::Text {
            text: format!("\u{00b7} {branch}"),
            size: Size::Small,
            tone: Tone::Muted,
        });
    }
    row.push(Node::Fill);
    row.push(Node::Text {
        text: compact(project.tokens),
        size: Size::Small,
        tone: Tone::Muted,
    });

    Node::Row(row)
}

/// How many characters of a project's label fit beside its figure.
///
/// A count of characters standing in for a width the plugin cannot measure,
/// so it is sized for the widest glyphs a name is likely to be made of —
/// thirty capitals, Cyrillic ones included, still leave the figure a few
/// pixels of air — and an ordinary lowercase name has room to spare. The host
/// neither wraps a run of text nor cuts it at the edge: a label that did not
/// fit would push the figure into the panel's inset and, past that, off it.
/// So a long branch is cut here, and which project it is reads from the start
/// of the name.
const LABEL_CHARS: usize = 30;

/// What the separator between a name and its branch costs of that budget.
const SEPARATOR_CHARS: usize = 3;

/// A project's name and its branch, cut to fit together.
///
/// The branch is what gets cut, because the name is what people call the
/// thing; and a branch with no room left at all is left out rather than
/// reduced to an ellipsis, which would be a qualifier that says nothing.
fn project_label(project: &Project) -> (String, Option<String>) {
    let name = elided(&project.name, LABEL_CHARS);
    let Some(branch) = project.branch.as_deref() else {
        return (name, None);
    };

    let room = LABEL_CHARS.saturating_sub(name.chars().count() + SEPARATOR_CHARS);
    if room < 2 {
        return (name, None);
    }
    (name, Some(elided(branch, room)))
}

/// A label, cut to fit in `chars`.
///
/// The cut is made before the ellipsis rather than at it, and a separator
/// left dangling there goes too: "worktree-…" is a word with a hyphen and a
/// hole, where "worktree…" is a word that goes on.
fn elided(label: &str, chars: usize) -> String {
    if label.chars().count() <= chars {
        return String::from(label);
    }
    let kept: String = label.chars().take(chars.saturating_sub(1)).collect();
    let kept =
        kept.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '_' | '.' | '/'));
    format!("{kept}\u{2026}")
}

/// A number a person reads at a glance rather than counts the digits of.
fn compact(tokens: u64) -> String {
    const THOUSAND: f64 = 1_000.;
    let tokens = tokens as f64;

    for (limit, suffix) in [
        (THOUSAND.powi(3), "B"),
        (THOUSAND.powi(2), "M"),
        (THOUSAND, "k"),
    ] {
        if tokens >= limit {
            let scaled = tokens / limit;
            // One decimal below ten, none above it: 9.4M, then 12M.
            return if scaled < 10. {
                format!("{scaled:.1}{suffix}")
            } else {
                format!("{scaled:.0}{suffix}")
            };
        }
    }

    format!("{tokens:.0}")
}

/// A count with its thousands grouped, for the figures that are counted rather
/// than measured.
fn thousands(count: u64) -> String {
    let digits = count.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);

    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }

    grouped
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
