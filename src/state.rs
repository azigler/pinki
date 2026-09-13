//! The fold — DESIGN.md §3.
//!
//! State is computed from the log, never stored. Given the same events and the same
//! `now`, any two implementations must produce the same answer; that is the whole
//! claim of §3's "computed states" column, and it is why `now` is a **parameter**
//! here rather than something this module reads off the clock. An injected `now` is
//! also the only way the overdue boundary is testable to the second.
//!
//! Asserted state — `violated` — is deliberately absent from [`State`]. §3: pinki
//! computes the first column and *publishes, never computes*, the second. Assessments
//! are collected onto each promise ([`PromiseView::assessments`]) and have no effect
//! whatsoever on the computed answer.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::str::FromStr;

use jiff::Timestamp;

use crate::event::{Event, EventBody, Resolution};
use crate::record::Promise;

/// A computed state. §3's first table, and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    /// `on` is set and the antecedent is not yet satisfied.
    Conditional,
    /// Owed now. Either no `on`, or the antecedent is satisfied.
    Detached,
    /// `detached`, unresolved, and `now > until`.
    Overdue,
    /// Resolved by the debtor, with evidence.
    Satisfied,
    /// Resolved by the debtor, with a reason.
    Cancelled,
    /// Resolved by the creditor letting the debtor off.
    Released,
    /// The antecedent ended without being satisfied — never became owed.
    Expired,
}

impl State {
    /// The state's name, as `ls --json` and `show` print it.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Conditional => "conditional",
            State::Detached => "detached",
            State::Overdue => "overdue",
            State::Satisfied => "satisfied",
            State::Cancelled => "cancelled",
            State::Released => "released",
            State::Expired => "expired",
        }
    }

    /// Terminal states are the ones the promise cannot leave: it ended, one way or
    /// another. §5's `ls --all` shows these; `--open` excludes them.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            State::Satisfied | State::Cancelled | State::Released | State::Expired
        )
    }

    /// The complement of [`State::is_terminal`] — `conditional`, `detached`,
    /// `overdue`. This is §5's `--open`.
    pub fn is_open(self) -> bool {
        !self.is_terminal()
    }
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Everything the fold knows about one promise.
#[derive(Debug, Clone)]
pub struct PromiseView<'a> {
    /// The record, from its `promise` event.
    pub record: &'a Promise,
    /// The computed state at the `now` the fold was given.
    pub state: State,
    /// The `resolve` event that ended it, if any. The **first** one in the log —
    /// see [`fold`] on why later ones are ignored rather than rejected.
    pub resolution: Option<&'a Event>,
    /// Every `assess` event naming this promise, in log order. Never terminal, never
    /// consulted by the state computation — §4: "an assessed promise stays exactly as
    /// open as it was." Two contradicting assessments both appear here, which is §3's
    /// point, not a bug.
    pub assessments: Vec<&'a Event>,
}

impl<'a> PromiseView<'a> {
    /// The promise id.
    pub fn id(&self) -> &'a str {
        &self.record.id
    }

    /// How it ended, if it did.
    pub fn resolution_kind(&self) -> Option<&'a Resolution> {
        self.resolution.and_then(|event| event.resolution())
    }
}

/// The result of folding a ledger: every promise it declared, with its state.
#[derive(Debug, Clone, Default)]
pub struct Fold<'a> {
    order: Vec<&'a str>,
    views: HashMap<&'a str, PromiseView<'a>>,
}

impl<'a> Fold<'a> {
    /// One promise by id, or `None` if the ledger never declared it.
    pub fn get(&self, id: &str) -> Option<&PromiseView<'a>> {
        self.views.get(id)
    }

    /// Every promise, in the order the log first declared it. Stable ordering matters
    /// for `ls`: the ledger is the record, so its order is the natural one.
    pub fn iter(&self) -> impl Iterator<Item = &PromiseView<'a>> {
        self.order.iter().filter_map(|id| self.views.get(id))
    }

    /// How many promises the ledger declared.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Did the ledger declare no promises at all?
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// The state of one promise, or `None` if it was never declared.
    pub fn state(&self, id: &str) -> Option<State> {
        self.get(id).map(|view| view.state)
    }
}

/// Fold a ledger into states, as of `now`.
///
/// `now` is injected, never read from the clock: §6 — pinki "has no clock of its own
/// — it reads `until` and compares it to `now` when you ask."
///
/// Three tolerances, all of them because §4 says two ledgers may be concatenated into
/// a joined view and §7 makes that join the whole point of the format:
///
/// - a **second `promise` event** for an id already declared is ignored (the first
///   declaration stands);
/// - a **second `resolve` event** for a promise is ignored (the first resolution
///   stands) — a joined or hand-edited ledger can legitimately hold two, and picking
///   the first is the only choice that does not depend on which side of a join you
///   were standing on;
/// - a **dangling** `resolve` or `assess`, naming an id with no `promise` event, is
///   ignored rather than treated as an error. A joined ledger routinely carries half
///   a conversation.
pub fn fold<'a>(events: &'a [Event], now: Timestamp) -> Fold<'a> {
    let mut order: Vec<&'a str> = Vec::new();
    let mut records: HashMap<&'a str, &'a Promise> = HashMap::new();
    let mut resolutions: HashMap<&'a str, &'a Event> = HashMap::new();
    let mut assessments: HashMap<&'a str, Vec<&'a Event>> = HashMap::new();

    for event in events {
        match &event.body {
            EventBody::Promise(record) => {
                // First declaration wins: `insert` would overwrite, which would let a
                // re-declaration silently move a deadline.
                if let Entry::Vacant(slot) = records.entry(record.id.as_str()) {
                    slot.insert(record);
                    order.push(record.id.as_str());
                }
            }
            EventBody::Resolve { promise, .. } => {
                // First resolve wins.
                resolutions.entry(promise.as_str()).or_insert(event);
            }
            EventBody::Assess { promise, .. } => {
                assessments.entry(promise.as_str()).or_default().push(event);
            }
        }
    }

    let mut solver = Solver {
        now,
        records: &records,
        resolutions: &resolutions,
        memo: HashMap::new(),
    };
    for id in &order {
        solver.solve(id);
    }
    let memo = solver.memo;

    let views = records
        .iter()
        .map(|(&id, &record)| {
            let view = PromiseView {
                record,
                state: match memo.get(id) {
                    Some(Memo::Done(state)) => *state,
                    // Unreachable: every declared id was solved above, and solve()
                    // always terminates with a Done. Conditional is the conservative
                    // answer if that ever stopped being true.
                    _ => State::Conditional,
                },
                resolution: resolutions.get(id).copied(),
                assessments: assessments.get(id).cloned().unwrap_or_default(),
            };
            (id, view)
        })
        .collect();

    Fold { order, views }
}

#[derive(Debug, Clone, Copy)]
enum Memo {
    /// On the current descent — reaching this again means `on` forms a cycle.
    InProgress,
    Done(State),
}

struct Solver<'a, 'm> {
    now: Timestamp,
    records: &'m HashMap<&'a str, &'a Promise>,
    resolutions: &'m HashMap<&'a str, &'a Event>,
    memo: HashMap<&'a str, Memo>,
}

impl<'a> Solver<'a, '_> {
    /// Resolve one promise's state, descending through `on` edges.
    ///
    /// Iterative with an explicit stack rather than recursive: `on` is caller-supplied
    /// data, so neither its depth nor its acyclicity is ours to assume, and a deep
    /// chain must not blow the stack any more than a cycle may hang.
    fn solve(&mut self, start: &'a str) {
        let mut stack: Vec<&'a str> = vec![start];

        while let Some(&current) = stack.last() {
            match self.memo.get(current) {
                Some(Memo::Done(_)) => {
                    stack.pop();
                    continue;
                }
                // Revisited after descending into the antecedent; fall through and
                // decide now that the child is known.
                Some(Memo::InProgress) => {}
                None => {
                    self.memo.insert(current, Memo::InProgress);
                }
            }

            let Some(record) = self.records.get(current) else {
                // Dangling: nothing declared this promise. Nothing to state.
                self.memo.remove(current);
                stack.pop();
                continue;
            };

            // Rule 1: a resolution ends the promise, whatever the antecedent says.
            if let Some(event) = self.resolutions.get(current) {
                let state = match event.resolution() {
                    Some(Resolution::Satisfied { .. }) => State::Satisfied,
                    Some(Resolution::Cancelled { .. }) => State::Cancelled,
                    Some(Resolution::Released { .. }) => State::Released,
                    // Only `resolve` events land in `resolutions`, so this is
                    // unreachable; leaving it open is the honest fallback.
                    None => State::Detached,
                };
                self.memo.insert(current, Memo::Done(state));
                stack.pop();
                continue;
            }

            // Rule 3: no antecedent means born owed.
            let Some(antecedent) = record.on.as_deref() else {
                let state = self.owed_state(record);
                self.memo.insert(current, Memo::Done(state));
                stack.pop();
                continue;
            };

            // Rule 2, the unknown-antecedent case: §3's table reads "`on` is set and
            // the antecedent is not yet satisfied", and an id this ledger has never
            // heard of has certainly not satisfied. Conditional is also the safe
            // reading — it keeps the promise open and visible rather than quietly
            // filing it as expired on the strength of a missing line.
            if !self.records.contains_key(antecedent) {
                self.memo.insert(current, Memo::Done(State::Conditional));
                stack.pop();
                continue;
            }

            match self.memo.get(antecedent) {
                None => {
                    stack.push(antecedent);
                }
                // A cycle. DESIGN.md does not cover this — see the module tests — but
                // the rule above answers it: an antecedent that can never satisfy has
                // not satisfied, so the promise stays conditional. It is never owed
                // and it never expires, which is exactly what a promise waiting on
                // something impossible deserves.
                Some(Memo::InProgress) => {
                    self.memo.insert(current, Memo::Done(State::Conditional));
                    stack.pop();
                }
                Some(Memo::Done(antecedent_state)) => {
                    let state = match antecedent_state {
                        // Detached: the antecedent satisfied, so this is owed now —
                        // and only now can it be late (rule 4).
                        State::Satisfied => self.owed_state(record),
                        // The antecedent ended without satisfying, so this never
                        // became owed. Note this propagates: a promise on an expired
                        // antecedent is itself expired, transitively down the chain.
                        State::Cancelled | State::Released | State::Expired => State::Expired,
                        // Still open — including `overdue`, which is late but *not*
                        // terminal. A dependent does not expire because its
                        // antecedent is running late; it expires only when the
                        // antecedent actually ends unsatisfied.
                        State::Conditional | State::Detached | State::Overdue => State::Conditional,
                    };
                    self.memo.insert(current, Memo::Done(state));
                    stack.pop();
                }
            }
        }
    }

    /// Rule 4. A promise that is owed and unresolved is `detached` until `now` passes
    /// `until`, at which point it is `overdue`.
    ///
    /// Two things this function encodes on purpose:
    ///
    /// - **Only owed promises can be late.** A `conditional` promise whose `until` has
    ///   passed stays `conditional` — it was never owed, so it cannot be late. That is
    ///   why this is reachable only from the two branches where the promise is
    ///   detached, and never from the conditional ones.
    /// - **The comparison is strictly greater.** §3: `overdue` is "`now > until`". At
    ///   exactly `until` the promise is still due, not yet late; a deadline is a
    ///   moment you may use, not one you have already missed.
    fn owed_state(&self, record: &Promise) -> State {
        match Timestamp::from_str(&record.until) {
            Ok(until) if self.now > until => State::Overdue,
            Ok(_) => State::Detached,
            // An `until` this build cannot parse can only arrive from a hand-edited
            // or foreign ledger — the `promise` verb parses it on the way in. There
            // is no honest comparison to make against a deadline we cannot read, and
            // inventing an ordering would be worse than declining to: the promise
            // stays owed and open, and nothing is asserted about its lateness.
            Err(_) => State::Detached,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventBody;

    fn ts(s: &str) -> Timestamp {
        Timestamp::from_str(s).unwrap_or_else(|e| panic!("bad test timestamp {s}: {e}"))
    }

    const UNTIL: &str = "2026-09-01T17:00:00Z";
    const BEFORE: &str = "2026-09-01T16:59:59Z";
    const EXACTLY: &str = "2026-09-01T17:00:00Z";
    const AFTER: &str = "2026-09-01T17:00:01Z";

    /// A promise event: `id`, optional antecedent, deadline.
    fn promise(id: &str, on: Option<&str>, until: &str) -> Event {
        Event::new(
            "2026-08-28T20:14:03Z",
            EventBody::promise(Promise {
                id: id.into(),
                promise: format!("do the {id} thing"),
                by: "…/debtor".into(),
                to: "…/creditor".into(),
                on: on.map(str::to_string),
                until: until.into(),
                task: None,
                meta: None,
            }),
        )
    }

    fn satisfied(id: &str) -> Event {
        Event::new(
            "2026-08-29T00:00:00Z",
            EventBody::satisfied(id, "…/debtor", vec!["https://example.org/x".into()]),
        )
    }

    fn cancelled(id: &str) -> Event {
        Event::new(
            "2026-08-29T00:00:00Z",
            EventBody::cancelled(id, "…/debtor", "upstream withdrew"),
        )
    }

    fn released(id: &str) -> Event {
        Event::new(
            "2026-08-29T00:00:00Z",
            EventBody::released(id, "…/creditor", None),
        )
    }

    fn assessed(id: &str) -> Event {
        Event::new(
            "2026-09-02T09:00:00Z",
            EventBody::violated(id, "…/author", Some("nothing shipped".into())),
        )
    }

    /// State of `id` after folding `events` at `now`.
    fn state_of(events: &[Event], now: &str, id: &str) -> State {
        fold(events, ts(now))
            .state(id)
            .unwrap_or_else(|| panic!("{id} was not in the fold"))
    }

    // ---- the ground cases ------------------------------------------------

    #[test]
    fn born_owed_promise_is_detached() {
        let log = [promise("pnk_a", None, UNTIL)];
        assert_eq!(state_of(&log, BEFORE, "pnk_a"), State::Detached);
    }

    #[test]
    fn conditional_while_antecedent_is_unresolved() {
        let log = [
            promise("pnk_a", None, UNTIL),
            promise("pnk_b", Some("pnk_a"), UNTIL),
        ];
        assert_eq!(state_of(&log, BEFORE, "pnk_a"), State::Detached);
        assert_eq!(state_of(&log, BEFORE, "pnk_b"), State::Conditional);
    }

    #[test]
    fn detaches_when_the_antecedent_satisfies() {
        let log = [
            promise("pnk_a", None, UNTIL),
            promise("pnk_b", Some("pnk_a"), UNTIL),
            satisfied("pnk_a"),
        ];
        assert_eq!(state_of(&log, BEFORE, "pnk_a"), State::Satisfied);
        assert_eq!(state_of(&log, BEFORE, "pnk_b"), State::Detached);
    }

    #[test]
    fn declaration_order_does_not_matter() {
        // The dependent is declared before its antecedent — a join can produce this.
        let log = [
            promise("pnk_b", Some("pnk_a"), UNTIL),
            promise("pnk_a", None, UNTIL),
            satisfied("pnk_a"),
        ];
        assert_eq!(state_of(&log, BEFORE, "pnk_b"), State::Detached);
    }

    // ---- expiry ----------------------------------------------------------

    #[test]
    fn expired_when_the_antecedent_is_cancelled() {
        let log = [
            promise("pnk_a", None, UNTIL),
            promise("pnk_b", Some("pnk_a"), UNTIL),
            cancelled("pnk_a"),
        ];
        assert_eq!(state_of(&log, BEFORE, "pnk_b"), State::Expired);
    }

    #[test]
    fn expired_when_the_antecedent_is_released() {
        let log = [
            promise("pnk_a", None, UNTIL),
            promise("pnk_b", Some("pnk_a"), UNTIL),
            released("pnk_a"),
        ];
        assert_eq!(state_of(&log, BEFORE, "pnk_b"), State::Expired);
    }

    #[test]
    fn expiry_propagates_transitively() {
        let log = [
            promise("pnk_a", None, UNTIL),
            promise("pnk_b", Some("pnk_a"), UNTIL),
            promise("pnk_c", Some("pnk_b"), UNTIL),
            promise("pnk_d", Some("pnk_c"), UNTIL),
            cancelled("pnk_a"),
        ];
        assert_eq!(state_of(&log, BEFORE, "pnk_b"), State::Expired);
        assert_eq!(state_of(&log, BEFORE, "pnk_c"), State::Expired);
        assert_eq!(state_of(&log, BEFORE, "pnk_d"), State::Expired);
    }

    // ---- the overdue boundary -------------------------------------------

    #[test]
    fn detached_before_the_deadline() {
        let log = [promise("pnk_a", None, UNTIL)];
        assert_eq!(state_of(&log, BEFORE, "pnk_a"), State::Detached);
    }

    #[test]
    fn exactly_at_the_deadline_is_not_yet_overdue() {
        let log = [promise("pnk_a", None, UNTIL)];
        assert_eq!(state_of(&log, EXACTLY, "pnk_a"), State::Detached);
    }

    #[test]
    fn one_second_past_the_deadline_is_overdue() {
        let log = [promise("pnk_a", None, UNTIL)];
        assert_eq!(state_of(&log, AFTER, "pnk_a"), State::Overdue);
    }

    #[test]
    fn a_detached_dependent_can_go_overdue_too() {
        let log = [
            promise("pnk_a", None, UNTIL),
            promise("pnk_b", Some("pnk_a"), UNTIL),
            satisfied("pnk_a"),
        ];
        assert_eq!(state_of(&log, AFTER, "pnk_b"), State::Overdue);
    }

    #[test]
    fn conditional_past_its_deadline_stays_conditional() {
        // It was never owed, so it cannot be late.
        let log = [
            promise("pnk_a", None, "2026-12-31T00:00:00Z"),
            promise("pnk_b", Some("pnk_a"), UNTIL),
        ];
        assert_eq!(state_of(&log, AFTER, "pnk_b"), State::Conditional);
    }

    #[test]
    fn a_promise_on_an_overdue_antecedent_stays_conditional() {
        // `overdue` is late, not ended: the dependent does not expire yet.
        let log = [
            promise("pnk_a", None, UNTIL),
            promise("pnk_b", Some("pnk_a"), "2026-12-31T00:00:00Z"),
        ];
        assert_eq!(state_of(&log, AFTER, "pnk_a"), State::Overdue);
        assert_eq!(state_of(&log, AFTER, "pnk_b"), State::Conditional);
    }

    #[test]
    fn offset_and_minute_precision_deadlines_compare_correctly() {
        // Same instant, three spellings §5's CLI accepts.
        for spelling in [
            "2026-09-01T17:00:00Z",
            "2026-09-01T17:00Z",
            "2026-09-01T10:00:00-07:00",
        ] {
            let log = [promise("pnk_a", None, spelling)];
            assert_eq!(
                state_of(&log, EXACTLY, "pnk_a"),
                State::Detached,
                "{spelling} at the boundary"
            );
            assert_eq!(
                state_of(&log, AFTER, "pnk_a"),
                State::Overdue,
                "{spelling} past the boundary"
            );
        }
    }

    #[test]
    fn an_unreadable_deadline_leaves_the_promise_owed_but_never_late() {
        let log = [promise("pnk_a", None, "sometime next week")];
        assert_eq!(state_of(&log, AFTER, "pnk_a"), State::Detached);
        assert_eq!(
            state_of(&log, "2099-01-01T00:00:00Z", "pnk_a"),
            State::Detached
        );
    }

    // ---- terminal states -------------------------------------------------

    #[test]
    fn satisfied_cancelled_and_released_are_terminal() {
        for (event, expected) in [
            (satisfied("pnk_a"), State::Satisfied),
            (cancelled("pnk_a"), State::Cancelled),
            (released("pnk_a"), State::Released),
        ] {
            let log = [promise("pnk_a", None, UNTIL), event];
            let state = state_of(&log, BEFORE, "pnk_a");
            assert_eq!(state, expected);
            assert!(state.is_terminal(), "{state} should be terminal");
            assert!(!state.is_open());
        }
    }

    #[test]
    fn open_states_are_exactly_conditional_detached_overdue() {
        for state in [State::Conditional, State::Detached, State::Overdue] {
            assert!(state.is_open(), "{state}");
            assert!(!state.is_terminal(), "{state}");
        }
        for state in [
            State::Satisfied,
            State::Cancelled,
            State::Released,
            State::Expired,
        ] {
            assert!(state.is_terminal(), "{state}");
        }
    }

    #[test]
    fn state_names_are_the_design_vocabulary() {
        assert_eq!(State::Conditional.as_str(), "conditional");
        assert_eq!(State::Detached.as_str(), "detached");
        assert_eq!(State::Overdue.as_str(), "overdue");
        assert_eq!(State::Satisfied.as_str(), "satisfied");
        assert_eq!(State::Cancelled.as_str(), "cancelled");
        assert_eq!(State::Released.as_str(), "released");
        assert_eq!(State::Expired.as_str(), "expired");
    }

    #[test]
    fn a_state_formats_as_the_word_it_prints() {
        // `Display` and `as_str` are the same vocabulary; a state interpolated into a
        // message must not read as its Rust variant name.
        assert_eq!(format!("{}", State::Overdue), "overdue");
        assert_eq!(format!("{}", State::Conditional), "conditional");
        assert_eq!(format!("{}", State::Expired), "expired");
    }

    #[test]
    fn resolving_after_the_deadline_still_yields_satisfied() {
        // Late, but done. `overdue` was a fact about an unresolved promise; the
        // resolve ends it, and whether the lateness mattered is an `assess`, not
        // arithmetic.
        let log = [promise("pnk_a", None, UNTIL), satisfied("pnk_a")];
        assert_eq!(state_of(&log, AFTER, "pnk_a"), State::Satisfied);
    }

    // ---- duplicate resolves ----------------------------------------------

    #[test]
    fn first_resolve_wins_satisfied_then_cancelled() {
        let log = [
            promise("pnk_a", None, UNTIL),
            satisfied("pnk_a"),
            cancelled("pnk_a"),
        ];
        assert_eq!(state_of(&log, BEFORE, "pnk_a"), State::Satisfied);
    }

    #[test]
    fn first_resolve_wins_cancelled_then_satisfied() {
        let log = [
            promise("pnk_a", None, UNTIL),
            cancelled("pnk_a"),
            satisfied("pnk_a"),
        ];
        assert_eq!(state_of(&log, BEFORE, "pnk_a"), State::Cancelled);
    }

    #[test]
    fn the_view_reports_the_winning_resolution() {
        let log = [
            promise("pnk_a", None, UNTIL),
            released("pnk_a"),
            satisfied("pnk_a"),
        ];
        let folded = fold(&log, ts(BEFORE));
        let view = folded.get("pnk_a").unwrap();
        assert_eq!(view.state, State::Released);
        assert_eq!(view.resolution_kind().unwrap().as_str(), "released");
    }

    #[test]
    fn a_duplicate_promise_declaration_does_not_duplicate_the_promise() {
        let log = [
            promise("pnk_a", None, UNTIL),
            promise("pnk_a", None, "2099-01-01T00:00:00Z"),
        ];
        let folded = fold(&log, ts(BEFORE));
        assert_eq!(folded.len(), 1);
        // The first declaration stands.
        assert_eq!(folded.get("pnk_a").unwrap().record.until, UNTIL);
    }

    // ---- unknown antecedents and cycles ----------------------------------

    #[test]
    fn an_unknown_antecedent_leaves_the_promise_conditional() {
        let log = [promise("pnk_b", Some("pnk_missing"), UNTIL)];
        assert_eq!(state_of(&log, AFTER, "pnk_b"), State::Conditional);
    }

    #[test]
    fn a_self_cycle_is_conditional_and_terminates() {
        let log = [promise("pnk_a", Some("pnk_a"), UNTIL)];
        assert_eq!(state_of(&log, AFTER, "pnk_a"), State::Conditional);
    }

    #[test]
    fn a_two_node_cycle_is_conditional_and_terminates() {
        let log = [
            promise("pnk_a", Some("pnk_b"), UNTIL),
            promise("pnk_b", Some("pnk_a"), UNTIL),
        ];
        assert_eq!(state_of(&log, AFTER, "pnk_a"), State::Conditional);
        assert_eq!(state_of(&log, AFTER, "pnk_b"), State::Conditional);
    }

    #[test]
    fn a_three_node_cycle_with_a_dependent_outside_it() {
        let log = [
            promise("pnk_a", Some("pnk_c"), UNTIL),
            promise("pnk_b", Some("pnk_a"), UNTIL),
            promise("pnk_c", Some("pnk_b"), UNTIL),
            promise("pnk_d", Some("pnk_c"), UNTIL),
        ];
        let folded = fold(&log, ts(AFTER));
        for id in ["pnk_a", "pnk_b", "pnk_c", "pnk_d"] {
            assert_eq!(folded.state(id), Some(State::Conditional), "{id}");
        }
    }

    #[test]
    fn a_long_chain_does_not_blow_the_stack() {
        // The `on` edge is caller data; depth is not ours to assume.
        let mut log = vec![promise("pnk_000000", None, UNTIL)];
        for i in 1..20_000u32 {
            log.push(promise(
                &format!("pnk_{i:06x}"),
                Some(&format!("pnk_{:06x}", i - 1)),
                UNTIL,
            ));
        }
        log.push(satisfied("pnk_000000"));
        let folded = fold(&log, ts(BEFORE));
        assert_eq!(folded.state("pnk_000001"), Some(State::Detached));
        assert_eq!(folded.state("pnk_004e1f"), Some(State::Conditional));
    }

    // ---- dangling events and assessments ---------------------------------

    #[test]
    fn dangling_resolve_and_assess_events_do_not_panic() {
        let log = [
            promise("pnk_a", None, UNTIL),
            satisfied("pnk_ghost"),
            cancelled("pnk_ghost2"),
            assessed("pnk_ghost3"),
        ];
        let folded = fold(&log, ts(BEFORE));
        assert_eq!(folded.len(), 1);
        assert_eq!(folded.state("pnk_a"), Some(State::Detached));
        assert!(folded.get("pnk_ghost").is_none());
        assert!(folded.get("pnk_ghost3").is_none());
    }

    #[test]
    fn assessments_are_collected_but_change_nothing() {
        let log = [
            promise("pnk_a", None, UNTIL),
            assessed("pnk_a"),
            assessed("pnk_a"),
        ];
        let folded = fold(&log, ts(AFTER));
        let view = folded.get("pnk_a").unwrap();
        // Two assessments, both visible — §3: pinki shows you both.
        assert_eq!(view.assessments.len(), 2);
        // And the computed state is exactly what it would have been without them.
        assert_eq!(view.state, State::Overdue);
        assert_eq!(
            fold(&[promise("pnk_a", None, UNTIL)], ts(AFTER)).state("pnk_a"),
            Some(State::Overdue)
        );
    }

    #[test]
    fn an_assessment_does_not_make_a_promise_terminal() {
        let log = [promise("pnk_a", None, UNTIL), assessed("pnk_a")];
        let state = state_of(&log, AFTER, "pnk_a");
        assert!(
            state.is_open(),
            "an assessed promise stays open, got {state}"
        );
    }

    // ---- the fold's shape ------------------------------------------------

    #[test]
    fn an_empty_ledger_folds_to_nothing() {
        let folded = fold(&[], ts(BEFORE));
        assert!(folded.is_empty());
        assert_eq!(folded.len(), 0);
        assert_eq!(folded.iter().count(), 0);
        assert!(folded.get("pnk_a").is_none());
    }

    #[test]
    fn iteration_follows_declaration_order() {
        let log = [
            promise("pnk_c", None, UNTIL),
            promise("pnk_a", None, UNTIL),
            promise("pnk_b", None, UNTIL),
        ];
        let folded = fold(&log, ts(BEFORE));
        let ids: Vec<&str> = folded.iter().map(|view| view.id()).collect();
        assert_eq!(ids, ["pnk_c", "pnk_a", "pnk_b"]);
    }

    #[test]
    fn the_full_design_section_2_graph() {
        // author → reviewer → publisher, exactly the chain in §2.
        let log = [
            promise("pnk_0c2b77", None, "2026-08-30T17:00:00Z"),
            promise("pnk_4f3a91", Some("pnk_0c2b77"), "2026-09-01T17:00:00Z"),
            promise("pnk_88de10", Some("pnk_4f3a91"), "2026-09-03T17:00:00Z"),
        ];
        // Nothing resolved, mid-window: the head is owed, the tail waits.
        let folded = fold(&log, ts("2026-08-29T00:00:00Z"));
        assert_eq!(folded.state("pnk_0c2b77"), Some(State::Detached));
        assert_eq!(folded.state("pnk_4f3a91"), Some(State::Conditional));
        assert_eq!(folded.state("pnk_88de10"), Some(State::Conditional));

        // The head satisfies; the middle detaches and is already late.
        let mut log = log.to_vec();
        log.push(satisfied("pnk_0c2b77"));
        let folded = fold(&log, ts("2026-09-02T00:00:00Z"));
        assert_eq!(folded.state("pnk_0c2b77"), Some(State::Satisfied));
        assert_eq!(folded.state("pnk_4f3a91"), Some(State::Overdue));
        assert_eq!(folded.state("pnk_88de10"), Some(State::Conditional));

        // The middle is released instead; the tail never becomes owed.
        log.push(released("pnk_4f3a91"));
        let folded = fold(&log, ts("2026-09-04T00:00:00Z"));
        assert_eq!(folded.state("pnk_4f3a91"), Some(State::Released));
        assert_eq!(folded.state("pnk_88de10"), Some(State::Expired));
    }
}
