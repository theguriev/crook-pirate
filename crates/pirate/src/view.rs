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

use crook_plugin_api::{Gap, Node, Size, Tone};

use crate::claude::{Limit, Reading};
use crate::state::{Pirate, Problem};
use crate::time::format_countdown;

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
/// about to run out, and on what".
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
        rows.push(Node::Note {
            text: problem.message(),
            tone: Tone::Muted,
        });
    }

    rows.push(Node::Rule);
    rows.push(Node::Note {
        text: String::from(
            "Read from the session Claude Code already keeps on this machine, and asked of \
             Anthropic once a minute. Sent nowhere else.",
        ),
        tone: Tone::Muted,
    });
    rows.push(Node::Row(vec![
        Node::Fill,
        Node::Button {
            label: String::from("Refresh"),
            action: String::from("refresh"),
            tone: Tone::Accent,
        },
    ]));

    Node::Column(rows)
}

/// What Claude says is left: the session, the week, and any credits past them.
fn limits(reading: &Reading, now: i64) -> Vec<Node> {
    let mut rows = vec![];
    rows.extend(limit("Session", reading.session, now));

    if let Some(weekly) = reading.weekly {
        rows.push(Node::Rule);
        rows.extend(limit("Week", weekly, now));
    }

    if let Some((used, monthly)) = reading.extra {
        rows.push(Node::Rule);
        rows.push(Node::Row(vec![
            Node::Text {
                text: String::from("Extra usage"),
                size: Size::Small,
                tone: Tone::Muted,
            },
            Node::Fill,
            Node::Text {
                text: format!("${used:.0} of ${monthly:.0}"),
                size: Size::Small,
                tone: Tone::Primary,
            },
        ]));
    }

    rows
}

/// One limit: its name, its percentage, a bar, and what is left of its window.
fn limit(name: &str, limit: Limit, now: i64) -> Vec<Node> {
    let percent = limit.percent.clamp(0., 100.);
    let tone = band(percent);

    let mut rows = vec![
        Node::Row(vec![
            Node::Text {
                text: String::from(name),
                size: Size::Small,
                tone: Tone::Muted,
            },
            Node::Fill,
            Node::Text {
                text: format!("{}%", rounded(percent)),
                size: Size::Small,
                tone,
            },
        ]),
        Node::Meter {
            fraction: percent / 100.,
            tone,
        },
    ];

    if let Some(resets_at) = limit.resets_at {
        rows.push(Node::Text {
            text: format!("resets in {}", format_countdown(resets_at - now)),
            size: Size::Small,
            tone: Tone::Muted,
        });
    }

    rows
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
