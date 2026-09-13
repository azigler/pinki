//! Promise ids: the ones pinki mints, and the ones a caller brings.
//!
//! Ids are opaque (DESIGN.md §1) — nothing reads structure out of one — and that is
//! true of pinki's own ids too, which is why a caller may supply their own. Two
//! spaces, one field:
//!
//! - **Minted**, when `--id` is absent: `pnk_` + exactly six lowercase hex
//!   characters. 16.7M values, which is not a lot; the point is a short, typeable
//!   handle for a human-scale ledger, and collisions are handled by *asking the
//!   ledger* rather than by pretending the space is big enough. Minting takes the set
//!   of ids already present and retries until it finds a free one.
//! - **Supplied**, when `--id` is given: any non-blank string with no whitespace and
//!   no control characters, up to [`MAX_SUPPLIED_LEN`] characters — see
//!   [`check_supplied`]. An adopter arriving with an id space already referenced from
//!   somewhere else carries it in rather than maintaining a mapping, and §7's ledger
//!   join can then key across parties who did not both mint here.
//!
//! The one thing a supplied id may not be is a *malformed* minted id: `pnk_` is
//! reserved for the space this module mints, so `pnk_` followed by anything that is
//! not six lowercase hex digits is refused. The reservation is what keeps
//! [`is_well_formed`] a usable question — everything wearing the prefix really was
//! minted here — and it costs an adopter nothing, since an id space that already
//! starts with `pnk_` is pinki's own.
//!
//! There is no `rand` dependency, and that is a design constraint rather than an
//! inconvenience: the dependency tree is what enforces DESIGN.md §7's no-network
//! invariant, so every crate added has to earn itself. The standard library already
//! ships a randomly-seeded hasher — `RandomState`, the one `HashMap` uses to resist
//! collision attacks — and mixing the clock, the process id and an attempt counter
//! through a fresh one of those is entropy enough for a name.

use std::collections::hash_map::RandomState;
use std::collections::HashSet;
use std::fmt;
use std::hash::{BuildHasher, Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// Every promise id starts with this.
pub const PREFIX: &str = "pnk_";

/// How many free-id attempts before giving up.
const MAX_ATTEMPTS: u32 = 64;

/// Minting failed: every attempt collided with an id already in the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintError {
    pub attempts: u32,
    pub taken: usize,
}

impl fmt::Display for MintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "could not mint a free promise id in {} attempts ({} ids already in the ledger); \
             the 6-hex-digit id space is crowded — the ledger may need splitting",
            self.attempts, self.taken
        )
    }
}

impl std::error::Error for MintError {}

/// Mint an id that is not already `taken`.
pub fn mint(taken: &HashSet<String>) -> Result<String, MintError> {
    mint_with(taken, candidate)
}

/// [`mint`] with the candidate source injected, so the exhaustion path can be
/// exercised without conjuring 16.7M strings.
fn mint_with(
    taken: &HashSet<String>,
    generate: impl Fn(u32) -> String,
) -> Result<String, MintError> {
    for attempt in 0..MAX_ATTEMPTS {
        let candidate = generate(attempt);
        if !taken.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Err(MintError {
        attempts: MAX_ATTEMPTS,
        taken: taken.len(),
    })
}

/// One candidate id. Public only to the crate so tests can hammer it.
fn candidate(attempt: u32) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    // A fresh RandomState per attempt: its keys are randomly seeded by the standard
    // library, so two calls in the same nanosecond still diverge.
    let mut hasher = RandomState::new().build_hasher();
    nanos.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    attempt.hash(&mut hasher);

    let bits = (hasher.finish() as u32) & 0x00ff_ffff;
    format!("{PREFIX}{bits:06x}")
}

/// Is this an id from the space [`mint`] draws from — `pnk_` plus six lowercase hex
/// digits?
///
/// This is *not* the question `--id` asks. A supplied id is deliberately not required
/// to look like this (see [`check_supplied`]); what this answers is "did pinki mint
/// this", which is the question the reserved prefix above keeps answerable.
pub fn is_well_formed(id: &str) -> bool {
    match id.strip_prefix(PREFIX) {
        Some(hex) => {
            hex.len() == 6
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        None => false,
    }
}

/// The longest id `--id` will accept, in characters.
///
/// A bound rather than a taste: the id is a column in `ls`, the first field of the
/// tab-separated line every mutating verb prints, and the key two ledgers get joined
/// on (§7). 128 clears every id space we have had to carry — a UUID is 36 characters,
/// a timestamped fleet id around 30, a URN comfortably under 100 — while keeping a
/// ledger line something a human can still read with `less`. It is not a claim that
/// 129 would be meaningless; it is a refusal to let the field grow without anyone
/// deciding that it should.
pub const MAX_SUPPLIED_LEN: usize = 128;

/// Why a caller-supplied id was refused.
///
/// Every one of these is malformed *input* — the `--id` as typed — so the CLI turns
/// them all into exit code 2, with nothing appended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuppliedIdError {
    /// Empty, or nothing but whitespace.
    Blank,
    /// Holds a whitespace character somewhere inside it.
    Whitespace,
    /// Holds a control character.
    Control,
    /// Longer than [`MAX_SUPPLIED_LEN`] characters. Carries the length it had.
    TooLong(usize),
    /// Wears the `pnk_` prefix without being an id this module could have minted.
    ReservedPrefix,
}

impl fmt::Display for SuppliedIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SuppliedIdError::Blank => f.write_str(
                "an id has to be something: whitespace is not a value, and a record whose `id` \
                 is blank cannot be shown, resolved, or joined on (§1). Nothing was appended",
            ),
            SuppliedIdError::Whitespace => f.write_str(
                "an id may not contain whitespace: it is a column in `pinki ls`, the first \
                 field of the tab-separated line every mutating verb prints, and the key two \
                 ledgers are joined on (§1, §7). Nothing was appended",
            ),
            SuppliedIdError::Control => f.write_str(
                "an id may not contain control characters: §1's id is a handle meant to be read \
                 and typed, and one that can move a terminal's cursor is not that. Nothing was \
                 appended",
            ),
            SuppliedIdError::TooLong(length) => write!(
                f,
                "an id may be at most {MAX_SUPPLIED_LEN} characters and this one is {length}: \
                 §1's id is a handle, not a payload. Nothing was appended"
            ),
            SuppliedIdError::ReservedPrefix => write!(
                f,
                "`{PREFIX}` is reserved for the ids pinki mints, which are `{PREFIX}` followed \
                 by exactly six lowercase hex digits, e.g. `{PREFIX}4f3a91` (§1). Any other id \
                 is welcome — bring one that does not wear this prefix, or drop `--id` and let \
                 pinki mint one. Nothing was appended"
            ),
        }
    }
}

impl std::error::Error for SuppliedIdError {}

/// Check an id a caller brought with them — §1's id policy, enforced at the edge.
///
/// Deliberately permissive: pinki never parses an id, so the only rules are the ones
/// that keep it usable *as* a handle, plus the reserved prefix. Call it on the
/// trimmed value — leading and trailing whitespace is the caller's shell, not their
/// id.
pub fn check_supplied(id: &str) -> Result<(), SuppliedIdError> {
    if id.is_empty() {
        return Err(SuppliedIdError::Blank);
    }
    for c in id.chars() {
        if c.is_whitespace() {
            return Err(SuppliedIdError::Whitespace);
        }
        if c.is_control() {
            return Err(SuppliedIdError::Control);
        }
    }
    let length = id.chars().count();
    if length > MAX_SUPPLIED_LEN {
        return Err(SuppliedIdError::TooLong(length));
    }
    // The minted namespace stays well-formed, so `is_well_formed` keeps meaning what
    // it says: everything wearing the prefix really was minted here.
    if id.starts_with(PREFIX) && !is_well_formed(id) {
        return Err(SuppliedIdError::ReservedPrefix);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minted_ids_are_well_formed() {
        let empty = HashSet::new();
        for _ in 0..200 {
            let id = mint(&empty).unwrap();
            assert_eq!(id.len(), PREFIX.len() + 6, "{id}");
            assert!(is_well_formed(&id), "{id}");
        }
    }

    #[test]
    fn minting_avoids_ids_already_taken() {
        let mut taken = HashSet::new();
        for _ in 0..50 {
            let id = mint(&taken).unwrap();
            assert!(taken.insert(id.clone()), "minted a duplicate: {id}");
        }
        assert_eq!(taken.len(), 50);
    }

    #[test]
    fn minting_retries_past_a_collision() {
        // A generator that hands back a taken id on the first three attempts, then a
        // free one. The real generator is random, so the retry is only observable
        // with the source injected.
        let mut taken = HashSet::new();
        taken.insert(format!("{PREFIX}aaaaaa"));
        let id = mint_with(&taken, |attempt| {
            if attempt < 3 {
                format!("{PREFIX}aaaaaa")
            } else {
                format!("{PREFIX}bbbbbb")
            }
        })
        .unwrap();
        assert_eq!(id, format!("{PREFIX}bbbbbb"));
    }

    #[test]
    fn minting_gives_up_loudly_when_every_attempt_collides() {
        let mut taken = HashSet::new();
        taken.insert(format!("{PREFIX}aaaaaa"));
        let err = mint_with(&taken, |_| format!("{PREFIX}aaaaaa")).unwrap_err();
        assert_eq!(err.attempts, MAX_ATTEMPTS);
        let msg = err.to_string();
        assert!(msg.contains("64 attempts"), "{msg}");
        assert!(msg.contains("1 ids already in the ledger"), "{msg}");
    }

    #[test]
    fn rejects_malformed_ids() {
        assert!(
            !is_well_formed("pnk_4F3A91"),
            "uppercase hex is not our form"
        );
        assert!(!is_well_formed("pnk_4f3a9"), "five digits");
        assert!(!is_well_formed("pnk_4f3a911"), "seven digits");
        assert!(!is_well_formed("4f3a91"), "no prefix");
        assert!(!is_well_formed("pnk_zzzzzz"), "not hex");
        assert!(is_well_formed("pnk_4f3a91"));
        assert!(is_well_formed("pnk_000000"));
    }

    #[test]
    fn a_supplied_id_may_be_any_shape_a_handle_can_have() {
        // The shapes real id spaces actually arrive in. None of them is a pinki id,
        // and that is the point: an adopter carries their own space in.
        for id in [
            "pr-20260906203134-225e24cd",
            "PR-20260906203134-225E24CD",
            "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
            "JIRA-1234",
            "https://example.org/obligations/91",
            "urn:example:promise:91",
            "約束-91",
            "a",
        ] {
            assert_eq!(check_supplied(id), Ok(()), "{id}");
        }
    }

    #[test]
    fn a_blank_supplied_id_is_refused() {
        // `--id "   "` reaches this trimmed, so the blank case is the empty string.
        assert_eq!(check_supplied(""), Err(SuppliedIdError::Blank));
        let message = SuppliedIdError::Blank.to_string();
        assert!(message.contains("§1"), "{message}");
        assert!(message.contains("Nothing was appended"), "{message}");
    }

    #[test]
    fn a_supplied_id_with_whitespace_or_a_control_char_is_refused() {
        assert_eq!(
            check_supplied("two words"),
            Err(SuppliedIdError::Whitespace)
        );
        assert_eq!(
            check_supplied("tab\there"),
            Err(SuppliedIdError::Whitespace)
        );
        assert_eq!(
            check_supplied("new\nline"),
            Err(SuppliedIdError::Whitespace)
        );
        // DEL is a control character and not whitespace, so it is the other arm.
        assert_eq!(
            check_supplied("bel\u{7}here"),
            Err(SuppliedIdError::Control)
        );
        assert_eq!(check_supplied("del\u{7f}"), Err(SuppliedIdError::Control));
    }

    #[test]
    fn a_supplied_id_is_bounded() {
        let at_the_bound = "x".repeat(MAX_SUPPLIED_LEN);
        assert_eq!(check_supplied(&at_the_bound), Ok(()));

        let over = "x".repeat(MAX_SUPPLIED_LEN + 1);
        assert_eq!(
            check_supplied(&over),
            Err(SuppliedIdError::TooLong(MAX_SUPPLIED_LEN + 1))
        );
        let message = SuppliedIdError::TooLong(MAX_SUPPLIED_LEN + 1).to_string();
        assert!(message.contains("129"), "the length it had: {message}");
        assert!(message.contains("128"), "the bound: {message}");

        // Characters, not bytes: 128 three-byte characters is 384 bytes and legal.
        assert_eq!(check_supplied(&"約".repeat(MAX_SUPPLIED_LEN)), Ok(()));
    }

    #[test]
    fn the_minted_prefix_is_reserved_even_though_everything_else_is_open() {
        // A well-formed minted id is of course acceptable as a supplied one — that is
        // how an id minted in one ledger is carried into another.
        assert_eq!(check_supplied("pnk_4f3a91"), Ok(()));

        for id in [
            "pnk_4F3A91",
            "pnk_4f3a9",
            "pnk_4f3a911",
            "pnk_zzzzzz",
            "pnk_",
        ] {
            assert_eq!(
                check_supplied(id),
                Err(SuppliedIdError::ReservedPrefix),
                "{id}"
            );
        }
        // The reservation is exactly the prefix, and nothing near it.
        assert_eq!(check_supplied("pnk-4f3a91"), Ok(()));
        assert_eq!(check_supplied("pnkish"), Ok(()));
        assert_eq!(check_supplied("PNK_4F3A91"), Ok(()));

        let message = SuppliedIdError::ReservedPrefix.to_string();
        assert!(message.contains("pnk_"), "{message}");
        assert!(message.contains("§1"), "{message}");
    }

    #[test]
    fn distinct_attempts_diverge() {
        let a: HashSet<String> = (0..64).map(candidate).collect();
        // Not a strict guarantee — it is entropy, not a permutation — but 64 draws
        // from 16.7M colliding at all would be a strong signal the mixing is broken.
        assert!(a.len() >= 60, "only {} distinct of 64", a.len());
    }
}
