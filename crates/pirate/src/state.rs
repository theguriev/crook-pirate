//! When to ask, what to remember, and what a person is waiting on.
//!
//! # Everything happens because a tick happened
//!
//! A plugin has one timer, and the host keeps the last thing it was asked for
//! rather than a queue of them. So there is exactly one place that asks — see
//! [`Pirate::arm`] — and the schedule lives in [`Pirate::next_poll_at`] rather
//! than in the length of the wait. A tick is a heartbeat that asks "is it time
//! yet"; two timers outstanding would be two heartbeats, then four, and a
//! plugin that polls Anthropic at an exponentially rising rate is a plugin
//! that gets an account rate-limited.
//!
//! # The mouth means a person is waiting, and nothing else
//!
//! The chomp runs while a refresh *somebody asked for* is in flight, never on
//! a background poll. That is the rule the chip kept when it was in the box,
//! and the reason is unchanged: an animation on a timer nobody is watching
//! repaints the header sixty times for no one.
//!
//! # A failure does not throw the last number away
//!
//! A network blip keeps the percentage and greys it. A session that has gone
//! will never refresh that number, so it replaces it. Which of the two happens
//! is [`Problem::invalidates_the_reading`], not the order of the arms.

use crook_plugin_api::{Answer, Method, Request};

use crate::claude::{self, CREDENTIALS_PATH, OAUTH_BETA, Reading, Session, USAGE_URL};
use crate::sys::{self, Level};

/// How often the reading is refreshed while there is a session to read it
/// with. The endpoint's own window is five hours; a minute is what makes the
/// number in the header true enough to act on.
pub const POLL_MILLIS: i64 = 60_000;

/// How often it is refreshed when there is not one.
///
/// There is nothing to poll until somebody runs Claude Code, and a plugin that
/// asked every minute for a file that is not there would be a plugin spending
/// a wake-up a minute to learn the same thing.
pub const IDLE_POLL_MILLIS: i64 = 10 * 60_000;

/// How long one frame of the bite is held.
pub const CHOMP_MILLIS: i64 = 110;

/// The bite, in the host's own icon names, ending where it starts so that
/// stopping on any frame boundary stops on a whole face.
pub const CHOMP_CYCLE: [&str; 4] = ["pirate", "pirate-open", "pirate-wide", "pirate-open"];

/// The mark when nobody is waiting: a shut mouth.
pub const PIRATE: &str = "pirate";

/// What is wrong, when something is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// Nobody has allowed this plugin to do what it needs yet.
    ///
    /// Carries the sentence the permission dialog printed, so that the panel
    /// can name the thing to go and allow instead of saying that something
    /// went wrong.
    NotAllowed(String),
    /// There is no Claude Code session on this machine.
    NoSession,
    /// There was one and it has stopped working.
    SessionExpired,
    /// Something else: a socket, a proxy, a 500.
    Unreachable,
}

impl Problem {
    /// Whether it makes the last reading meaningless.
    ///
    /// A blip does not: a percentage from a minute ago is still the best
    /// answer there is, drawn grey. A session that has gone does, because that
    /// number will never be refreshed and leaving it up would be the chip
    /// saying something it no longer knows.
    pub fn invalidates_the_reading(&self) -> bool {
        matches!(
            self,
            Self::NoSession | Self::SessionExpired | Self::NotAllowed(_)
        )
    }

    /// What the chip says when there is no percentage to say.
    pub fn chip_label(&self) -> &'static str {
        match self {
            Self::NotAllowed(_) => "not allowed",
            Self::NoSession => "no session",
            Self::SessionExpired => "session expired",
            Self::Unreachable => "unavailable",
        }
    }

    /// The line the panel prints under it, which has to say what to do.
    pub fn message(&self) -> String {
        match self {
            Self::NotAllowed(sentence) => {
                format!("Allow this plugin to: {sentence}. Settings, then Plugins.")
            }
            Self::NoSession => String::from("Run Claude Code once to show usage here."),
            Self::SessionExpired => {
                String::from("The Claude Code session expired — run Claude Code to refresh it.")
            }
            Self::Unreachable => String::from("Couldn't reach Claude."),
        }
    }
}

/// Everything the plugin knows.
#[derive(Debug, Default)]
pub struct Pirate {
    /// The session, once the credentials file has been read.
    session: Option<Session>,
    /// The last reading that arrived, however old.
    reading: Option<Reading>,
    /// What went wrong on the last cycle, if anything.
    problem: Option<Problem>,
    /// Whether the panel is up. The plugin's state, not the host's.
    panel_open: bool,
    /// Whether somebody is waiting on a refresh they asked for.
    busy: bool,
    /// How far into the bite the mouth is.
    chomp: usize,
    /// The ticket the credentials read will answer with.
    reading_credentials: Option<i32>,
    /// The ticket the usage request will answer with.
    fetching: Option<i32>,
    /// When the next background poll is due, in milliseconds since the epoch.
    next_poll_at: i64,
    /// Whether a tick is already coming. See the module note: exactly one.
    ticking: bool,
}

impl Pirate {
    /// A plugin that has not been asked anything yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers everything, and starts the first reading.
    pub fn build(&mut self) {
        sys::contribute("header.right", "chip", 0);
        sys::register_action("refresh", Some("Refresh the usage reading"));
        sys::register_action("panel", Some("Show the usage panel"));
        // Reachable and not offered: it is what a click outside the panel
        // runs, and nobody goes looking for it in a palette.
        sys::register_action("dismiss", None);

        self.refresh(false);
        self.arm();
    }

    /// Runs one of the actions registered above.
    pub fn run(&mut self, action: &str) {
        match action {
            "refresh" => self.refresh(true),
            "panel" => {
                self.panel_open = !self.panel_open;
                // Asked for on the way up only. The figures a person is
                // looking at should not move under them because they clicked
                // the chip again to put the panel away.
                if self.panel_open {
                    self.refresh(true);
                }
            }
            "dismiss" => self.panel_open = false,
            other => sys::log(Level::Warn, &format!("nothing here is called {other:?}")),
        }
        self.arm();
    }

    /// The wait asked for has passed.
    pub fn tick(&mut self) {
        self.ticking = false;

        if self.busy {
            self.chomp = (self.chomp + 1) % CHOMP_CYCLE.len();
        }
        if sys::now() >= self.next_poll_at && !self.is_waiting() {
            self.refresh(false);
        }

        self.arm();
    }

    /// An answer to something asked for.
    pub fn deliver(&mut self, ticket: i32, answer: Answer) {
        if Some(ticket) == self.reading_credentials {
            self.reading_credentials = None;
            self.take_credentials(answer);
        } else if Some(ticket) == self.fetching {
            self.fetching = None;
            self.take_usage(answer);
        } else {
            // An answer to something this no longer cares about: a refresh
            // that was superseded, or a ticket from before a failure. Not
            // worth a state change, and not worth being quiet about either.
            sys::log(
                Level::Info,
                &format!("nothing is waiting on ticket {ticket}"),
            );
            return;
        }

        self.arm();
    }

    /// What the credentials file turned out to hold.
    fn take_credentials(&mut self, answer: Answer) {
        match answer {
            Answer::Read { bytes } => match claude::read_session(&bytes) {
                Some(session) => {
                    let token = session.token.clone();
                    self.session = Some(session);
                    self.fetch(&token);
                }
                // A file with nothing usable in it and no file at all are the
                // same fact: Claude Code has not signed in on this machine.
                None => self.settle(Some(Problem::NoSession)),
            },
            Answer::Refused(sentence) => self.settle(Some(Problem::NotAllowed(sentence))),
            Answer::Failed(_) => self.settle(Some(Problem::NoSession)),
            // The host answering a file read with a fetch would be a bug in
            // the host, and there is nothing useful to draw about it.
            Answer::Fetched { .. } => self.settle(Some(Problem::Unreachable)),
        }
    }

    /// What Claude answered.
    fn take_usage(&mut self, answer: Answer) {
        match answer {
            // A status is an answer about the session, not a transport
            // failure, which is why the host hands it over as it stands.
            Answer::Fetched { status, .. } if status == 401 || status == 403 => {
                self.session = None;
                self.settle(Some(Problem::SessionExpired));
            }
            Answer::Fetched { status, body } if (200..300).contains(&status) => {
                match claude::read_usage(&body) {
                    Some(reading) => {
                        self.reading = Some(reading);
                        self.settle(None);
                    }
                    None => self.settle(Some(Problem::Unreachable)),
                }
            }
            Answer::Fetched { .. } | Answer::Failed(_) | Answer::Read { .. } => {
                self.settle(Some(Problem::Unreachable));
            }
            Answer::Refused(sentence) => self.settle(Some(Problem::NotAllowed(sentence))),
        }
    }

    /// Starts a cycle, unless one is already running.
    ///
    /// `asked_for` is what the mouth means. A second click while a refresh is
    /// in flight starts no second request — but it does start the animation,
    /// because somebody is now waiting on an answer that was already coming.
    fn refresh(&mut self, asked_for: bool) {
        if self.is_waiting() {
            self.busy |= asked_for;
            return;
        }

        self.busy = asked_for;
        self.chomp = 0;

        match self
            .session
            .as_ref()
            .filter(|session| session.is_usable(sys::now()))
        {
            // The token is still good, so the file does not have to be read
            // again: a poll a minute that opened a credentials file a minute
            // would be a poll that shows up in somebody's audit log.
            Some(session) => {
                let token = session.token.clone();
                self.fetch(&token);
            }
            None => self.read_credentials(),
        }
    }

    /// Asks the host for the credentials file.
    fn read_credentials(&mut self) {
        self.reading_credentials = sys::ask(&Request::ReadFile {
            path: String::from(CREDENTIALS_PATH),
        });
        if self.reading_credentials.is_none() {
            self.settle(Some(Problem::Unreachable));
        }
    }

    /// Asks the host for the reading.
    fn fetch(&mut self, token: &str) {
        self.fetching = sys::ask(&Request::Fetch {
            method: Method::Get,
            url: String::from(USAGE_URL),
            headers: vec![
                (String::from("authorization"), format!("Bearer {token}")),
                (String::from("anthropic-beta"), String::from(OAUTH_BETA)),
            ],
            body: None,
        });
        if self.fetching.is_none() {
            self.settle(Some(Problem::Unreachable));
        }
    }

    /// Ends a cycle: what it found, and when to come back.
    fn settle(&mut self, problem: Option<Problem>) {
        self.problem = problem;
        self.busy = false;
        self.chomp = 0;

        let nothing_to_poll = matches!(
            self.problem,
            Some(Problem::NoSession | Problem::NotAllowed(_))
        );
        self.next_poll_at = sys::now()
            + if nothing_to_poll {
                IDLE_POLL_MILLIS
            } else {
                POLL_MILLIS
            };
    }

    /// Asks to be ticked, if a tick is not already coming.
    ///
    /// The one place a timer is asked for. See the module note.
    fn arm(&mut self) {
        if self.ticking {
            return;
        }

        let waiting = if self.busy {
            CHOMP_MILLIS
        } else {
            // A second at the shortest, so that a plugin whose schedule has
            // fallen behind catches up rather than spinning.
            (self.next_poll_at - sys::now()).clamp(1_000, IDLE_POLL_MILLIS)
        };

        sys::set_timer(waiting as i32);
        self.ticking = true;
    }

    /// Whether an answer is already on its way.
    fn is_waiting(&self) -> bool {
        self.reading_credentials.is_some() || self.fetching.is_some()
    }

    /// The reading, if the last failure has not made it meaningless.
    pub fn usable_reading(&self) -> Option<&Reading> {
        self.reading.as_ref().filter(|_| {
            !self
                .problem
                .as_ref()
                .is_some_and(Problem::invalidates_the_reading)
        })
    }

    /// What went wrong last, if anything.
    pub fn problem(&self) -> Option<&Problem> {
        self.problem.as_ref()
    }

    /// Whether the panel is up.
    pub fn panel_open(&self) -> bool {
        self.panel_open
    }

    /// Which of the host's icons to draw: the frame of the bite the mouth is
    /// on, or a shut one when nobody is waiting.
    pub fn mark(&self) -> &'static str {
        if self.busy {
            CHOMP_CYCLE[self.chomp % CHOMP_CYCLE.len()]
        } else {
            PIRATE
        }
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
