//! Promise ids: `pnk_` + exactly six lowercase hex characters.
//!
//! Ids are opaque (DESIGN.md §1) — nothing reads structure out of one. Six hex digits
//! is 16.7M values, which is not a lot; the point is a short, typeable handle for a
//! human-scale ledger, and collisions are handled by *asking the ledger* rather than
//! by pretending the space is big enough. Minting takes the set of ids already
//! present and retries until it finds a free one.
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

/// Is this a well-formed pinki id? Used by the verbs that accept one from a human.
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
    fn distinct_attempts_diverge() {
        let a: HashSet<String> = (0..64).map(candidate).collect();
        // Not a strict guarantee — it is entropy, not a permutation — but 64 draws
        // from 16.7M colliding at all would be a strong signal the mixing is broken.
        assert!(a.len() >= 60, "only {} distinct of 64", a.len());
    }
}
