//! The week behind the number: what was spent, on which model, where.
//!
//! The endpoint answers one question — how much of the limit is gone —
//! because that is all a limit knows. It reports no breakdown by model, none
//! by day and none by project, and it never will: those are not facts about a
//! limit. They are facts about what this machine did, and this machine already
//! wrote them down, in `~/.claude/projects/<slug>/<session>.jsonl`, one JSON
//! object per line.
//!
//! # A plugin does not read a hundred megabytes
//!
//! A week of transcripts here is 335MB across 493 files, and the lines that
//! carry a `usage` object are 101MB of that. None of it can cross into a
//! sixteen-megabyte sandbox, and parsing it there would take a minute of an
//! interpreter on the thread that draws.
//!
//! But the *facts* in those 101MB are 41,605 turns of a dozen fields each —
//! about three megabytes. So the host walks the files, skips the ones nothing
//! has touched, never parses a line without the needle in it, and hands back
//! only the fields that were asked for, a page at a time. What is left is this
//! file's, and it is the part that is actually about usage: which lines are
//! the same turn written twice, which fall inside the window, and how they add
//! up.
//!
//! # Three things the format makes you do
//!
//! **Deduplicate.** A resumed or forked session copies the turns it inherited
//! into its own file, so the same answer is on disk several times — about two
//! in five lines. Summing them would overstate a heavy week by that much, so
//! every turn is keyed by the message and the request it came from and counted
//! once.
//!
//! **Trust the line's own timestamp, not the file's.** A file's modification
//! time is only ever used to *skip* one that cannot hold a turn inside the
//! window; a long session's file is touched on every turn, so its early turns
//! are older than it is.
//!
//! **Read days in the machine's own time zone.** A chart of days is read
//! against the days a person lived, and midnight three hours out puts a
//! night's work on the wrong column.

use std::collections::HashMap;

use crook_plugin_api::{Bound, Cell, Key, Request, Table, Tallied};

use crate::sys;
use crate::time::{format_rfc3339, parse_rfc3339};

/// Where Claude Code writes its transcripts.
pub const TRANSCRIPTS: &str = "~/.claude/projects";

/// What the capability asks for, which is everything under that.
pub const TRANSCRIPTS_GRANT: &str = "~/.claude/projects/**";

/// How many days the week covers, which is the window the weekly limit is
/// measured over.
pub const DAYS: i64 = 7;

/// A day, in milliseconds.
const DAY: i64 = 86_400_000;

/// How many projects are worth naming. Past a handful the list stops being a
/// summary and starts being a directory listing.
const TOP_PROJECTS: usize = 4;

/// Claude Code's placeholder for a turn it produced without a model — a
/// cancellation notice, an injected reminder. It costs nothing, and naming it
/// in a breakdown of models would be a bug with a straight face.
const SYNTHETIC_MODEL: &str = "<synthetic>";

/// Where in a key each table puts what.
///
/// Every table starts with the line's `type`, because that is a *filter* the
/// host has no notion of: a line carrying a usage object that is not an
/// assistant turn is not a turn.
const KIND: usize = 0;
/// What each table is grouped by, after that.
const SUBJECT: usize = 1;
/// And the branch, which only the projects table has.
const BRANCH: usize = 2;

/// Which table is which in the answer.
///
/// Only the first is keyed by the model. Keying every table by it looked tidy
/// and was wrong twice over: a session that used two models is one session and
/// would have been counted as two — the sort of error that reads as a busy
/// week — and it tripled every table for the sake of dropping `<synthetic>`,
/// Claude Code's placeholder for a turn it produced without a model. That
/// placeholder has to go from the breakdown of models, where naming it would
/// be a bug with a straight face; everywhere else it costs nothing, because a
/// turn nobody made spent no tokens.
const BY_MODEL: usize = 0;
const BY_HOUR: usize = 1;
const BY_PROJECT: usize = 2;
const BY_SESSION: usize = 3;

/// The four numbers a turn costs, in the order every table sums them.
const TOKENS: [&str; 4] = [
    "message.usage.input_tokens",
    "message.usage.output_tokens",
    "message.usage.cache_creation_input_tokens",
    "message.usage.cache_read_input_tokens",
];

/// How much of a timestamp is its hour: `2026-09-04T18`.
///
/// The hour rather than the day, because which day an hour belongs to is a
/// question about time zones and the host has no idea what one is. Seven days
/// of hours is a hundred and sixty-eight rows, which is nothing.
const HOUR: u32 = 13;

/// What one model was used for over the window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Model {
    /// The model's own id, as the transcript recorded it.
    pub id: String,
    /// Prompt tokens that were neither cached nor written to cache.
    pub input: u64,
    /// Tokens the model produced.
    pub output: u64,
    /// Prompt tokens written into the cache.
    pub cache_write: u64,
    /// Prompt tokens served from it.
    pub cache_read: u64,
    /// Assistant turns answered.
    pub turns: u64,
}

impl Model {
    /// Every token this model moved, which is what its share of the week is.
    ///
    /// Cache reads dominate the sum by an order of magnitude on any session
    /// long enough to matter — that is what a cache is for — so this is a
    /// measure of *traffic* rather than of what a token cost.
    pub fn tokens(&self) -> u64 {
        self.input + self.output + self.cache_write + self.cache_read
    }

    /// The model's name as a person writes it: `claude-opus-5` is Opus 5.
    ///
    /// Derived rather than looked up in a table, because a table would be
    /// wrong the week a new model ships and this is only ever a label. A name
    /// that does not fit the pattern is returned as it was recorded, which is
    /// still the most useful thing that can be said about it.
    pub fn name(&self) -> String {
        let name = self.id.strip_prefix("claude-").unwrap_or(&self.id);
        let mut parts: Vec<&str> = name.split('-').collect();

        // A trailing release date is noise in a panel this size:
        // `haiku-4-5-20251001` is Haiku 4.5.
        if parts
            .last()
            .is_some_and(|part| part.len() == 8 && part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            parts.pop();
        }

        let Some((family, version)) = parts.split_first() else {
            return self.id.clone();
        };
        if family.is_empty()
            || !version
                .iter()
                .all(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return self.id.clone();
        }

        let mut capitalized = String::new();
        let mut letters = family.chars();
        if let Some(first) = letters.next() {
            capitalized.extend(first.to_uppercase());
            capitalized.push_str(letters.as_str());
        }
        if version.is_empty() {
            capitalized
        } else {
            format!("{capitalized} {}", version.join("."))
        }
    }
}

/// One working directory's share of the window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Project {
    /// The directory's last component, which is what people call it.
    pub name: String,
    /// The branch most of its turns were on, when they were on one.
    pub branch: Option<String>,
    /// Every token moved in it.
    pub tokens: u64,
}

/// What the transcripts say about the last [`DAYS`] days.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Week {
    /// Models, heaviest first.
    pub models: Vec<Model>,
    /// Every day in the window, oldest first, including the quiet ones — a gap
    /// in a chart has to be a column of no height rather than a missing one,
    /// or the chart says the week was shorter than it was.
    pub days: Vec<u64>,
    /// The busiest projects, heaviest first, at most [`TOP_PROJECTS`].
    pub projects: Vec<Project>,
    /// How many distinct sessions were open in the window.
    pub sessions: u64,
    /// How many assistant turns they took.
    pub turns: u64,
}

impl Week {
    /// Every token moved in the window.
    pub fn tokens(&self) -> u64 {
        self.models.iter().map(Model::tokens).sum()
    }

    /// Whether the window holds nothing worth drawing.
    ///
    /// A fresh machine, a week off, or a home directory with no transcripts in
    /// it — three states with one answer, which is that the panel says the
    /// week is empty rather than drawing an axis with nothing on it.
    pub fn is_empty(&self) -> bool {
        self.turns == 0
    }
}

/// A walk of the transcripts, in flight.
///
/// Almost nothing, which is the point: the host counts, and what comes back is
/// a few hundred rows of totals rather than forty thousand lines. This holds
/// the ticket and the window, and turns the answer into a [`Week`].
#[derive(Debug, Default)]
pub struct Reading {
    /// The ticket the answer will carry.
    pending: Option<i32>,
    /// The first of the seven local days the chart has a column for.
    first_day: i64,
}

impl Reading {
    /// Asks for the week ending now.
    pub fn start() -> Self {
        let now = sys::now();
        let mut reading = Self {
            pending: None,
            // Not the day the window opens on: seven times twenty-four hours
            // back lands part-way through a day, so counting from *that* day
            // gives eight of them. "The last seven days" is the six behind
            // today and today.
            first_day: day_of(now) - (DAYS - 1),
        };
        reading.ask(now);
        reading
    }

    /// Whether this is the answer it is waiting for.
    pub fn is_waiting_on(&self, ticket: i32) -> bool {
        self.pending == Some(ticket)
    }

    /// Asks the host to count.
    fn ask(&mut self, now: i64) {
        let sums: Vec<String> = TOKENS.iter().map(|field| String::from(*field)).collect();
        let by = |subject: &str, third: Option<&str>| {
            let mut by = vec![
                Key {
                    field: String::from("type"),
                    prefix: None,
                },
                Key {
                    field: String::from(subject),
                    prefix: (subject == "timestamp").then_some(HOUR),
                },
            ];
            if let Some(third) = third {
                by.push(Key {
                    field: String::from(third),
                    prefix: None,
                });
            }
            by
        };

        self.pending = sys::ask(&Request::Tally {
            root: String::from(TRANSCRIPTS),
            extension: String::from(".jsonl"),
            // A file nobody has touched since the window opened cannot hold a
            // turn inside it. This is what makes walking three hundred
            // megabytes affordable at all.
            touched_since: now - DAYS * DAY,
            // And a line with no usage object is not a turn, so it is never
            // parsed.
            containing: String::from("\"usage\""),
            // The file's time only skips files. A file touched an hour ago can
            // hold turns from a month ago, and without this they would be in
            // the totals.
            at_least: vec![Bound {
                field: String::from("timestamp"),
                at_least: format_rfc3339(now - DAYS * DAY),
            }],
            // A resumed or forked session copies the turns it inherited into
            // its own file, so about two lines in five are a second copy.
            distinct_by: vec![String::from("message.id"), String::from("requestId")],
            tables: vec![
                Table {
                    by: by("message.model", None),
                    sum: sums.clone(),
                },
                Table {
                    by: by("timestamp", None),
                    sum: sums.clone(),
                },
                Table {
                    by: by("cwd", Some("gitBranch")),
                    sum: sums.clone(),
                },
                Table {
                    by: by("sessionId", None),
                    sum: Vec::new(),
                },
            ],
        });
    }

    /// Turns what the host counted into the week a panel draws.
    pub fn take(&mut self, tables: Vec<Vec<Tallied>>) -> Week {
        self.pending = None;

        let mut models: Vec<Model> = rows(&tables, BY_MODEL)
            .filter(|row| text(&row.key, SUBJECT) != SYNTHETIC_MODEL)
            .map(|row| Model {
                id: text(&row.key, SUBJECT),
                input: sum(row, 0),
                output: sum(row, 1),
                cache_write: sum(row, 2),
                cache_read: sum(row, 3),
                turns: row.lines,
            })
            .collect();
        models.sort_by(|left, right| {
            right
                .tokens()
                .cmp(&left.tokens())
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut by_day: HashMap<i64, u64> = HashMap::new();
        for row in rows(&tables, BY_HOUR) {
            // An hour, read back as an instant so that the machine's own
            // offset decides which day it belongs to.
            let Some(at) = parse_rfc3339(&format!("{}:00:00Z", text(&row.key, SUBJECT))) else {
                continue;
            };
            let day = day_of(at);
            if day >= self.first_day {
                *by_day.entry(day).or_default() += tokens(row);
            }
        }
        let days = (0..DAYS)
            .map(|offset| {
                by_day
                    .get(&(self.first_day + offset))
                    .copied()
                    .unwrap_or_default()
            })
            .collect();

        let mut totals: HashMap<String, (u64, HashMap<String, u64>)> = HashMap::new();
        for row in rows(&tables, BY_PROJECT) {
            let Some(name) = project_name(&text(&row.key, SUBJECT)) else {
                continue;
            };
            let project = totals.entry(name).or_default();
            project.0 += tokens(row);
            // A detached head reports "HEAD", which names nothing a person
            // would recognise; the project's own name is the better answer.
            let branch = text(&row.key, BRANCH);
            if !branch.is_empty() && branch != "HEAD" {
                *project.1.entry(branch).or_default() += tokens(row);
            }
        }
        let mut projects: Vec<Project> = totals
            .into_iter()
            .map(|(name, (tokens, branches))| Project {
                name,
                branch: branches
                    .into_iter()
                    .max_by_key(|(branch, tokens)| (*tokens, branch.clone()))
                    .map(|(branch, _)| branch),
                tokens,
            })
            .collect();
        projects.sort_by(|left, right| {
            right
                .tokens
                .cmp(&left.tokens)
                .then_with(|| left.name.cmp(&right.name))
        });
        projects.truncate(TOP_PROJECTS);

        Week {
            turns: models.iter().map(|model| model.turns).sum(),
            models,
            days,
            projects,
            // One row per session that answered, which is what a session is.
            sessions: rows(&tables, BY_SESSION).count() as u64,
        }
    }

    /// Whatever it managed to read, when nothing is coming.
    pub fn give_up(&mut self) -> Week {
        self.pending = None;
        Week::default()
    }
}

/// The rows of one table that are assistant turns.
///
/// The filter is here rather than in the request because the host has no
/// notion of it: a line carrying a usage object that is not an assistant turn
/// is not a turn.
fn rows(tables: &[Vec<Tallied>], table: usize) -> impl Iterator<Item = &Tallied> {
    tables
        .get(table)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|row| text(&row.key, KIND) == "assistant")
}

/// One column of a key, as text. A column that is not there is empty.
fn text(key: &[Cell], at: usize) -> String {
    match key.get(at) {
        Some(Cell::Text(text)) => text.clone(),
        _ => String::new(),
    }
}

/// One of a row's sums, as a count.
fn sum(row: &Tallied, at: usize) -> u64 {
    match row.sums.get(at) {
        Some(number) if *number >= 0. => *number as u64,
        _ => 0,
    }
}

/// Every token a row moved.
fn tokens(row: &Tallied) -> u64 {
    (0..TOKENS.len()).map(|at| sum(row, at)).sum()
}

/// Which day an instant falls on, in the machine's own time zone.
///
/// A day number rather than a date, because nothing here prints one: the chart
/// is seven columns in order and the only question asked of a timestamp is
/// which column it belongs in. The offset is the one in force *now* rather
/// than the one in force then, which is wrong for the day the clocks go back
/// and right for the other three hundred and sixty four.
fn day_of(millis: i64) -> i64 {
    let local = millis + i64::from(sys::timezone()) * 60_000;
    local.div_euclid(DAY)
}

/// What a working directory is called, which is its last component.
fn project_name(cwd: &str) -> Option<String> {
    cwd.trim_end_matches('/')
        .rsplit('/')
        .find(|part| !part.is_empty())
        .map(String::from)
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
