//! The Claude Code usage chip, as a plugin Crook does not carry.
//!
//! A pirate in the header saying how much of the session budget is spent, and
//! a panel under him saying what it is made of. Crook used to have this in the
//! box; it is here now, outside the binary, installed as one file — which is
//! the whole point of it. Everything below is what a plugin from a store may
//! do, and nothing else: it has no network, no filesystem and no thread. It
//! describes, it asks, and it is answered.
//!
//! # What this file is
//!
//! The ABI, and nothing that thinks. Every export the host calls is here,
//! every one of them is three lines, and each hands straight over to
//! [`state`] — so that the part which decides anything is a plain Rust module
//! that `cargo test` can run on an ordinary machine. See [`sys`] for the other
//! half of that trick.
//!
//! # The one global
//!
//! wasm32 is single-threaded and the host calls these one at a time, so the
//! plugin's state is a `static` reached through an [`UnsafeCell`]. That is the
//! ordinary shape for a wasm guest and it is sound for the reason it is
//! ordinary: there is no second thread to race with, and no export below is
//! re-entrant — the host refuses to call in while it is already inside.

use std::cell::UnsafeCell;

use crook_plugin_api::{ABI_VERSION, Answer, Capability, Manifest, Node, from_bytes, to_bytes};

pub mod claude;
pub mod history;
pub mod state;
pub mod sys;
pub mod time;
pub mod view;

use state::Pirate;

/// A `static` that is only ever touched by one thread, which on wasm32 is
/// every thread there is.
struct Single<T>(UnsafeCell<T>);

// SAFETY: wasm32 has one thread. Off wasm this crate is a library under test,
// where each test gets its own `Pirate` rather than this one.
unsafe impl<T> Sync for Single<T> {}

impl<T> Single<T> {
    /// SAFETY: the caller must not be inside another borrow. Every export
    /// below takes one, does its work and returns, and the host does not call
    /// in while it is already inside.
    #[allow(clippy::mut_from_ref)]
    unsafe fn get(&self) -> &mut T {
        unsafe { &mut *self.0.get() }
    }
}

/// Everything the plugin knows.
static PIRATE: Single<Option<Pirate>> = Single(UnsafeCell::new(None));

/// The last thing handed back to the host, kept alive until the next one.
///
/// A tree is answered as an offset and a length into this memory, so the bytes
/// have to outlive the call that returned them. Kept rather than leaked
/// because a render happens every frame, and a leak per frame is a plugin that
/// eventually stops fitting in its own sixteen megabytes.
static ANSWER: Single<Vec<u8>> = Single(UnsafeCell::new(Vec::new()));

/// Packs an answer as the host reads it: `(pointer << 32) | length`.
fn hand_back(bytes: Vec<u8>) -> i64 {
    // SAFETY: see `Single::get`.
    let answer = unsafe { ANSWER.get() };
    *answer = bytes;
    ((answer.as_ptr() as u64) << 32 | answer.len() as u64) as i64
}

/// The plugin's own state, made on first use.
fn pirate() -> &'static mut Pirate {
    // SAFETY: see `Single::get`.
    unsafe { PIRATE.get() }.get_or_insert_with(Pirate::new)
}

/// Which version of the vocabulary this was built against.
///
/// Called before anything else, and a mismatch is a refusal by number rather
/// than a plugin that decodes a shape which means something else now.
#[unsafe(no_mangle)]
pub extern "C" fn crook_abi_version() -> i32 {
    ABI_VERSION as i32
}

/// Somewhere for the host to put a string it is handing over.
///
/// Exact rather than `Vec::with_capacity`, because [`take`] frees it with the
/// same layout and a capacity the allocator rounded up would be a free of the
/// wrong size.
#[unsafe(no_mangle)]
pub extern "C" fn crook_alloc(length: i32) -> i32 {
    let Ok(layout) = std::alloc::Layout::from_size_align(length.max(1) as usize, 1) else {
        return 0;
    };
    // SAFETY: a non-zero size, and a layout built for it.
    unsafe { std::alloc::alloc(layout) as i32 }
}

/// Copies out what the host wrote there, and gives the memory back.
///
/// SAFETY: `pointer` and `length` must be exactly what a previous
/// [`crook_alloc`] answered and what the host wrote into.
unsafe fn take(pointer: i32, length: i32) -> Vec<u8> {
    if pointer <= 0 || length < 0 {
        return Vec::new();
    }
    // SAFETY: the host wrote `length` bytes at `pointer` before calling in.
    let bytes =
        unsafe { std::slice::from_raw_parts(pointer as *const u8, length as usize) }.to_vec();
    // SAFETY: the same layout `crook_alloc` used.
    unsafe {
        std::alloc::dealloc(
            pointer as *mut u8,
            std::alloc::Layout::from_size_align_unchecked(length.max(1) as usize, 1),
        );
    }
    bytes
}

/// What this plugin is and what it needs to be allowed to do.
///
/// Read before any of it runs, which is what lets a person see what it wants
/// and refuse it without running a line of it.
#[unsafe(no_mangle)]
pub extern "C" fn crook_manifest() -> i64 {
    hand_back(to_bytes(&manifest()).unwrap_or_default())
}

/// The manifest, as a value, so that a test can read it.
pub fn manifest() -> Manifest {
    Manifest {
        abi: ABI_VERSION,
        id: String::from("theguriev/pirate"),
        name: String::from("Pirate"),
        description: String::from(
            "How much of the Claude Code session budget is spent, and when it resets.",
        ),
        version: String::from(env!("CARGO_PKG_VERSION")),
        capabilities: vec![
            Capability::ReadFiles(vec![
                String::from(claude::CREDENTIALS_PATH),
                String::from(history::TRANSCRIPTS_GRANT),
            ]),
            Capability::Network(vec![String::from(claude::USAGE_HOST)]),
        ],
    }
}

/// Registers the chip and the actions, and starts the first reading.
///
/// **From nothing, every time.** A build is not resumed: the host builds a
/// plugin again when it is switched back on and when a person answers what it
/// asked to be allowed, and what was left of the previous life is worse than
/// useless — a timer this plugin is waiting on that nobody will ever fire
/// again, and a ticket nobody will ever answer. So the state is replaced
/// rather than reused, which is the same promise the host makes about a plugin
/// it rebuilds.
#[unsafe(no_mangle)]
pub extern "C" fn crook_build() -> i32 {
    // SAFETY: see `Single::get`.
    let held = unsafe { PIRATE.get() };
    *held = Some(Pirate::new());
    held.get_or_insert_with(Pirate::new).build();
    0
}

/// What to draw in one slot.
#[unsafe(no_mangle)]
pub extern "C" fn crook_render(slot: i32, length: i32) -> i64 {
    // SAFETY: the host allocated and wrote this before calling in.
    let slot = unsafe { take(slot, length) };
    let tree = match std::str::from_utf8(&slot) {
        Ok("header.right") => view::chip(pirate()),
        // A slot this plugin does not contribute to, which cannot happen and
        // is drawn as nothing rather than guessed at.
        _ => Node::Empty,
    };
    hand_back(to_bytes(&tree).unwrap_or_default())
}

/// Runs one of the actions registered while building.
#[unsafe(no_mangle)]
pub extern "C" fn crook_run(name: i32, length: i32) -> i32 {
    // SAFETY: as above.
    let name = unsafe { take(name, length) };
    match std::str::from_utf8(&name) {
        Ok(action) => pirate().run(action),
        Err(_) => return 1,
    }
    0
}

/// The answer to something this plugin asked for.
#[unsafe(no_mangle)]
pub extern "C" fn crook_deliver(ticket: i32, bytes: i32, length: i32) -> i32 {
    // SAFETY: as above.
    let bytes = unsafe { take(bytes, length) };
    match from_bytes::<Answer>(&bytes) {
        Ok(answer) => pirate().deliver(ticket, answer),
        // An answer this build cannot read is a host speaking a version this
        // one does not, which the ABI check should already have caught.
        Err(_) => return 1,
    }
    0
}

/// The wait asked for has passed.
#[unsafe(no_mangle)]
pub extern "C" fn crook_tick() -> i32 {
    pirate().tick();
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_says_what_it_needs_in_sentences_a_person_can_refuse() {
        let manifest = manifest();

        assert_eq!(manifest.abi, ABI_VERSION);
        assert_eq!(manifest.id, "theguriev/pirate");
        let sentences: Vec<String> = manifest
            .capabilities
            .iter()
            .map(Capability::sentence)
            .collect();
        assert_eq!(
            sentences,
            vec![
                String::from(
                    "Read ~/.claude/.credentials.json, everything under ~/.claude/projects"
                ),
                String::from("Reach api.anthropic.com"),
            ]
        );
    }

    #[test]
    fn what_it_asks_for_is_exactly_what_it_uses() {
        // The one invariant tying the manifest to the code: a plugin that
        // asked for less than it uses is refused at runtime, and one that asks
        // for more is one nobody should allow.
        let keys: Vec<String> = manifest()
            .capabilities
            .iter()
            .flat_map(Capability::keys)
            .collect();

        assert!(keys.contains(&format!("file:{}", claude::CREDENTIALS_PATH)));
        assert!(keys.contains(&format!("file:{}", history::TRANSCRIPTS_GRANT)));
        assert!(keys.contains(&format!("net:{}", claude::USAGE_HOST)));
        assert!(
            claude::USAGE_URL.starts_with(&format!("https://{}/", claude::USAGE_HOST)),
            "the URL it fetches is not on the host it asks for"
        );
        assert_eq!(keys.len(), 3, "it asks for something it does not use");
    }

    #[test]
    fn the_manifest_crosses_the_wire_as_itself() {
        let bytes = to_bytes(&manifest()).expect("a manifest should encode");

        assert_eq!(
            from_bytes::<Manifest>(&bytes).expect("and decode"),
            manifest()
        );
    }
}
