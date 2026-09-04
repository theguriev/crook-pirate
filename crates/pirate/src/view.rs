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
use crate::history::{Model, Project};
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
    rows.extend(week(pirate));

    rows.push(Node::Rule);
    rows.push(Node::Note {
        text: String::from(
            "The limits come from Anthropic; the week comes from the transcripts Claude Code \
             writes on this machine, read where they are and sent nowhere.",
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

/// What this machine did with the week: per model, per day, per project.
///
/// Two sources, and the seam between them is drawn rather than hidden: the
/// block above is what Anthropic says is left, this one is what happened here,
/// and a week's tokens do not add up to a percentage of a limit — different
/// windows, different weights, and a cache read is not priced like a token the
/// model wrote. Nothing here pretends to derive one from the other.
fn week(pirate: &Pirate) -> Vec<Node> {
    let Some(week) = pirate.week() else {
        return vec![
            heading("Last 7 days", None),
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
    let mut rows = vec![
        heading("Last 7 days", Some(&compact(total))),
        // Shares of the busiest day rather than of anything absolute: nobody
        // reads the height, they read which day was the busy one.
        Node::Bars {
            values: week
                .days
                .iter()
                .map(|tokens| {
                    *tokens as f32 / week.days.iter().copied().max().unwrap_or(1).max(1) as f32
                })
                .collect(),
            tone: Tone::Accent,
        },
    ];

    for model in &week.models {
        rows.extend(model_rows(model, total));
    }

    if !week.projects.is_empty() {
        rows.push(Node::Rule);
        rows.push(heading("Projects", None));
        for project in &week.projects {
            rows.push(project_row(project));
        }
    }

    rows.push(Node::Note {
        text: format!(
            "{} turns \u{00b7} {} sessions \u{00b7} {} models",
            thousands(week.turns),
            thousands(week.sessions),
            week.models.len()
        ),
        tone: Tone::Muted,
    });

    rows
}

/// A section's name, with the figure it sums to when there is one.
fn heading(title: &str, figure: Option<&str>) -> Node {
    let mut row = vec![
        Node::Text {
            text: String::from(title),
            size: Size::Small,
            tone: Tone::Muted,
        },
        Node::Fill,
    ];
    if let Some(figure) = figure {
        row.push(Node::Text {
            text: String::from(figure),
            size: Size::Small,
            tone: Tone::Muted,
        });
    }
    Node::Row(row)
}

/// One model: its share of the week, what it was made of, and a bar.
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
                tone: Tone::Primary,
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
        Node::Meter {
            fraction: share,
            tone: Tone::Accent,
        },
    ]
}

/// One project: what it is called, the branch most of it was on, and its
/// tokens.
fn project_row(project: &Project) -> Node {
    let label = match project.branch.as_deref() {
        Some(branch) => format!("{} \u{00b7} {branch}", project.name),
        None => project.name.clone(),
    };

    Node::Row(vec![
        Node::Text {
            text: elided(&label),
            size: Size::Small,
            tone: Tone::Primary,
        },
        Node::Fill,
        Node::Text {
            text: compact(project.tokens),
            size: Size::Small,
            tone: Tone::Muted,
        },
    ])
}

/// How many characters of a label fit before the figure beside it starts to
/// move. A long branch name is cut rather than allowed to push the number off
/// the panel: which project it is reads from the start of the name.
const LABEL_CHARS: usize = 28;

/// A label, cut to fit.
fn elided(label: &str) -> String {
    if label.chars().count() <= LABEL_CHARS {
        return String::from(label);
    }
    let kept: String = label.chars().take(LABEL_CHARS - 1).collect();
    format!("{}\u{2026}", kept.trim_end())
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
