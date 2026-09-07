//! When to ask, what to remember, and what a person is waiting on.
//!
//! # Nothing is asked until somebody opens the panel
//!
//! There is no background poll. The endpoint behind the number has a budget
//! small enough that a handful of requests spends it, and it is shared with
//! Claude Code itself — which is the thing a person actually came to use. A
//! chip refreshing itself once a minute in the corner of a window nobody is
//! looking at spends that budget on nobody, and the first thing it costs is
//! the answer to "how much have I got left", which is the one question the
//! chip exists to answer. The plugin used to do exactly that, and the number
//! it drew was "asked too often" often enough to be the only thing it said.
//!
//! So opening the panel is the only thing that asks — see [`Pirate::run`] —
//! and a build asks for nothing at all, which matters because a build happens
//! every time the host is restarted or a grant is answered. The number on the
//! chip between two openings is the one the last opening got, and it is
//! exactly as old as it looks.
//!
//! # A timer is asked for only when there is something to do
//!
//! A plugin has one timer and cannot take an answer back: the host keeps the
//! newest thing it was asked for and drops whatever was coming before it. So
//! there is exactly one place that asks — see [`Pirate::arm`] — and it asks
//! only when it wants to be woken *sooner* than it already will be, because
//! asking again for a moment already booked is a heartbeat that doubles and
//! then quadruples.
//!
//! Three things want a tick: the bite, which is a frame every hundred
//! milliseconds; the watchdog, which is what frees a cycle whose answer never
//! came; and an open panel, whose countdowns go stale if nothing redraws
//! them. When none of them do, nothing is asked for and the plugin costs
//! nothing at all until it is clicked.
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
use crate::history::{self, Week};
use crate::sys::{self, Level};

/// How long to leave the endpoint alone after it says to.
///
/// The one thing that overrides "opening the panel asks". A person who has
/// just been told to ask less often can open and shut the panel as often as
/// they like and nothing goes out until this has passed: a rate limit
/// answered by asking again straight away is a rate limit made worse, and the
/// endpoint is shared with Claude Code itself. Nothing here is urgent — the
/// number on the chip is a percentage of a five-hour window.
pub const BACK_OFF_FOR: i64 = 5 * 60_000;

/// How long one frame of the bite is held.
pub const CHOMP_MILLIS: i64 = 110;

/// How often an open panel is redrawn.
///
/// Nothing is asked of Anthropic on this timer and nothing changes because of
/// it. It exists for the line under each bar — "resets in 40m" — which is
/// worked out from the clock at the moment it is drawn, and which would
/// otherwise sit at the minute the panel happened to open while somebody
/// watched it.
pub const COUNTDOWN_MILLIS: i64 = 30_000;

/// How long a week read stays fresh.
///
/// A minute: the transcripts are read when the panel opens, and opening it
/// twice inside a minute should not walk three hundred megabytes twice for a
/// chart that cannot have changed enough to see.
pub const WEEK_FRESH_FOR: i64 = 60_000;

/// How long to wait for an answer before deciding one is not coming.
///
/// Nothing in the ABI promises an answer. The host takes a request, does the
/// work somewhere else and delivers it later — and "later" is a word with no
/// upper bound in it: the host can fail to hand it over, decide the plugin has
/// asked for too much, or be restarted underneath. A plugin that waited on a
/// ticket forever would then be a chip drawing a number from an hour ago,
/// looking exactly as current as one drawn a second ago, and no click would
/// wake it because a cycle is already "in flight".
///
/// Three minutes, because a request that is genuinely slow is slow in seconds
/// and this must not race one that is merely on a bad network.
pub const GIVE_UP_WAITING_AFTER: i64 = 3 * 60_000;

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
    /// Anthropic said to ask less often.
    ///
    /// Its own state rather than a failure, because it is not one: the request
    /// was right, the session was good, and the answer was "later". Saying
    /// "couldn't reach Claude" to that is telling somebody their network is
    /// broken when it is not.
    RateLimited,
    /// It answered with something that is not an answer.
    Returned(u16),
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
            Self::RateLimited => "asked too often",
            Self::Returned(_) | Self::Unreachable => "unavailable",
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
            Self::RateLimited => String::from(
                "Anthropic is rate-limiting this endpoint, which Claude Code itself shares. \
                 Leaving it alone for a few minutes.",
            ),
            Self::Returned(status) => format!("Claude answered {status}."),
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
    /// The week behind it, once the transcripts have been read.
    week: Option<Week>,
    /// When they were last read, so opening the panel twice does not walk them
    /// twice. See [`WEEK_FRESH_FOR`].
    week_read_at: Option<i64>,
    /// The scan in flight, which is several pages long.
    scanning: Option<history::Reading>,
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
    /// The earliest a new cycle may start, in milliseconds since the epoch.
    ///
    /// Zero nearly always, because opening the panel is allowed to ask. Only
    /// a rate limit sets it — see [`BACK_OFF_FOR`] — which is the one answer
    /// a click is not allowed to argue with.
    not_before: i64,
    /// When the cycle in flight asked for what it is waiting on.
    ///
    /// The watchdog's whole state. See [`GIVE_UP_WAITING_AFTER`].
    waiting_since: Option<i64>,
    /// When the tick it asked for is due, in milliseconds since the epoch.
    ///
    /// The moment rather than a flag, because "is one coming?" is the wrong
    /// question — "is one coming *soon enough*?" is the one that decides
    /// whether to ask again. See the module note.
    waking_at: Option<i64>,
}

impl Pirate {
    /// A plugin that has not been asked anything yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the chip and the two actions, and asks for nothing.
    pub fn build(&mut self) {
        sys::contribute(crate::HEADER_SLOT, "chip", 0);
        sys::register_action("panel", Some("Show the usage panel"));
        // Reachable and not offered: it is what a click outside the panel
        // runs, and nobody goes looking for it in a palette.
        sys::register_action("dismiss", None);

        // And nothing else. A build is not a person asking, and the host
        // builds a plugin again whenever it is switched back on, whenever a
        // grant is answered, and every time it is started — so a build that
        // asked would be a plugin that spends a request on every restart.
        self.arm();
    }

    /// Runs one of the actions registered above.
    ///
    /// Opening the panel is the whole of when this plugin asks Anthropic
    /// anything. See the module note.
    pub fn run(&mut self, action: &str) {
        match action {
            "panel" => {
                self.panel_open = !self.panel_open;
                // Asked for on the way up only. The figures a person is
                // looking at should not move under them because they clicked
                // the chip again to put the panel away.
                if self.panel_open {
                    self.refresh();
                    self.read_the_week();
                }
            }
            "dismiss" => self.panel_open = false,
            other => sys::log(Level::Warn, &format!("nothing here is called {other:?}")),
        }
        self.arm();
    }

    /// The wait asked for has passed.
    ///
    /// It never starts a cycle: a tick is the bite moving, the watchdog
    /// looking, and an open panel being redrawn. Nothing here asks Anthropic
    /// for anything, because nobody clicked.
    pub fn tick(&mut self) {
        self.waking_at = None;
        self.give_up_waiting();

        if self.busy {
            self.chomp = (self.chomp + 1) % CHOMP_CYCLE.len();
        }

        self.arm();
    }

    /// Starts reading the transcripts, unless that was done a moment ago.
    ///
    /// Only ever from the panel, because it is the only thing that draws the
    /// week: walking three hundred megabytes for a chart nobody has opened
    /// would be the plugin spending somebody's disk on nothing.
    fn read_the_week(&mut self) {
        if self.scanning.is_some() {
            return;
        }
        if self
            .week_read_at
            .is_some_and(|at| sys::now() - at < WEEK_FRESH_FOR)
        {
            return;
        }
        self.scanning = Some(history::Reading::start());
    }

    /// An answer to something asked for.
    pub fn deliver(&mut self, ticket: i32, answer: Answer) {
        if self
            .scanning
            .as_ref()
            .is_some_and(|scan| scan.is_waiting_on(ticket))
        {
            self.take_a_page(answer);
            self.arm();
            return;
        }

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

    /// One page of the transcripts.
    fn take_a_page(&mut self, answer: Answer) {
        let Some(scan) = self.scanning.as_mut() else {
            return;
        };

        match answer {
            Answer::Counted { tables, .. } => {
                self.week = Some(scan.take(tables));
                self.week_read_at = Some(sys::now());
                self.scanning = None;
            }
            // Refused or failed: keep whatever was read rather than throwing a
            // half-read week away, and stop. An unreadable home directory is
            // not something a person can act on from a popover, and a chart
            // that says "some of a week" is worse than one that says nothing.
            other => {
                sys::log(Level::Warn, &format!("the transcripts: {other:?}"));
                let week = scan.give_up();
                self.week = (!week.is_empty()).then_some(week);
                self.week_read_at = Some(sys::now());
                self.scanning = None;
            }
        }
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
            // The host answering a file read with something else would be a
            // bug in the host, and there is nothing useful to draw about it.
            _ => self.settle(Some(Problem::Unreachable)),
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
            // Not a failure: the request was right and the answer was
            // "later". Backing off is the whole of what to do about it.
            Answer::Fetched { status: 429, .. } => self.settle(Some(Problem::RateLimited)),
            Answer::Fetched { status, body } if (200..300).contains(&status) => {
                match claude::read_usage(&body) {
                    Some(reading) => {
                        self.reading = Some(reading);
                        self.settle(None);
                    }
                    None => self.settle(Some(Problem::Unreachable)),
                }
            }
            // A status that is none of the above, said with its number: a
            // person who can see 503 knows more than one who is told the
            // network failed.
            Answer::Fetched { status, .. } => self.settle(Some(Problem::Returned(status))),
            Answer::Refused(sentence) => self.settle(Some(Problem::NotAllowed(sentence))),
            // The host answering a fetch with anything else would be a bug in
            // the host, and there is nothing useful to draw about it.
            _ => self.settle(Some(Problem::Unreachable)),
        }
    }

    /// Stops waiting on an answer that is not coming.
    ///
    /// Which frees the next cycle to start. Without this the plugin is
    /// perfectly healthy and permanently asleep: it draws, it ticks, and every
    /// refresh returns early because something it will never hear about is
    /// still "in flight".
    fn give_up_waiting(&mut self) {
        let Some(since) = self.waiting_since else {
            return;
        };
        if sys::now() - since < GIVE_UP_WAITING_AFTER {
            return;
        }

        sys::log(
            Level::Warn,
            "nothing came back from what was asked for; starting again",
        );
        self.reading_credentials = None;
        self.fetching = None;
        self.settle(Some(Problem::Unreachable));
    }

    /// Starts a cycle, unless one is already running.
    ///
    /// Every cycle is one a person asked for, which is why the mouth always
    /// moves. A second click while a refresh is in flight starts no second
    /// request — but it does keep the animation, because somebody is still
    /// waiting on an answer that was already coming.
    fn refresh(&mut self) {
        if self.is_waiting() {
            self.busy = true;
            return;
        }
        // A click during a back-off is not a reason to break it. The endpoint
        // said to ask later and it meant later; asking because somebody opened
        // the panel again is how a rate limit becomes a longer one.
        if sys::now() < self.not_before {
            return;
        }

        self.busy = true;
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
        self.waiting_since = Some(sys::now());
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
        self.waiting_since = Some(sys::now());
        if self.fetching.is_none() {
            self.settle(Some(Problem::Unreachable));
        }
    }

    /// Ends a cycle: what it found, and whether the next click may ask.
    fn settle(&mut self, problem: Option<Problem>) {
        self.problem = problem;
        self.waiting_since = None;
        self.busy = false;
        self.chomp = 0;

        // Only one answer books anything: being told to ask less often. Every
        // other ending leaves the next opening of the panel free to ask.
        self.not_before = match self.problem {
            Some(Problem::RateLimited) => sys::now() + BACK_OFF_FOR,
            _ => 0,
        };
    }

    /// Asks to be ticked, if the tick that is coming is not soon enough.
    ///
    /// The one place a timer is asked for. See the module note: asking again
    /// for a moment already booked is a heartbeat that doubles, and never
    /// asking again is a mark that stands still while somebody watches it.
    fn arm(&mut self) {
        let Some(waiting) = self.wants_a_tick_in() else {
            return;
        };

        let now = sys::now();
        let waking_at = now + waiting;
        if self.waking_at.is_some_and(|booked| booked <= waking_at) {
            return;
        }

        sys::set_timer(waiting as i32);
        self.waking_at = Some(waking_at);
    }

    /// How long until there is something to do, when there is anything.
    ///
    /// `None` is the ordinary state of this plugin: a number on a chip, a
    /// panel that is shut, and nothing that will change either until somebody
    /// clicks. A plugin in that state asks for no timer, so the host never
    /// wakes it and it costs a person nothing.
    fn wants_a_tick_in(&self) -> Option<i64> {
        if self.busy {
            return Some(CHOMP_MILLIS);
        }
        // Not busy and still waiting: the watchdog is the only thing that can
        // free a cycle whose answer nobody is going to deliver.
        if self.is_waiting() {
            return Some(GIVE_UP_WAITING_AFTER);
        }
        self.panel_open.then_some(COUNTDOWN_MILLIS)
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

    /// The week behind the number, once it has been read.
    pub fn week(&self) -> Option<&Week> {
        self.week.as_ref()
    }

    /// Whether the transcripts are being walked right now.
    pub fn is_reading_the_week(&self) -> bool {
        self.scanning.is_some()
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
