//! The panel, standing still for its picture.
//!
//! ```sh
//! cargo run --example fixture > panel.json
//! crook --plugin-fixture panel.json --snapshot panel.png
//! ```
//!
//! A describing plugin's whole surface is a value, and this prints it: the
//! same [`chip`] the host is handed on every frame, carried to a reading and a
//! week that are the same on every machine, in the JSON `--plugin-fixture`
//! reads. So the picture in the README is a picture of what the code draws,
//! taken by the host's own renderer, and nobody has to sign in and wait for a
//! week to look a particular way.
//!
//! One argument picks the scene, and the default is the busy week the README
//! shows. The rest are the states a panel has to look right in and nobody can
//! arrange on demand: `expired` (no reading at all), `stale` (a reading the
//! last refresh could not replace), `danger` (both bands lit, and credits
//! past them), `quiet` (a week with nothing in it) and `reading` (the
//! transcripts still being counted).
//!
//! Off wasm the imports are stubs that answer from a clock this sets, which is
//! the same arrangement `cargo test` runs under.

use crook_plugin_api::{Answer, Cell, Request, Tallied};

use pirate::state::Pirate;
use pirate::sys::stub;
use pirate::view::chip;

/// The stub's clock: 2026-09-04T18:00:00Z, three hours east.
const NOW: i64 = 1_788_544_800_000;

/// A credentials file with a session that does not expire.
const CREDENTIALS: &[u8] = br#"{"claudeAiOauth":{"accessToken":"x"}}"#;

/// What Anthropic says is left, against that clock.
const USAGE: &[u8] = br#"{
    "five_hour": {"utilization": 47.0, "resets_at": "2026-09-04T21:30:00Z"},
    "seven_day": {"utilization": 37.0, "resets_at": "2026-09-09T11:00:00Z"}
}"#;

/// One row of a table, in the shape the host builds one.
fn tallied(key: &[&str], sums: [f64; 4], lines: u64) -> Tallied {
    Tallied {
        key: key
            .iter()
            .map(|part| Cell::Text(String::from(*part)))
            .collect(),
        sums: sums.to_vec(),
        lines,
    }
}

/// A week that looks like a week: one heavy model, three light ones, a busy
/// Sunday, and four projects.
fn tables() -> Vec<Vec<Tallied>> {
    let models = vec![
        tallied(
            &["assistant", "claude-opus-5"],
            [3_100_000., 8_000_000., 214_000_000., 4_900_000_000.],
            24_444,
        ),
        tallied(
            &["assistant", "claude-fable-5-1"],
            [120_000., 319_000., 9_000_000., 120_000_000.],
            1_054,
        ),
        tallied(
            &["assistant", "claude-opus-4-8"],
            [40_000., 145_000., 4_000_000., 57_000_000.],
            148,
        ),
        tallied(
            &["assistant", "claude-haiku-4-5-20251001"],
            [500., 20., 50_000., 673_000.],
            14,
        ),
    ];

    // Oldest first: the six days behind the clock's, then the clock's own.
    let days = [
        ("2026-08-29", 720_000_000.),
        ("2026-08-30", 2_600_000_000.),
        ("2026-08-31", 610_000_000.),
        ("2026-09-01", 90_000_000.),
        ("2026-09-02", 130_000_000.),
        ("2026-09-03", 640_000_000.),
        ("2026-09-04", 510_000_000.),
    ];
    let hours = days
        .iter()
        .map(|(day, tokens)| {
            tallied(
                &["assistant", &format!("{day}T12")],
                [0., 0., 0., *tokens],
                1,
            )
        })
        .collect();

    let projects = vec![
        tallied(
            &["assistant", "/home/eugen/Work/crook/crook", "main"],
            [0., 0., 0., 1_900_000_000.],
            9_000,
        ),
        tallied(
            &[
                "assistant",
                "/home/eugen/.local/share/crook/worktrees/crook/terminal-features",
                "worktree-tidy-and-the-rest",
            ],
            [0., 0., 0., 579_000_000.],
            4_000,
        ),
        tallied(
            &[
                "assistant",
                "/home/eugen/Work/crook/crook-pirate",
                "pirate-ext",
            ],
            [0., 0., 0., 443_000_000.],
            3_000,
        ),
        tallied(
            &["assistant", "/home/eugen/Work/dziling", "dziling"],
            [0., 0., 0., 385_000_000.],
            2_000,
        ),
    ];

    let sessions = (0..72)
        .map(|index| {
            tallied(
                &["assistant", &format!("session-{index}")],
                [0., 0., 0., 0.],
                1,
            )
        })
        .collect();

    vec![models, hours, projects, sessions]
}

/// Which state to draw the panel in.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Scene {
    Busy,
    Expired,
    Stale,
    Danger,
    Quiet,
    Reading,
}

impl Scene {
    fn named(name: &str) -> Option<Self> {
        Some(match name {
            "busy" => Self::Busy,
            "expired" => Self::Expired,
            "stale" => Self::Stale,
            "danger" => Self::Danger,
            "quiet" => Self::Quiet,
            "reading" => Self::Reading,
            _ => return None,
        })
    }

    /// What the endpoint answers, in this scene.
    fn usage(self) -> (u16, &'static [u8]) {
        match self {
            Self::Expired => (401, b""),
            Self::Danger => (
                200,
                br#"{
                    "five_hour": {"utilization": 97.0, "resets_at": "2026-09-04T19:10:00Z"},
                    "seven_day": {"utilization": 86.0, "resets_at": "2026-09-06T11:00:00Z"},
                    "extra_usage": {"is_enabled": true, "monthly_limit": 5000.0, "used_credits": 1234.0}
                }"#,
            ),
            _ => (200, USAGE),
        }
    }
}

/// Answers everything the plugin has asked for, and everything it asks for in
/// reply, until it asks for nothing this scene answers.
///
/// A loop rather than two passes, because the chain has a shape: the
/// credentials are read, and only then is the token spent on the one fetch.
fn answer(pirate: &mut Pirate, answers: impl Fn(&Request) -> Option<Answer>) {
    loop {
        let mut delivered = false;
        for (ticket, request) in stub::taken().requests {
            if let Some(answer) = answers(&request) {
                pirate.deliver(ticket, answer);
                delivered = true;
            }
        }
        if !delivered {
            return;
        }
    }
}

/// Carries the plugin to the scene and prints what it draws.
fn main() {
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| String::from("busy"));
    let Some(scene) = Scene::named(&name) else {
        eprintln!("no scene is called {name:?}: busy, expired, stale, danger, quiet or reading");
        std::process::exit(2);
    };

    stub::forget();
    stub::set_now(NOW);
    stub::set_timezone(180);

    let mut pirate = Pirate::new();
    pirate.build();
    pirate.run("panel");

    let (status, body) = scene.usage();
    answer(&mut pirate, |request| match request {
        Request::ReadFile { .. } => Some(Answer::Read {
            bytes: CREDENTIALS.to_vec(),
        }),
        Request::Fetch { .. } => Some(Answer::Fetched {
            status,
            body: body.to_vec(),
        }),
        // Left unanswered when the transcripts are still being read.
        Request::Tally { .. } if scene == Scene::Reading => None,
        Request::Tally { .. } if scene == Scene::Quiet => Some(Answer::Counted {
            tables: vec![vec![], vec![], vec![], vec![]],
            lines: 0,
        }),
        Request::Tally { .. } => Some(Answer::Counted {
            tables: tables(),
            lines: 25_660,
        }),
        _ => None,
    });

    // Stale: a good reading, then the panel opened again later, and the
    // refresh that opening asked for coming back rate-limited. The number
    // stays and the failure is named under it. The week is asked for again
    // too — a minute is as long as one stays fresh — and answered the same.
    if scene == Scene::Stale {
        pirate.run("dismiss");
        stub::advance(10 * 60_000);
        pirate.run("panel");
        answer(&mut pirate, |request| match request {
            Request::Fetch { .. } => Some(Answer::Fetched {
                status: 429,
                body: Vec::new(),
            }),
            Request::Tally { .. } => Some(Answer::Counted {
                tables: tables(),
                lines: 25_660,
            }),
            _ => None,
        });
    }

    let mut slots = serde_json::Map::new();
    slots.insert(
        String::from(pirate::HEADER_SLOT),
        serde_json::to_value(chip(&pirate)).expect("a node serializes"),
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&slots).expect("a map serializes")
    );
}
