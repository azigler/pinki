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
//! - **Supplied**, when `--id` is given: any non-blank string with no whitespace, no
//!   control characters and no invisible ones ([`DEFAULT_IGNORABLE`]), up to
//!   [`MAX_SUPPLIED_LEN`] characters — see [`check_supplied`]. An adopter arriving
//!   with an id space already referenced from somewhere else carries it in rather
//!   than maintaining a mapping, and §7's ledger join can then key across parties who
//!   did not both mint here.
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

/// Every character with the Unicode `Default_Ignorable_Code_Point` property, as
/// closed ranges.
///
/// Transcribed from `DerivedCoreProperties.txt` of **Unicode 16.0.0** (dated
/// 2024-05-31); the file's 27 rows for this property are coalesced here into the 17
/// ranges they form, 4,174 code points in total. The property is the one Unicode
/// itself defines for "should render as nothing when unsupported" — zero-width
/// spaces and joiners, the bidi controls, variation selectors, the tag characters,
/// soft hyphen, the Hangul fillers — which is exactly the class of character that
/// makes two unequal ids look identical.
///
/// A range table rather than a crate: `Cargo.toml`'s dependency list is DESIGN.md
/// §7's no-network invariant, so every crate has to earn itself, and this one would
/// buy seventeen lines. The cost of the table is that a future Unicode version
/// adding to the property is not picked up until someone updates it — additions are
/// rare (the set has been these same 4,174 code points since Unicode 14.0, whose one
/// addition to it was U+180F), and the failure mode is the old behavior for a new
/// code point, not a wrong answer for an existing one.
const DEFAULT_IGNORABLE: [(char, char); 17] = [
    ('\u{00AD}', '\u{00AD}'),   // SOFT HYPHEN
    ('\u{034F}', '\u{034F}'),   // COMBINING GRAPHEME JOINER
    ('\u{061C}', '\u{061C}'),   // ARABIC LETTER MARK
    ('\u{115F}', '\u{1160}'),   // HANGUL CHOSEONG/JUNGSEONG FILLER
    ('\u{17B4}', '\u{17B5}'),   // KHMER VOWEL INHERENT AQ..AA
    ('\u{180B}', '\u{180F}'),   // MONGOLIAN FREE VARIATION SELECTORS, VOWEL SEPARATOR
    ('\u{200B}', '\u{200F}'),   // ZERO WIDTH SPACE..RIGHT-TO-LEFT MARK
    ('\u{202A}', '\u{202E}'),   // the bidi embedding/override controls
    ('\u{2060}', '\u{206F}'),   // WORD JOINER..NOMINAL DIGIT SHAPES
    ('\u{3164}', '\u{3164}'),   // HANGUL FILLER
    ('\u{FE00}', '\u{FE0F}'),   // VARIATION SELECTOR-1..16
    ('\u{FEFF}', '\u{FEFF}'),   // ZERO WIDTH NO-BREAK SPACE (the BOM)
    ('\u{FFA0}', '\u{FFA0}'),   // HALFWIDTH HANGUL FILLER
    ('\u{FFF0}', '\u{FFF8}'),   // reserved, property-assigned
    ('\u{1BCA0}', '\u{1BCA3}'), // SHORTHAND FORMAT controls
    ('\u{1D173}', '\u{1D17A}'), // MUSICAL SYMBOL BEGIN BEAM..END PHRASE
    ('\u{E0000}', '\u{E0FFF}'), // the tag characters and variation selectors 17..256
];

/// Does this character have the Unicode `Default_Ignorable_Code_Point` property?
fn is_default_ignorable(c: char) -> bool {
    // Nothing below U+00AD is in the table, which is every id anybody actually has.
    c >= DEFAULT_IGNORABLE[0].0
        && DEFAULT_IGNORABLE
            .iter()
            .any(|&(low, high)| (low..=high).contains(&c))
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
    /// Holds a character with the Unicode `Default_Ignorable_Code_Point` property —
    /// one that renders as nothing. Carries the character, since naming it is the
    /// only way the caller can see it.
    Ignorable(char),
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
            SuppliedIdError::Ignorable(c) => write!(
                f,
                "an id may not contain U+{:04X}, a character with the Unicode \
                 Default_Ignorable_Code_Point property: it renders as nothing, so two ids \
                 that are not equal can look identical in `pinki ls`, in a terminal, and to \
                 whoever is reading the ledger — and §1's id is a handle those readings are \
                 of, and §7's join key. Nothing was appended",
                *c as u32
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
///
/// "Usable as a handle" includes being *visible*: an id carrying a
/// [`DEFAULT_IGNORABLE`] character is refused, because two ids that are not equal
/// must not be able to look equal.
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
        // Not covered by either check above: `char::is_whitespace` is the White_Space
        // property (U+200B is not one, despite its name) and `char::is_control` is the
        // Cc category, while these are mostly Cf. An invisible character in a key is
        // its own rule because it defeats the eye rather than the terminal.
        if is_default_ignorable(c) {
            return Err(SuppliedIdError::Ignorable(c));
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
    fn a_supplied_id_may_not_hold_an_invisible_character() {
        // A spread across the property, and — asserted first, because it is the whole
        // reason this rule exists — none of them is caught by either check above:
        // `is_whitespace()` is the White_Space property (U+200B is not one, despite
        // the name) and `is_control()` is the Cc category.
        for (id, c) in [
            ("pr-9\u{200B}1", '\u{200B}'),   // ZERO WIDTH SPACE
            ("pr-9\u{200C}1", '\u{200C}'),   // ZERO WIDTH NON-JOINER
            ("pr-9\u{200D}1", '\u{200D}'),   // ZERO WIDTH JOINER
            ("pr-9\u{FEFF}1", '\u{FEFF}'),   // ZERO WIDTH NO-BREAK SPACE (BOM)
            ("pr-9\u{2060}1", '\u{2060}'),   // WORD JOINER
            ("pr-9\u{00AD}1", '\u{00AD}'),   // SOFT HYPHEN
            ("pr-9\u{202E}1", '\u{202E}'),   // RIGHT-TO-LEFT OVERRIDE
            ("pr-9\u{FE0F}1", '\u{FE0F}'),   // VARIATION SELECTOR-16
            ("pr-9\u{E0001}1", '\u{E0001}'), // LANGUAGE TAG
        ] {
            assert!(!c.is_whitespace() && !c.is_control(), "U+{:04X}", c as u32);
            assert_eq!(
                check_supplied(id),
                Err(SuppliedIdError::Ignorable(c)),
                "{c:?}"
            );
        }

        // The message names the code point — the only way to see a character that
        // renders as nothing — and the property, and cites §1.
        let message = SuppliedIdError::Ignorable('\u{200B}').to_string();
        assert!(message.contains("U+200B"), "{message}");
        assert!(
            message.contains("Default_Ignorable_Code_Point"),
            "{message}"
        );
        assert!(message.contains("§1"), "{message}");
        assert!(message.contains("Nothing was appended"), "{message}");

        // Not a ban on non-ASCII: a script is not an invisible character.
        for id in ["υπόσχεση-91", "約束-91", "Ω", "ñ-1", "pr-٩١"] {
            assert_eq!(check_supplied(id), Ok(()), "{id}");
        }
    }

    #[test]
    fn the_ignorable_table_is_the_property_it_claims_to_be() {
        // A transcribed table earns one structural check, because a typo in it is
        // silent: sorted, non-overlapping, non-adjacent (adjacent ranges would mean
        // the coalescing was not carried through), and the total the UCD gives.
        let mut total = 0usize;
        for (i, &(low, high)) in DEFAULT_IGNORABLE.iter().enumerate() {
            assert!(low <= high, "range {i} is inverted");
            if i > 0 {
                let previous = DEFAULT_IGNORABLE[i - 1].1;
                assert!(
                    low as u32 > previous as u32 + 1,
                    "range {i} is not disjoint from, and clear of, the one before it"
                );
            }
            total += (high as u32 - low as u32 + 1) as usize;
            assert!(is_default_ignorable(low) && is_default_ignorable(high));
        }
        assert_eq!(total, 4174, "Unicode 16.0.0's Default_Ignorable_Code_Point");

        // And the boundaries hold on both sides of the first and last ranges.
        assert!(!is_default_ignorable('\u{00AC}'));
        assert!(!is_default_ignorable('\u{00AE}'));
        assert!(!is_default_ignorable('\u{E1000}'));
        assert!(!is_default_ignorable('a'));
        // U+2028/2029 are separators, not ignorables — `is_whitespace` has them.
        assert!(!is_default_ignorable('\u{2028}'));
        assert!('\u{2028}'.is_whitespace());
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
