//! Integration tests: the real binary, a real ledger file, real exit codes.
//!
//! These drive `CARGO_BIN_EXE_pinki` with [`std::process::Command`] rather than a
//! test-harness crate. That is the same rule Cargo.toml states for the dependency
//! list — the no-network invariant of DESIGN.md §7 is enforced by what this binary is
//! linked against, so the tree stays short enough to read, dev-dependencies included.
//!
//! Every test gets its own ledger under a uniquely named temp directory and passes it
//! through `PINKI_LEDGER` on the child. Tests run in parallel; a shared path would
//! race, and the resulting failures would be intermittent and blamed on the code.
//!
//! Nothing here needs a clock seam. Time-dependent assertions use deadlines that are
//! unambiguous from any plausible `now` — `2020-01-01T00:00:00Z` for definitely
//! overdue, `2099-01-01T00:00:00Z` for definitely not. The exact `now > until`
//! boundary is unit-tested to the second in `src/state.rs`.

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_pinki");

/// Far enough in the past that `now > until` regardless of when this runs.
const PAST: &str = "2020-01-01T00:00:00Z";
/// Far enough in the future that it is not.
const FUTURE: &str = "2099-01-01T00:00:00Z";

/// A scratch ledger that removes itself.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pinki-it-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).expect("scratch dir");
        Scratch { dir }
    }

    /// The ledger path handed to the child. One directory deeper than the scratch
    /// root on purpose: the first `promise` has to create `.pinki/` itself.
    fn ledger(&self) -> PathBuf {
        self.dir.join(".pinki").join("ledger.jsonl")
    }

    /// Every non-empty line currently in the ledger. An absent file is no lines.
    fn lines(&self) -> Vec<String> {
        match fs::read_to_string(self.ledger()) {
            Ok(text) => text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(str::to_string)
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn run(&self, args: &[&str]) -> Run {
        self.spawn(args, None, self.ledger().into_os_string())
    }

    fn feed(&self, args: &[&str], stdin: &str) -> Run {
        self.spawn(args, Some(stdin), self.ledger().into_os_string())
    }

    /// Run with `PINKI_LEDGER` set to a bare filename. The child already runs in the
    /// scratch directory, so the ledger lands there — and there is no parent directory
    /// for pinki to create.
    fn run_with_bare_ledger(&self, name: &str, args: &[&str]) -> Run {
        self.spawn(args, None, OsString::from(name))
    }

    fn spawn(&self, args: &[&str], stdin: Option<&str>, ledger: OsString) -> Run {
        let mut child = Command::new(BIN)
            .args(args)
            .env("PINKI_LEDGER", &ledger)
            .current_dir(&self.dir)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn pinki");

        if let Some(text) = stdin {
            // A child that refuses its arguments exits before it ever reads stdin, and
            // the pipe then breaks under this write. That early exit is the behaviour
            // some tests exist to observe (`--meta` beside a record on stdin is refused
            // before the record is read), so a broken pipe here is the child declining
            // the input, not the harness failing — and which side wins the race is the
            // scheduler's call, not the test's (it lost once on CI, never locally).
            match child
                .stdin
                .as_mut()
                .expect("stdin pipe")
                .write_all(text.as_bytes())
            {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
                Err(e) => panic!("write stdin: {e}"),
            }
        }

        let output = child.wait_with_output().expect("wait for pinki");
        Run {
            args: args.iter().map(|a| a.to_string()).collect(),
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8(output.stdout).expect("utf-8 stdout"),
            stderr: String::from_utf8(output.stderr).expect("utf-8 stderr"),
        }
    }

    /// Make a promise and hand back its id.
    fn promise(&self, text: &str, by: &str, to: &str, until: &str) -> String {
        let run = self.run(&["promise", text, "--by", by, "--to", to, "--until", until]);
        run.expect(0);
        run.id()
    }

    /// Every ledger line, parsed. Panics on a line that is not JSON — the ledger this
    /// binary wrote is always JSONL, and a test should say so loudly if it is not.
    fn events(&self) -> Vec<serde_json::Map<String, serde_json::Value>> {
        self.lines()
            .iter()
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|e| panic!("not a JSON line ({e}): {line}"))
            })
            .collect()
    }

    /// Rewrite the whole ledger, passing each event through `edit`.
    ///
    /// This is how a test reaches a state the CLI will not produce — a `meta` on an
    /// event some other writer put there, for instance — and it goes through the real
    /// file, so the next command really deserializes what was written.
    fn rewrite(&self, edit: impl Fn(usize, &mut serde_json::Map<String, serde_json::Value>)) {
        let mut out = String::new();
        for (n, mut event) in self.events().into_iter().enumerate() {
            edit(n, &mut event);
            out.push_str(&serde_json::to_string(&event).expect("re-encode the event"));
            out.push('\n');
        }
        fs::write(self.ledger(), out).expect("rewrite the ledger");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct Run {
    args: Vec<String>,
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    /// Assert the exit code, reporting the whole run when it disagrees.
    fn expect(&self, code: i32) -> &Self {
        assert_eq!(
            self.code,
            code,
            "`pinki {}` exited {} (wanted {code})\n--- stdout ---\n{}\n--- stderr ---\n{}",
            self.args.join(" "),
            self.code,
            self.stdout,
            self.stderr
        );
        self
    }

    fn expect_failure(&self) -> &Self {
        assert_ne!(
            self.code,
            0,
            "`pinki {}` unexpectedly succeeded\n--- stdout ---\n{}",
            self.args.join(" "),
            self.stdout
        );
        self
    }

    /// The id from a mutating verb's `id<TAB>state` line.
    fn id(&self) -> String {
        self.stdout
            .trim()
            .split('\t')
            .next()
            .expect("an id on stdout")
            .to_string()
    }

    /// The state from a mutating verb's `id<TAB>state` line.
    fn state(&self) -> String {
        self.stdout
            .trim()
            .split('\t')
            .nth(1)
            .unwrap_or_else(|| panic!("no state in {:?}", self.stdout))
            .to_string()
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| {
            panic!(
                "stdout of `pinki {}` is not JSON ({e}):\n{}",
                self.args.join(" "),
                self.stdout
            )
        })
    }
}

fn is_minted_id(id: &str) -> bool {
    match id.strip_prefix("pnk_") {
        Some(hex) => {
            hex.len() == 6
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        }
        None => false,
    }
}

/// The state `ls --json` reports for one id.
fn state_of(scratch: &Scratch, id: &str) -> String {
    let run = scratch.run(&["ls", "--all", "--json"]);
    run.expect(0);
    let rows = run.json();
    for row in rows.as_array().expect("an array") {
        if row["id"] == id {
            return row["state"].as_str().expect("a state string").to_string();
        }
    }
    panic!("{id} is missing from `ls --all --json`:\n{}", run.stdout);
}

// ------------------------------------------------------------------ promise

#[test]
fn promise_creates_the_ledger_and_prints_a_minted_id() {
    let scratch = Scratch::new();
    assert!(!scratch.ledger().exists(), "ledger must not pre-exist");

    let run = scratch.run(&[
        "promise",
        "hand back a reviewed schema",
        "--by",
        "reviewer",
        "--to",
        "author",
        "--until",
        FUTURE,
    ]);
    run.expect(0);

    let id = run.id();
    assert!(is_minted_id(&id), "not a minted id: {id:?}");
    assert_eq!(run.state(), "detached");

    // The parent directory did not exist a moment ago; the first write makes it.
    assert!(scratch.ledger().exists(), "the ledger was not created");

    let lines = scratch.lines();
    assert_eq!(lines.len(), 1, "expected exactly one event: {lines:?}");
    let event: serde_json::Value = serde_json::from_str(&lines[0]).expect("valid JSON line");
    assert_eq!(event["type"], "promise");
    assert_eq!(event["id"], id.as_str());
    assert_eq!(event["promise"], "hand back a reviewed schema");
    assert_eq!(event["by"], "reviewer");
    assert_eq!(event["to"], "author");
    assert!(event.get("on").is_none(), "absent `on` must not be written");
}

#[test]
fn until_is_stored_normalized() {
    let scratch = Scratch::new();
    let run = scratch.run(&[
        "promise",
        "send the draft schema",
        "--by",
        "author",
        "--to",
        "reviewer",
        "--until",
        "2026-09-01T17:00Z",
    ]);
    run.expect(0);

    let event: serde_json::Value = serde_json::from_str(&scratch.lines()[0]).expect("valid JSON");
    assert_eq!(event["until"], "2026-09-01T17:00:00Z");
    // §4's examples are second-precision; nanoseconds would be precision pinki has
    // no use for.
    let ts = event["ts"].as_str().expect("a ts");
    assert!(ts.ends_with('Z') && !ts.contains('.'), "ts was {ts:?}");
}

#[test]
fn a_naive_until_is_rejected_and_writes_nothing() {
    let scratch = Scratch::new();
    let run = scratch.run(&[
        "promise",
        "send the draft schema",
        "--by",
        "author",
        "--to",
        "reviewer",
        "--until",
        "2026-09-01T17:00:00",
    ]);
    run.expect(2);
    assert!(
        run.stderr.contains("offset"),
        "the error should say what was wrong: {:?}",
        run.stderr
    );
    assert!(
        run.stderr.contains("2026-09-01T17:00Z"),
        "the error should show an accepted form: {:?}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "errors never go to stdout");
    assert!(scratch.lines().is_empty(), "nothing may be appended");
}

#[test]
fn a_missing_required_flag_is_a_usage_error() {
    let scratch = Scratch::new();
    scratch
        .run(&[
            "promise",
            "no creditor",
            "--by",
            "author",
            "--until",
            FUTURE,
        ])
        .expect(2);
    assert!(scratch.lines().is_empty());
}

#[test]
fn a_full_record_on_stdin_is_accepted() {
    let scratch = Scratch::new();
    let record = r#"{"promise":"ship it","by":"author","to":"publisher",
                     "until":"2026-09-03T12:00Z","task":"a2a-task-9c1f0e"}"#;
    let run = scratch.feed(&["promise"], record);
    run.expect(0);
    assert!(is_minted_id(&run.id()));

    let event: serde_json::Value = serde_json::from_str(&scratch.lines()[0]).expect("valid JSON");
    assert_eq!(event["promise"], "ship it");
    assert_eq!(event["task"], "a2a-task-9c1f0e");
    assert_eq!(event["until"], "2026-09-03T12:00:00Z");
}

#[test]
fn a_stdin_record_without_until_is_rejected() {
    let scratch = Scratch::new();
    let run = scratch.feed(
        &["promise"],
        r#"{"promise":"ship it","by":"author","to":"publisher"}"#,
    );
    run.expect(2);
    assert!(run.stderr.contains("until"), "{:?}", run.stderr);
    assert!(scratch.lines().is_empty());
}

#[test]
fn a_stdin_record_with_an_unknown_key_is_rejected() {
    let scratch = Scratch::new();
    // A typo'd key silently dropped is exactly the failure this tool exists to catch.
    let run = scratch.feed(
        &["promise"],
        r#"{"promise":"ship it","by":"author","to":"publisher","untl":"2026-09-03T12:00Z"}"#,
    );
    run.expect(2);
    assert!(run.stderr.contains("untl"), "{:?}", run.stderr);
    assert!(scratch.lines().is_empty());
}

#[test]
fn promise_with_neither_text_nor_stdin_is_a_usage_error() {
    let scratch = Scratch::new();
    // stdin is /dev/null here — nothing was piped in.
    scratch.run(&["promise"]).expect(2);
    assert!(scratch.lines().is_empty());
}

#[test]
fn an_unknown_antecedent_warns_but_proceeds() {
    let scratch = Scratch::new();
    let run = scratch.run(&[
        "promise",
        "waits on a ghost",
        "--by",
        "author",
        "--to",
        "reviewer",
        "--until",
        FUTURE,
        "--on",
        "pnk_000000",
    ]);
    // §3: an id the ledger has never heard of has certainly not satisfied, so this is
    // conditional — not an error. Ledgers get joined; half a conversation is normal.
    run.expect(0);
    assert_eq!(run.state(), "conditional");
    assert!(
        run.stderr.contains("pnk_000000"),
        "expected a warning naming the antecedent: {:?}",
        run.stderr
    );
    assert_eq!(scratch.lines().len(), 1);
}

#[test]
fn an_explicit_id_is_recorded_instead_of_a_minted_one() {
    let scratch = Scratch::new();
    let run = scratch.run(&[
        "promise",
        "send the draft schema",
        "--by",
        "author",
        "--to",
        "reviewer",
        "--until",
        FUTURE,
        "--id",
        "pnk_4f3a91",
    ]);
    run.expect(0);
    assert_eq!(run.id(), "pnk_4f3a91");

    let event: serde_json::Value = serde_json::from_str(&scratch.lines()[0]).expect("valid JSON");
    assert_eq!(event["id"], "pnk_4f3a91");
}

// --------------------------------------------------------------- foreign ids
//
// §1: an id a caller brings is opaque. These drive the real id space that motivated
// issue #5 — a fleet with ~584 rows keyed `pr-<stamp>-<hash>`, referenced from other
// systems — through every verb that takes an id, because an id that can be declared
// and then not resolved is worse than one that was refused up front.

/// A real-shaped id from the fleet that reported #5. Nothing about it is a pinki id.
const FOREIGN: &str = "pr-20260906203134-225e24cd";

#[test]
fn a_foreign_id_round_trips_through_every_verb_that_takes_one() {
    let scratch = Scratch::new();
    let declared = scratch.run(&[
        "promise",
        "hand back a reviewed schema",
        "--by",
        "reviewer",
        "--to",
        "author",
        "--until",
        FUTURE,
        "--id",
        FOREIGN,
        "--task",
        "a2a-task-9c1f0e",
    ]);
    declared.expect(0);
    assert_eq!(declared.id(), FOREIGN);
    assert_eq!(declared.state(), "detached");

    // On the line, verbatim — pinki neither rewrites nor decorates it. And the line is
    // the same *shape* it has always been: §1 gained no `external_id`, so the keys are
    // exactly the ones a minted id writes, which is why a v0.1.0 reader can still read
    // this file (see CHANGELOG's Compatibility note).
    let event: serde_json::Value = serde_json::from_str(&scratch.lines()[0]).expect("valid JSON");
    assert_eq!(event["id"], FOREIGN);
    let keys: Vec<&str> = event
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    // serde_json sorts an object's keys on the way in, so this is the key *set*; the
    // on-disk field order is asserted by `src/record.rs`'s own round-trip test.
    assert_eq!(
        keys,
        ["by", "id", "promise", "task", "to", "ts", "type", "until"],
        "a supplied id adds no field and removes none"
    );

    // ls, in both forms.
    let table = scratch.run(&["ls"]);
    table.expect(0);
    assert!(table.stdout.contains(FOREIGN), "{:?}", table.stdout);
    let rows = scratch.run(&["ls", "--json"]);
    rows.expect(0);
    let rows = rows.json();
    assert_eq!(rows[0]["id"], FOREIGN);
    assert_eq!(rows[0]["state"], "detached");

    // show, in both forms.
    let shown = scratch.run(&["show", FOREIGN]);
    shown.expect(0);
    assert!(shown.stdout.starts_with(FOREIGN), "{:?}", shown.stdout);
    let detail = scratch.run(&["show", FOREIGN, "--json"]);
    detail.expect(0);
    assert_eq!(detail.json()["id"], FOREIGN);

    // The A2A metadata block carries it unchanged — which is the point of #5's second
    // consequence: two parties join on this key or they do not join at all.
    let block = scratch.run(&["a2a", "task", FOREIGN]);
    block.expect(0);
    let block = block.json();
    let (_, record) = block.as_object().expect("an object").iter().next().unwrap();
    assert_eq!(record["id"], FOREIGN);

    // assess, then resolve. Both name the id, neither re-validates its shape.
    scratch
        .run(&["assess", FOREIGN, "--violated", "--observer", "author"])
        .expect(0);
    let resolved = scratch.run(&[
        "resolve",
        FOREIGN,
        "--satisfied",
        "--evidence",
        "https://example.org/reviews/91",
    ]);
    resolved.expect(0);
    assert_eq!(resolved.state(), "satisfied");
    assert_eq!(state_of(&scratch, FOREIGN), "satisfied");

    // And it is a legal antecedent, so a foreign id joins the graph too.
    let dependent = scratch.run(&[
        "promise",
        "ship it",
        "--by",
        "author",
        "--to",
        "publisher",
        "--until",
        FUTURE,
        "--on",
        FOREIGN,
    ]);
    dependent.expect(0);
    assert_eq!(
        dependent.state(),
        "detached",
        "the antecedent is satisfied, so the dependent detaches"
    );
    assert_eq!(
        dependent.stderr, "",
        "a declared antecedent earns no warning: {:?}",
        dependent.stderr
    );
}

#[test]
fn a_foreign_id_declared_twice_is_refused_like_any_other() {
    let scratch = Scratch::new();
    let declare = |text: &str| {
        scratch.run(&[
            "promise", text, "--by", "author", "--to", "reviewer", "--until", FUTURE, "--id",
            FOREIGN,
        ])
    };
    declare("send the draft schema").expect(0);

    let run = declare("send a different schema");
    run.expect(1);
    assert!(
        run.stderr.contains("already declared"),
        "§4's first-declaration-wins guard does not care which id space this came \
         from: {:?}",
        run.stderr
    );
    assert_eq!(scratch.lines().len(), 1, "the first declaration stands");
}

#[test]
fn a_supplied_id_that_collides_with_a_minted_one_is_refused() {
    let scratch = Scratch::new();
    // The minted id is random, so the only deterministic way to collide with one is to
    // mint it first and then hand it back. That is also the real case: a caller who
    // read an id out of `ls` and passed it to `--id` by mistake.
    let minted = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    assert!(is_minted_id(&minted));

    let run = scratch.run(&[
        "promise",
        "send a different schema",
        "--by",
        "author",
        "--to",
        "reviewer",
        "--until",
        FUTURE,
        "--id",
        &minted,
    ]);
    run.expect(1);
    assert!(run.stderr.contains("already declared"), "{:?}", run.stderr);
    assert_eq!(scratch.lines().len(), 1);
}

#[test]
fn a_supplied_id_wearing_the_minted_prefix_must_be_a_minted_one() {
    let scratch = Scratch::new();
    // `pnk_` stays reserved so `is_well_formed` keeps meaning "pinki minted this".
    for id in ["pnk_promise", "pnk_4f3a9", "pnk_4F3A91", "pnk_"] {
        let run = scratch.run(&[
            "promise",
            "send the draft schema",
            "--by",
            "author",
            "--to",
            "reviewer",
            "--until",
            FUTURE,
            "--id",
            id,
        ]);
        run.expect(2);
        assert!(
            run.stderr.contains("reserved"),
            "the refusal should name the reservation: {:?}",
            run.stderr
        );
        assert!(
            run.stderr.contains("six lowercase hex digits"),
            "and say what a minted id looks like: {:?}",
            run.stderr
        );
        assert!(scratch.lines().is_empty(), "nothing may be appended: {id}");
    }

    // An id merely near the prefix is nobody's business but the caller's.
    scratch
        .run(&[
            "promise",
            "send the draft schema",
            "--by",
            "author",
            "--to",
            "reviewer",
            "--until",
            FUTURE,
            "--id",
            "pnk-4f3a91",
        ])
        .expect(0);
}

#[test]
fn a_supplied_id_that_is_not_usable_as_a_handle_is_a_usage_error() {
    let scratch = Scratch::new();
    // Blank, whitespace-only, whitespace inside, and a control character. Each is
    // malformed input — exit 2, nothing appended.
    for id in ["", "   ", "two words", "tab\there", "bel\u{7}here"] {
        let run = scratch.run(&[
            "promise",
            "send the draft schema",
            "--by",
            "author",
            "--to",
            "reviewer",
            "--until",
            FUTURE,
            "--id",
            id,
        ]);
        run.expect(2);
        assert!(
            run.stderr.contains("§1"),
            "the refusal cites the rule: {:?}",
            run.stderr
        );
        assert!(
            scratch.lines().is_empty(),
            "nothing may be appended for {id:?}"
        );
    }
}

#[test]
fn a_supplied_id_may_not_hide_an_invisible_character() {
    // §1: an id is a handle you can read, type and compare by eye. A character with
    // the Unicode `Default_Ignorable_Code_Point` property renders as nothing, so two
    // ids that are not equal can be indistinguishable in `ls`, in a terminal, and in
    // a code review — and the id is the key §7's join is made on. Neither
    // `char::is_whitespace` nor `char::is_control` is true of any of them (that is
    // the gap this test exists for), so the property has to be its own rule.
    let scratch = Scratch::new();
    scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    let before = fs::read(scratch.ledger()).expect("a ledger to compare against");

    for (id, what) in [
        ("pr-9\u{200B}1", "U+200B ZERO WIDTH SPACE"),
        ("pr-9\u{FEFF}1", "U+FEFF ZERO WIDTH NO-BREAK SPACE (BOM)"),
        ("pr-9\u{2060}1", "U+2060 WORD JOINER"),
        ("pr-9\u{E0001}1", "U+E0001 LANGUAGE TAG"),
    ] {
        let run = scratch.run(&[
            "promise",
            "send a different schema",
            "--by",
            "author",
            "--to",
            "reviewer",
            "--until",
            FUTURE,
            "--id",
            id,
        ]);
        run.expect(2);
        assert!(
            run.stderr.contains("§1"),
            "the refusal cites the rule for {what}: {:?}",
            run.stderr
        );
        assert!(
            run.stderr.contains("Default_Ignorable_Code_Point"),
            "the refusal names the property for {what}: {:?}",
            run.stderr
        );
        assert_eq!(
            fs::read(scratch.ledger()).expect("the ledger is still there"),
            before,
            "the ledger must be byte-identical after refusing {what}"
        );
    }

    // The positive control, and the reason this is a property rather than a ban on
    // non-ASCII: an id in another script holds nothing invisible and is fine.
    for id in ["υπόσχεση-91", "約束-91", "promesse-91"] {
        let run = scratch.run(&[
            "promise",
            "ship it",
            "--by",
            "author",
            "--to",
            "publisher",
            "--until",
            FUTURE,
            "--id",
            id,
        ]);
        run.expect(0);
        assert_eq!(run.id(), id);
    }
}

#[test]
fn a_supplied_id_is_bounded_at_128_characters() {
    let scratch = Scratch::new();
    let declare = |id: &str| {
        scratch.run(&[
            "promise",
            "send the draft schema",
            "--by",
            "author",
            "--to",
            "reviewer",
            "--until",
            FUTURE,
            "--id",
            id,
        ])
    };

    let over = "x".repeat(129);
    let run = declare(&over);
    run.expect(2);
    assert!(run.stderr.contains("128"), "{:?}", run.stderr);
    assert!(run.stderr.contains("129"), "{:?}", run.stderr);
    assert!(scratch.lines().is_empty(), "nothing may be appended");

    let at_the_bound = "x".repeat(128);
    let run = declare(&at_the_bound);
    run.expect(0);
    assert_eq!(run.id(), at_the_bound);
}

#[test]
fn a_stdin_record_may_carry_a_foreign_id_too() {
    let scratch = Scratch::new();
    // §5: "`pinki promise` reads JSON on stdin like everything else" — so the same id
    // policy, on the same field, by the same code.
    let record = format!(
        r#"{{"id":"{FOREIGN}","promise":"ship it","by":"author","to":"publisher",
             "until":"2026-09-03T12:00Z"}}"#
    );
    let run = scratch.feed(&["promise"], &record);
    run.expect(0);
    assert_eq!(run.id(), FOREIGN);

    let event: serde_json::Value = serde_json::from_str(&scratch.lines()[0]).expect("valid JSON");
    assert_eq!(event["id"], FOREIGN);

    // And the refusal arrives the same way on that path.
    let bad = r#"{"id":"pnk_nope","promise":"ship it","by":"author","to":"publisher",
                  "until":"2026-09-03T12:00Z"}"#;
    let run = scratch.feed(&["promise"], bad);
    run.expect(2);
    assert!(run.stderr.contains("reserved"), "{:?}", run.stderr);
    assert_eq!(scratch.lines().len(), 1, "only the good one is on the file");
}

#[test]
fn an_explicit_id_that_is_already_declared_is_refused() {
    let scratch = Scratch::new();
    let declare = |text: &str| {
        scratch.run(&[
            "promise",
            text,
            "--by",
            "author",
            "--to",
            "reviewer",
            "--until",
            FUTURE,
            "--id",
            "pnk_4f3a91",
        ])
    };
    declare("send the draft schema").expect(0);

    // §4: ids are stable and the first declaration wins, so the second is refused
    // rather than allowed to move the deadline out from under the first.
    let run = declare("send a different schema");
    run.expect(1);
    assert!(
        run.stderr.contains("already declared"),
        "the error should say the id is spoken for: {:?}",
        run.stderr
    );
    assert_eq!(scratch.lines().len(), 1, "the first declaration stands");
}

// -------------------------------------------------------------------- amend

/// Fifteen minutes past `PAST`, so a nudge that lands here is still late.
const PAST_PLUS_15: &str = "2020-01-01T00:15:00Z";

/// The escalation ladder from issue #9, end to end.
///
/// A watchdog re-declares the *same* promise on a short clock when it lapses — minting
/// a new id per nudge was considered and rejected, because it leaves a phantom promise
/// behind for every rung. v0.1.0 refused that outright (first declaration wins), so the
/// ladder is what `amend` exists for, and this is its acceptance case:
///
/// ```text
/// declare  until = T
/// amend    until = T + 15m     (nudge 1, same id)
/// amend    until = T + 30m     (nudge 2, same id)
/// resolve
/// ```
#[test]
fn the_escalation_ladder_nudges_one_promise_twice() {
    let scratch = Scratch::new();
    let id = scratch.promise("hand back the report", "watchdog", "desk", PAST);
    assert_eq!(state_of(&scratch, &id), "overdue");

    // Nudge 1. The horizon moves and the state is recomputed against it — still late,
    // because this rung is still in the past.
    let first = scratch.run(&["amend", &id, "--until", PAST_PLUS_15, "--reason", "nudge 1"]);
    first.expect(0);
    assert_eq!(first.state(), "overdue");

    // Nudge 2, onto a horizon that has not passed: the state follows the deadline.
    let second = scratch.run(&["amend", &id, "--until", FUTURE, "--reason", "nudge 2"]);
    second.expect(0);
    assert_eq!(second.state(), "detached");

    // `ls` reports the current horizon, and the state computed against it.
    let rows = scratch.run(&["ls", "--json"]);
    rows.expect(0);
    let listed = rows.json();
    let row = &listed.as_array().expect("an array")[0];
    assert_eq!(row["id"], id.as_str());
    assert_eq!(row["until"], FUTURE, "ls shows the horizon as it stands");
    assert_eq!(row["state"], "detached");

    // Nothing was rewritten: one declaration, two amends, and the declaration still
    // says exactly what it said.
    let lines = scratch.lines();
    assert_eq!(lines.len(), 3, "{lines:?}");
    let declaration: serde_json::Value = serde_json::from_str(&lines[0]).expect("valid JSON");
    assert_eq!(declaration["type"], "promise");
    assert_eq!(declaration["until"], PAST);

    let nudge: serde_json::Value = serde_json::from_str(&lines[1]).expect("valid JSON");
    assert_eq!(nudge["type"], "amend");
    assert_eq!(nudge["promise"], id.as_str());
    // §8.2: the debtor's own act, and the default actor.
    assert_eq!(nudge["by"], "watchdog");
    assert_eq!(nudge["until"], PAST_PLUS_15);
    assert_eq!(nudge["reason"], "nudge 1");

    // `show` lists every horizon it has ever had, in order.
    let detail = scratch.run(&["show", &id, "--json"]);
    detail.expect(0);
    let json = detail.json();
    assert_eq!(json["until"], FUTURE);
    let horizons = json["horizons"].as_array().expect("horizons");
    assert_eq!(horizons.len(), 3, "{horizons:?}");
    assert_eq!(horizons[0]["until"], PAST);
    assert!(
        horizons[0].get("reason").is_none(),
        "the declaration needs no excuse: {}",
        horizons[0]
    );
    assert_eq!(horizons[1]["until"], PAST_PLUS_15);
    assert_eq!(horizons[1]["reason"], "nudge 1");
    assert_eq!(horizons[2]["until"], FUTURE);
    assert_eq!(horizons[2]["reason"], "nudge 2");
    for horizon in horizons {
        assert_eq!(horizon["by"], "watchdog");
        assert!(horizon["ts"].is_string());
        // A ladder that carries no provenance is handed back none: `meta` is offered,
        // never manufactured.
        assert!(horizon.get("meta").is_none(), "{horizon}");
    }

    let text = scratch.run(&["show", &id]);
    text.expect(0);
    assert!(
        text.stdout.contains(&format!("until        {FUTURE}\n")),
        "{:?}",
        text.stdout
    );
    assert!(text.stdout.contains("horizons\n"), "{:?}", text.stdout);
    assert!(
        text.stdout
            .contains(&format!("  {PAST}  watchdog  (declared)\n")),
        "{:?}",
        text.stdout
    );
    assert!(
        text.stdout
            .contains(&format!("  {PAST_PLUS_15}  watchdog  nudge 1\n")),
        "{:?}",
        text.stdout
    );

    // An amend is not a resolution: the promise is still there to be resolved, and the
    // first resolve still wins.
    scratch
        .run(&[
            "resolve",
            &id,
            "--satisfied",
            "--evidence",
            "https://example.org/reports/9",
        ])
        .expect(0);
    assert_eq!(state_of(&scratch, &id), "satisfied");

    let second_resolve =
        scratch.run(&["resolve", &id, "--cancelled", "--reason", "changed my mind"]);
    second_resolve.expect(1);
    assert!(
        second_resolve.stderr.contains("already resolved"),
        "{:?}",
        second_resolve.stderr
    );

    // And a rung of the ladder that arrives after the promise ended is refused too:
    // there is no horizon left to move.
    let late_nudge = scratch.run(&["amend", &id, "--until", FUTURE, "--reason", "nudge 3"]);
    late_nudge.expect(1);
    assert!(
        late_nudge.stderr.contains("already resolved"),
        "the error should name the resolution that ended it: {:?}",
        late_nudge.stderr
    );
    assert_eq!(
        scratch.lines().len(),
        4,
        "one promise, two amends, one resolve"
    );
}

#[test]
fn an_amend_by_anyone_but_the_debtor_is_refused() {
    let scratch = Scratch::new();
    let id = scratch.promise("hand back the report", "watchdog", "desk", PAST);
    let before = scratch.lines();

    // §8.2: moving a horizon is the debtor's own act. An observer who thinks the
    // deadline should move is making an assessment, which is a different speech act.
    let run = scratch.run(&["amend", &id, "--until", FUTURE, "--by", "desk"]);
    run.expect(1);
    assert!(
        run.stderr.contains("§8.2"),
        "the error should cite the rule: {:?}",
        run.stderr
    );
    assert!(
        run.stderr.contains("watchdog"),
        "the error should name the debtor who may: {:?}",
        run.stderr
    );
    assert!(
        run.stderr.contains("assess"),
        "the error should point at the verb that does fit: {:?}",
        run.stderr
    );
    assert_eq!(scratch.lines(), before, "the ledger must be unchanged");

    // The debtor naming itself is the same command, and it works.
    scratch
        .run(&["amend", &id, "--until", FUTURE, "--by", "watchdog"])
        .expect(0);
    assert_eq!(state_of(&scratch, &id), "detached");
}

#[test]
fn an_amend_with_an_unreadable_until_writes_nothing() {
    let scratch = Scratch::new();
    let id = scratch.promise("hand back the report", "watchdog", "desk", FUTURE);
    let before = scratch.lines();

    let run = scratch.run(&["amend", &id, "--until", "2026-09-01T17:00:00"]);
    run.expect(2);
    assert!(run.stderr.contains("offset"), "{:?}", run.stderr);
    assert_eq!(scratch.lines(), before);

    // A deadline is required: there is nothing to amend without one.
    scratch.run(&["amend", &id]).expect(2);
    assert_eq!(scratch.lines(), before);

    // Whitespace is not a reason.
    scratch
        .run(&["amend", &id, "--until", FUTURE, "--reason", "   "])
        .expect(2);
    assert_eq!(scratch.lines(), before);
}

/// The forward-compatibility property, through the real reader.
///
/// This is the half that has to be tested against a *file*, not an in-memory `Vec`:
/// the claim is about what a build does when a ledger carries an event type it has
/// never heard of, and that is a deserialization question before it is a fold question.
/// v0.1.0 answers it by refusing the entire file — which is exactly why this version
/// answers it differently, and why the CHANGELOG says so out loud.
#[test]
fn an_event_type_this_build_does_not_know_costs_nothing_else_in_the_ledger() {
    let scratch = Scratch::new();
    let id = scratch.promise("hand back the report", "watchdog", "desk", PAST);
    scratch
        .run(&["amend", &id, "--until", FUTURE, "--reason", "nudge 1"])
        .expect(0);

    // A line a newer pinki might write, appended between events this one understands.
    let mut text = fs::read_to_string(scratch.ledger()).expect("the ledger");
    text.push_str(
        "{\"ts\":\"2026-09-02T09:00:00Z\",\"type\":\"frobnicate\",\"promise\":\"pnk_000000\",\"wat\":1}\n",
    );
    fs::write(scratch.ledger(), &text).expect("write the ledger back");

    let run = scratch.run(&["ls", "--json"]);
    run.expect(0);
    let listed = run.json();
    let rows = listed.as_array().expect("an array");
    assert_eq!(rows.len(), 1, "the promise is still readable: {rows:?}");
    assert_eq!(rows[0]["id"], id.as_str());
    // The events it does know still fold exactly as they did.
    assert_eq!(rows[0]["until"], FUTURE);
    assert_eq!(rows[0]["state"], "detached");

    // Loud, not fatal, and never silent: the warning names the line.
    assert!(
        run.stderr.contains("unknown event type"),
        "the reader must say what it skipped: {:?}",
        run.stderr
    );
    assert!(
        run.stderr.contains("line 3"),
        "the warning should name the line: {:?}",
        run.stderr
    );

    // A line that is not an event at all is still fatal — that rule did not move.
    fs::write(scratch.ledger(), format!("{text}{{not json\n")).expect("write the ledger back");
    scratch.run(&["ls", "--json"]).expect(1);
}

#[test]
fn amending_an_unknown_id_fails_operationally() {
    let scratch = Scratch::new();
    let run = scratch.run(&["amend", "pnk_000000", "--until", FUTURE]);
    run.expect(1);
    assert!(scratch.lines().is_empty());
}

#[test]
fn a_deadline_may_move_earlier_too() {
    let scratch = Scratch::new();
    // Not every move is an extension — pinki has no opinion about the direction, only
    // about the move being visible.
    let id = scratch.promise("hand back the report", "watchdog", "desk", FUTURE);
    let run = scratch.run(&["amend", &id, "--until", PAST, "--reason", "brought forward"]);
    run.expect(0);
    assert_eq!(run.state(), "overdue");
}

#[test]
fn show_has_no_horizons_block_until_a_deadline_moves() {
    let scratch = Scratch::new();
    let id = scratch.promise("hand back the report", "watchdog", "desk", FUTURE);

    let text = scratch.run(&["show", &id]);
    text.expect(0);
    assert!(
        !text.stdout.contains("horizons"),
        "a promise nobody amended has one horizon, and a list of one is noise: {:?}",
        text.stdout
    );

    // The JSON field is always there, so a consumer can test it rather than probe.
    let json = scratch.run(&["show", &id, "--json"]);
    json.expect(0);
    let horizons = json.json()["horizons"].as_array().expect("horizons").len();
    assert_eq!(horizons, 1, "the declaration is a horizon");
}

#[test]
fn the_a2a_block_carries_the_horizon_as_it_stands() {
    let scratch = Scratch::new();
    let id = scratch.promise("hand back the report", "watchdog", "desk", PAST);
    scratch
        .run(&["amend", &id, "--until", FUTURE, "--reason", "nudge 1"])
        .expect(0);

    let run = scratch.run(&["a2a", "task", &id]);
    run.expect(0);
    let block = run.json();
    let (_, record) = block.as_object().expect("an object").iter().next().unwrap();
    // A peer computing `now > until` for itself must get the answer this ledger gets —
    // the divergence that filed issue #9 was exactly this, 32 minutes wide.
    assert_eq!(record["until"], FUTURE);
    assert_eq!(
        record.as_object().expect("record object").len(),
        5,
        "still the §1 record and nothing else: {record}"
    );
}

// ------------------------------------------------------------------ resolve

#[test]
fn resolve_satisfied_with_evidence() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);

    let run = scratch.run(&[
        "resolve",
        &id,
        "--satisfied",
        "--evidence",
        "https://example.org/schemas/draft-3.json",
    ]);
    run.expect(0);
    assert_eq!(run.state(), "satisfied");

    let event: serde_json::Value = serde_json::from_str(&scratch.lines()[1]).expect("valid JSON");
    assert_eq!(event["type"], "resolve");
    assert_eq!(event["as"], "satisfied");
    // §4's "Issued by": the debtor satisfies, and that is the default.
    assert_eq!(event["by"], "author");
    assert_eq!(
        event["evidence"][0],
        "https://example.org/schemas/draft-3.json"
    );
}

#[test]
fn satisfied_without_evidence_leaves_the_ledger_untouched() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    let before = scratch.lines();

    let run = scratch.run(&["resolve", &id, "--satisfied"]);
    run.expect_failure();
    assert!(
        run.stderr.contains("evidence"),
        "the error should name the rule: {:?}",
        run.stderr
    );
    assert_eq!(scratch.lines(), before, "the ledger must be unchanged");
}

#[test]
fn blank_evidence_leaves_the_ledger_untouched() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    let before = scratch.lines();

    scratch
        .run(&["resolve", &id, "--satisfied", "--evidence", ""])
        .expect(2);
    assert_eq!(scratch.lines(), before);

    scratch
        .run(&["resolve", &id, "--satisfied", "--evidence", "   "])
        .expect(2);
    assert_eq!(scratch.lines(), before);
}

#[test]
fn a_reason_is_refused_with_satisfied_because_a_reason_is_not_evidence() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    let before = scratch.lines();

    let run = scratch.run(&[
        "resolve",
        &id,
        "--satisfied",
        "--evidence",
        "https://example.org/x",
        "--reason",
        "it went fine",
    ]);
    run.expect(2);
    assert!(
        run.stderr.contains("--reason is not accepted"),
        "the error should name the rule: {:?}",
        run.stderr
    );
    assert_eq!(scratch.lines(), before, "the ledger must be unchanged");
}

#[test]
fn evidence_is_refused_with_released_because_releasing_claims_nothing_was_done() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    let before = scratch.lines();

    let run = scratch.run(&[
        "resolve",
        &id,
        "--released",
        "--evidence",
        "https://example.org/x",
    ]);
    run.expect(2);
    assert!(
        run.stderr.contains("--evidence is not accepted"),
        "the error should name the rule: {:?}",
        run.stderr
    );
    assert_eq!(scratch.lines(), before, "the ledger must be unchanged");
}

#[test]
fn a_released_reason_is_recorded_when_one_is_given() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);

    let run = scratch.run(&["resolve", &id, "--released", "--reason", "no longer needed"]);
    run.expect(0);

    let event: serde_json::Value = serde_json::from_str(&scratch.lines()[1]).expect("valid JSON");
    assert_eq!(event["as"], "released");
    assert_eq!(event["reason"], "no longer needed");
}

#[test]
fn a_blank_released_reason_leaves_the_ledger_untouched() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    let before = scratch.lines();

    // Whitespace is not a value: a reason offered and left empty is malformed input,
    // not the same thing as declining to give one.
    scratch
        .run(&["resolve", &id, "--released", "--reason", "   "])
        .expect(2);
    assert_eq!(scratch.lines(), before, "the ledger must be unchanged");
}

#[test]
fn resolve_cancelled_requires_a_reason() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);

    scratch.run(&["resolve", &id, "--cancelled"]).expect(2);
    assert_eq!(scratch.lines().len(), 1, "nothing appended");

    let run = scratch.run(&[
        "resolve",
        &id,
        "--cancelled",
        "--reason",
        "upstream schema was withdrawn",
    ]);
    run.expect(0);
    assert_eq!(run.state(), "cancelled");

    let event: serde_json::Value = serde_json::from_str(&scratch.lines()[1]).expect("valid JSON");
    assert_eq!(event["as"], "cancelled");
    assert_eq!(event["reason"], "upstream schema was withdrawn");
    assert_eq!(event["by"], "author");
}

#[test]
fn resolve_released_defaults_by_to_the_creditor() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);

    let run = scratch.run(&["resolve", &id, "--released"]);
    run.expect(0);
    assert_eq!(run.state(), "released");

    let event: serde_json::Value = serde_json::from_str(&scratch.lines()[1]).expect("valid JSON");
    assert_eq!(event["as"], "released");
    // Releasing is the creditor's act (§4), so `to` is the default actor.
    assert_eq!(event["by"], "reviewer");
    assert!(event.get("reason").is_none(), "no reason was given");
}

#[test]
fn an_explicit_by_that_disagrees_is_recorded_not_rejected() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);

    // §4: pinki "records who claimed what" — it does not verify the actor.
    let run = scratch.run(&[
        "resolve",
        &id,
        "--satisfied",
        "--evidence",
        "https://example.org/x",
        "--by",
        "somebody-else",
    ]);
    run.expect(0);

    let event: serde_json::Value = serde_json::from_str(&scratch.lines()[1]).expect("valid JSON");
    assert_eq!(event["by"], "somebody-else");
}

#[test]
fn the_first_resolve_wins_and_the_second_appends_nothing() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    scratch
        .run(&[
            "resolve",
            &id,
            "--satisfied",
            "--evidence",
            "https://example.org/x",
        ])
        .expect(0);

    let run = scratch.run(&["resolve", &id, "--cancelled", "--reason", "changed my mind"]);
    run.expect(1);
    assert!(
        run.stderr.contains("satisfied"),
        "the error should name the existing resolution: {:?}",
        run.stderr
    );

    let resolves = scratch
        .lines()
        .iter()
        .filter(|line| line.contains(r#""type":"resolve""#))
        .count();
    assert_eq!(resolves, 1, "exactly one resolve event may exist");
}

#[test]
fn resolving_an_unknown_id_fails_operationally() {
    let scratch = Scratch::new();
    let run = scratch.run(&[
        "resolve",
        "pnk_000000",
        "--satisfied",
        "--evidence",
        "https://example.org/x",
    ]);
    run.expect(1);
    assert!(scratch.lines().is_empty());
}

// ------------------------------------------------------------------ assess

#[test]
fn an_assessment_is_recorded_and_never_terminal() {
    let scratch = Scratch::new();
    let id = scratch.promise("ship it", "author", "publisher", PAST);

    let run = scratch.run(&[
        "assess",
        &id,
        "--violated",
        "--observer",
        "author",
        "--note",
        "nothing shipped, no reason given",
    ]);
    run.expect(0);

    let event: serde_json::Value = serde_json::from_str(&scratch.lines()[1]).expect("valid JSON");
    assert_eq!(event["type"], "assess");
    assert_eq!(event["state"], "violated");
    assert_eq!(event["observer"], "author");
    assert_eq!(event["note"], "nothing shipped, no reason given");

    // §4: "an assessed promise stays exactly as open as it was."
    let show = scratch.run(&["show", &id, "--json"]);
    show.expect(0);
    assert_eq!(show.json()["state"], "overdue");
    assert_eq!(show.json()["resolution"], serde_json::Value::Null);
    assert_eq!(
        show.json()["assessments"].as_array().expect("array").len(),
        1
    );
}

#[test]
fn assessing_an_unknown_id_fails_operationally() {
    let scratch = Scratch::new();
    scratch
        .run(&["assess", "pnk_000000", "--violated", "--observer", "author"])
        .expect(1);
}

// ---------------------------------------------------------------------- ls

#[test]
fn ls_on_an_empty_ledger_is_an_empty_array() {
    let scratch = Scratch::new();
    let run = scratch.run(&["ls", "--json"]);
    run.expect(0);
    assert_eq!(run.stdout.trim(), "[]");
    assert!(
        run.json().as_array().expect("an array").is_empty(),
        "an empty ledger is not an error"
    );
}

#[test]
fn ls_scopes_open_overdue_and_all() {
    let scratch = Scratch::new();
    let late = scratch.promise("ship it", "author", "publisher", PAST);
    let soon = scratch.promise("review it", "reviewer", "author", FUTURE);
    let done = scratch.promise("send it", "author", "reviewer", FUTURE);
    scratch
        .run(&[
            "resolve",
            &done,
            "--satisfied",
            "--evidence",
            "https://example.org/x",
        ])
        .expect(0);

    let ids = |args: &[&str]| -> Vec<String> {
        let run = scratch.run(args);
        run.expect(0);
        run.json()
            .as_array()
            .expect("an array")
            .iter()
            .map(|row| row["id"].as_str().expect("an id").to_string())
            .collect()
    };

    // Default is --open: conditional + detached + overdue, terminal states excluded.
    let default = ids(&["ls", "--json"]);
    assert_eq!(default, ids(&["ls", "--open", "--json"]));
    assert!(default.contains(&late) && default.contains(&soon));
    assert!(!default.contains(&done), "a satisfied promise is not open");

    assert_eq!(ids(&["ls", "--overdue", "--json"]), vec![late.clone()]);

    let all = ids(&["ls", "--all", "--json"]);
    assert_eq!(all.len(), 3);
    assert!(all.contains(&done));
}

#[test]
fn ls_scope_flags_are_mutually_exclusive() {
    let scratch = Scratch::new();
    scratch.run(&["ls", "--open", "--all"]).expect(2);
}

#[test]
fn ls_filters_by_and_to_orthogonally() {
    let scratch = Scratch::new();
    let mine = scratch.promise("ship it", "author", "publisher", FUTURE);
    let theirs = scratch.promise("review it", "reviewer", "author", FUTURE);

    let ids = |args: &[&str]| -> Vec<String> {
        let run = scratch.run(args);
        run.expect(0);
        run.json()
            .as_array()
            .expect("an array")
            .iter()
            .map(|row| row["id"].as_str().expect("an id").to_string())
            .collect()
    };

    assert_eq!(ids(&["ls", "--by", "author", "--json"]), vec![mine.clone()]);
    assert_eq!(ids(&["ls", "--to", "author", "--json"]), vec![theirs]);
    // Combined with each other and with the state filter.
    assert_eq!(
        ids(&[
            "ls",
            "--all",
            "--by",
            "author",
            "--to",
            "publisher",
            "--json"
        ]),
        vec![mine]
    );
    assert!(ids(&["ls", "--by", "author", "--to", "author", "--json"]).is_empty());
}

#[test]
fn ls_json_rows_carry_the_record_and_the_state() {
    let scratch = Scratch::new();
    let id = scratch.promise("ship it", "author", "publisher", PAST);

    let run = scratch.run(&["ls", "--json"]);
    run.expect(0);
    let rows = run.json();
    let row = &rows.as_array().expect("an array")[0];
    assert_eq!(row["id"], id.as_str());
    assert_eq!(row["promise"], "ship it");
    assert_eq!(row["by"], "author");
    assert_eq!(row["to"], "publisher");
    assert_eq!(row["until"], PAST);
    assert_eq!(row["state"], "overdue");
}

#[test]
fn ls_human_output_is_plain_ascii_columns() {
    let scratch = Scratch::new();
    let id = scratch.promise("ship it", "author", "publisher", FUTURE);

    let run = scratch.run(&["ls"]);
    run.expect(0);
    assert!(
        run.stdout.is_ascii(),
        "the table must stay ASCII — non-ASCII width breaks alignment over SSH: {:?}",
        run.stdout
    );
    assert!(run.stdout.contains(&id));
    assert!(
        run.stdout.contains("author -> publisher"),
        "{:?}",
        run.stdout
    );
    assert!(run.stdout.contains("detached"));
}

#[test]
fn ls_on_an_empty_ledger_says_so_on_stderr() {
    let scratch = Scratch::new();

    let run = scratch.run(&["ls"]);
    // Nobody having promised anything is a legitimate state, not an error — but
    // silence would be ambiguous, so the note goes to stderr and stdout stays empty.
    run.expect(0);
    assert!(run.stdout.is_empty(), "no table for an empty ledger");
    assert!(
        run.stderr.contains("holds no promises"),
        "the note should say the ledger is empty: {:?}",
        run.stderr
    );
}

#[test]
fn ls_says_how_many_the_ledger_holds_when_a_filter_hides_everything() {
    let scratch = Scratch::new();
    scratch.promise("ship it", "author", "publisher", FUTURE);

    let run = scratch.run(&["ls", "--by", "nobody"]);
    run.expect(0);
    assert!(run.stdout.is_empty(), "no table when nothing matched");
    // An empty ledger and a filter that hid everything must not read alike.
    assert!(
        run.stderr.contains("no promises match"),
        "the note should distinguish a filter from an empty ledger: {:?}",
        run.stderr
    );
    assert!(
        run.stderr.contains("the ledger holds 1"),
        "the note should say how much was hidden: {:?}",
        run.stderr
    );
}

// -------------------------------------------------------------------- show

#[test]
fn show_json_carries_state_resolution_and_assessments() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    scratch
        .run(&[
            "resolve",
            &id,
            "--satisfied",
            "--evidence",
            "https://example.org/schemas/draft-3.json",
            "--evidence",
            "sha:9c1f0e",
        ])
        .expect(0);
    scratch
        .run(&["assess", &id, "--violated", "--observer", "reviewer"])
        .expect(0);

    let run = scratch.run(&["show", &id, "--json"]);
    run.expect(0);
    let detail = run.json();

    assert_eq!(detail["id"], id.as_str());
    assert_eq!(detail["promise"], "send the draft schema");
    assert_eq!(detail["until"], FUTURE);
    assert_eq!(detail["state"], "satisfied");

    let resolution = &detail["resolution"];
    assert_eq!(resolution["as"], "satisfied");
    assert_eq!(resolution["by"], "author");
    assert!(resolution["ts"].is_string());
    assert_eq!(
        resolution["evidence"].as_array().expect("evidence").len(),
        2
    );

    let assessments = detail["assessments"].as_array().expect("assessments");
    assert_eq!(assessments.len(), 1);
    assert_eq!(assessments[0]["state"], "violated");
    assert_eq!(assessments[0]["observer"], "reviewer");
    assert!(assessments[0]["ts"].is_string());
}

#[test]
fn show_reports_no_resolution_as_null() {
    let scratch = Scratch::new();
    let id = scratch.promise("ship it", "author", "publisher", FUTURE);
    let run = scratch.run(&["show", &id, "--json"]);
    run.expect(0);
    assert_eq!(run.json()["resolution"], serde_json::Value::Null);
    assert!(run.json()["assessments"]
        .as_array()
        .expect("array")
        .is_empty());
}

#[test]
fn show_json_reports_a_cancelled_resolution_with_its_reason() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    scratch
        .run(&[
            "resolve",
            &id,
            "--cancelled",
            "--reason",
            "upstream schema was withdrawn",
        ])
        .expect(0);

    let run = scratch.run(&["show", &id, "--json"]);
    run.expect(0);
    let resolution = &run.json()["resolution"];
    assert_eq!(resolution["as"], "cancelled");
    assert_eq!(resolution["by"], "author");
    assert_eq!(resolution["reason"], "upstream schema was withdrawn");
    assert!(
        resolution.get("evidence").is_none(),
        "a cancelled promise was not done, so it points at nothing: {resolution}"
    );
}

#[test]
fn show_json_reports_a_released_resolution_with_its_reason() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    scratch
        .run(&["resolve", &id, "--released", "--reason", "no longer needed"])
        .expect(0);

    let run = scratch.run(&["show", &id, "--json"]);
    run.expect(0);
    let resolution = &run.json()["resolution"];
    assert_eq!(resolution["as"], "released");
    // Releasing is the creditor's act (§4).
    assert_eq!(resolution["by"], "reviewer");
    assert_eq!(resolution["reason"], "no longer needed");
    assert!(
        resolution.get("evidence").is_none(),
        "releasing is not a claim that the work was done: {resolution}"
    );
}

#[test]
fn show_json_omits_the_reason_a_release_did_not_give() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    scratch.run(&["resolve", &id, "--released"]).expect(0);

    let run = scratch.run(&["show", &id, "--json"]);
    run.expect(0);
    let resolution = &run.json()["resolution"];
    assert_eq!(resolution["as"], "released");
    assert!(
        resolution.get("reason").is_none(),
        "a reason is encouraged, not required — an absent one is absent: {resolution}"
    );
}

#[test]
fn show_prints_a_plain_text_block_for_an_unresolved_promise() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);

    let run = scratch.run(&["show", &id]);
    run.expect(0);
    assert!(
        run.stdout.starts_with(&format!("{id}  detached\n")),
        "the id and the computed state head the block: {:?}",
        run.stdout
    );
    assert!(
        run.stdout.contains("promise      send the draft schema\n"),
        "{:?}",
        run.stdout
    );
    assert!(
        run.stdout.contains("by           author\n"),
        "{:?}",
        run.stdout
    );
    assert!(
        run.stdout.contains("to           reviewer\n"),
        "{:?}",
        run.stdout
    );
    assert!(
        run.stdout.contains(&format!("until        {FUTURE}\n")),
        "{:?}",
        run.stdout
    );
    assert!(
        run.stdout.contains("resolution   (unresolved)\n"),
        "{:?}",
        run.stdout
    );
    assert!(
        run.stdout.contains("assessments  (none)\n"),
        "{:?}",
        run.stdout
    );
}

#[test]
fn show_omits_the_antecedent_and_task_lines_when_the_record_has_neither() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);

    let run = scratch.run(&["show", &id]);
    run.expect(0);
    assert!(
        !run.stdout.contains("  on   "),
        "an absent antecedent gets no line: {:?}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("  task "),
        "an absent task gets no line: {:?}",
        run.stdout
    );
}

#[test]
fn show_prints_the_antecedent_and_the_task_when_the_record_has_them() {
    let scratch = Scratch::new();
    let a = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    let b_run = scratch.run(&[
        "promise",
        "hand back a review",
        "--by",
        "reviewer",
        "--to",
        "author",
        "--until",
        FUTURE,
        "--on",
        &a,
        "--task",
        "a2a-task-9c1f0e",
    ]);
    b_run.expect(0);

    let run = scratch.run(&["show", &b_run.id()]);
    run.expect(0);
    assert!(
        run.stdout.contains(&format!("on           {a}\n")),
        "{:?}",
        run.stdout
    );
    assert!(
        run.stdout.contains("task         a2a-task-9c1f0e\n"),
        "{:?}",
        run.stdout
    );
}

#[test]
fn show_prints_every_evidence_reference_under_the_resolution() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    scratch
        .run(&[
            "resolve",
            &id,
            "--satisfied",
            "--evidence",
            "https://example.org/schemas/draft-3.json",
            "--evidence",
            "sha:9c1f0e",
        ])
        .expect(0);

    let run = scratch.run(&["show", &id]);
    run.expect(0);
    assert!(
        run.stdout.contains("resolution   satisfied at "),
        "{:?}",
        run.stdout
    );
    assert!(
        run.stdout.contains(" by author\n"),
        "the block names who claimed it: {:?}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("evidence   https://example.org/schemas/draft-3.json\n"),
        "{:?}",
        run.stdout
    );
    assert!(
        run.stdout.contains("evidence   sha:9c1f0e\n"),
        "every reference gets its own line: {:?}",
        run.stdout
    );
}

#[test]
fn show_prints_the_reason_a_promise_was_cancelled() {
    let scratch = Scratch::new();
    let id = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    scratch
        .run(&[
            "resolve",
            &id,
            "--cancelled",
            "--reason",
            "upstream schema was withdrawn",
        ])
        .expect(0);

    let run = scratch.run(&["show", &id]);
    run.expect(0);
    assert!(
        run.stdout.contains("resolution   cancelled at "),
        "{:?}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("reason     upstream schema was withdrawn\n"),
        "{:?}",
        run.stdout
    );
}

#[test]
fn show_lists_an_assessment_with_its_note() {
    let scratch = Scratch::new();
    let id = scratch.promise("ship it", "author", "publisher", PAST);
    scratch
        .run(&[
            "assess",
            &id,
            "--violated",
            "--observer",
            "publisher",
            "--note",
            "nothing shipped, no reason given",
        ])
        .expect(0);

    let run = scratch.run(&["show", &id]);
    run.expect(0);
    assert!(
        run.stdout.contains("assessments\n"),
        "the heading replaces the (none) line: {:?}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("violated  publisher  nothing shipped, no reason given\n"),
        "{:?}",
        run.stdout
    );
}

#[test]
fn show_lists_an_assessment_that_carries_no_note() {
    let scratch = Scratch::new();
    let id = scratch.promise("ship it", "author", "publisher", PAST);
    scratch
        .run(&["assess", &id, "--violated", "--observer", "publisher"])
        .expect(0);

    let run = scratch.run(&["show", &id]);
    run.expect(0);
    assert!(
        run.stdout.contains("violated  publisher\n"),
        "an absent note leaves the line ending at the observer: {:?}",
        run.stdout
    );
}

#[test]
fn showing_an_unknown_id_fails_operationally() {
    let scratch = Scratch::new();
    scratch.run(&["show", "pnk_000000"]).expect(1);
}

#[test]
fn showing_an_id_that_could_never_have_been_declared_says_so() {
    let scratch = Scratch::new();
    // `pnk_` is reserved, and this is not a minted id, so no route could have put it
    // in the ledger. That is a typo, not a lookup miss, and the hint says which.
    let run = scratch.run(&["show", "pnk_bogus"]);
    run.expect(1);
    assert!(
        run.stderr.contains("could never have been declared"),
        "{:?}",
        run.stderr
    );
    assert!(run.stderr.contains("reserved"), "{:?}", run.stderr);
}

#[test]
fn showing_an_unknown_foreign_id_is_a_plain_lookup_miss() {
    let scratch = Scratch::new();
    // Since §1 admits a caller's own id, "this does not look like a pinki id" is no
    // longer evidence of anything — `bogus` is a perfectly declarable id, so the only
    // true thing to say is that nothing declares it.
    let run = scratch.run(&["show", "bogus"]);
    run.expect(1);
    assert!(run.stderr.contains("nothing in"), "{:?}", run.stderr);
    assert!(
        !run.stderr.contains("could never"),
        "no hint is owed here: {:?}",
        run.stderr
    );
}

// ------------------------------------------------------------ the ledger file

#[test]
fn a_ledger_path_that_is_a_directory_fails_operationally() {
    let scratch = Scratch::new();
    // Not "missing" — an absent ledger is an empty one. This is a path that exists
    // and cannot be read, which is a real failure and must say so.
    fs::create_dir_all(scratch.ledger()).expect("a directory where the ledger should be");

    let run = scratch.run(&["ls"]);
    run.expect(1);
    assert!(
        run.stderr.contains("ledger.jsonl"),
        "the error should name the file it could not read: {:?}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "errors never go to stdout");
}

#[test]
fn a_bare_ledger_filename_lands_beside_the_working_directory() {
    let scratch = Scratch::new();

    // `PINKI_LEDGER=ledger.jsonl` names a file with no directory component. There is
    // no parent to create, and pinki must not invent one.
    let run = scratch.run_with_bare_ledger(
        "ledger.jsonl",
        &[
            "promise",
            "ship it",
            "--by",
            "author",
            "--to",
            "publisher",
            "--until",
            FUTURE,
        ],
    );
    run.expect(0);

    let written = scratch.dir.join("ledger.jsonl");
    assert!(
        written.exists(),
        "the ledger should be {}",
        written.display()
    );
    assert!(
        !scratch.dir.join(".pinki").exists(),
        "no directory should have been created"
    );
}

// ---------------------------------------------------------------- the graph

#[test]
fn a_dependent_detaches_when_its_antecedent_is_satisfied() {
    let scratch = Scratch::new();
    // DESIGN.md §2's graph, two links of it, end to end.
    let a = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    let b_run = scratch.run(&[
        "promise",
        "hand back a review",
        "--by",
        "reviewer",
        "--to",
        "author",
        "--until",
        FUTURE,
        "--on",
        &a,
    ]);
    b_run.expect(0);
    let b = b_run.id();

    assert_eq!(b_run.state(), "conditional");
    assert_eq!(state_of(&scratch, &b), "conditional");

    scratch
        .run(&[
            "resolve",
            &a,
            "--satisfied",
            "--evidence",
            "https://example.org/schemas/draft-3.json",
        ])
        .expect(0);

    // "When X is satisfied, the dependent detaches and the clock that matters starts
    // mattering."
    assert_eq!(state_of(&scratch, &a), "satisfied");
    assert_eq!(state_of(&scratch, &b), "detached");
}

// ----------------------------------------------------------------- a2a purity

#[test]
fn a2a_card_is_json_on_stdout_and_silence_on_stderr() {
    let scratch = Scratch::new();
    let run = scratch.run(&["a2a", "card"]);
    run.expect(0);
    assert_eq!(run.stderr, "", "the a2a verbs write nothing to stderr");

    let card = run.json();
    let extensions = card["capabilities"]["extensions"]
        .as_array()
        .expect("one extensions array");
    assert_eq!(extensions.len(), 1);
    assert_eq!(
        extensions[0]["uri"],
        "https://github.com/azigler/pinki/ext/promise/v0"
    );
    // A2A-EXTENSION.md §Surface 1: `required` is false, always.
    assert_eq!(extensions[0]["required"], serde_json::json!(false));
    assert_eq!(extensions[0]["params"], serde_json::json!({}));
    assert!(extensions[0]["description"].is_string());
}

#[test]
fn a2a_task_emits_the_metadata_block_for_one_promise() {
    let scratch = Scratch::new();
    let a = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    let b_run = scratch.run(&[
        "promise",
        "hand back a review",
        "--by",
        "reviewer",
        "--to",
        "author",
        "--until",
        FUTURE,
        "--on",
        &a,
        "--task",
        "a2a-task-9c1f0e",
    ]);
    b_run.expect(0);
    let b = b_run.id();

    let run = scratch.run(&["a2a", "task", &b]);
    run.expect(0);
    assert_eq!(run.stderr, "", "the a2a verbs write nothing to stderr");

    let block = run.json();
    let object = block.as_object().expect("an object");
    assert_eq!(object.len(), 1, "one key, one value");
    let (key, record) = object.iter().next().expect("the one key");
    assert_eq!(
        key,
        "https://github.com/azigler/pinki/ext/promise/v0/promise"
    );

    // The record round-trips: it is §1's record verbatim, with no computed state.
    assert_eq!(record["id"], b.as_str());
    assert_eq!(record["promise"], "hand back a review");
    assert_eq!(record["by"], "reviewer");
    assert_eq!(record["to"], "author");
    assert_eq!(record["on"], a.as_str());
    assert_eq!(record["until"], FUTURE);
    assert_eq!(record["task"], "a2a-task-9c1f0e");
    assert!(
        record.get("state").is_none(),
        "A2A-EXTENSION.md §2: no computed state rides the block"
    );
    assert_eq!(record.as_object().expect("record object").len(), 7);
}

#[test]
fn a2a_task_for_an_unknown_id_fails_operationally() {
    let scratch = Scratch::new();
    let run = scratch.run(&["a2a", "task", "pnk_000000"]);
    run.expect(1);
    assert!(run.stdout.is_empty(), "no half-written JSON on failure");
}

// -------------------------------------------------------------------- meta
//
// §1's one open door: an opaque object pinki carries and never reads. These drive the
// real binary against a real file, because the property being claimed is about what
// survives a write and a read — which a test that never leaves memory cannot see.

/// Provenance shaped like something a real adopter would carry, written with its keys
/// already in sorted order so a byte-for-byte claim is a claim about bytes.
const META: &str = r#"{"attempt":2,"by":"scheduler","policy":{"escalate":["t-24h","t-2h"],"kind":"nudge"},"session":"s-91"}"#;

/// The `meta` a surface reported, re-encoded compactly so two surfaces can be compared
/// byte for byte regardless of how each one printed it.
fn meta_bytes(value: &serde_json::Value) -> String {
    serde_json::to_string(&value["meta"]).expect("re-encode meta")
}

/// The same value with every `meta` key removed, however deep it sits.
///
/// This is what makes "the fold ignores `meta`" checkable against a whole read surface
/// rather than against the four states alone: strip the provenance back off a `show
/// --json` taken over a ledger with `meta` on every line, and what is left has to be
/// byte-identical to the one taken before any of it was there.
fn without_meta(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .iter()
                .filter(|(key, _)| key.as_str() != "meta")
                .map(|(key, value)| (key.clone(), without_meta(value)))
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(without_meta).collect())
        }
        other => other.clone(),
    }
}

#[test]
fn meta_from_the_flag_rides_the_promise_line_verbatim() {
    let scratch = Scratch::new();
    let run = scratch.run(&[
        "promise",
        "ship it",
        "--by",
        "author",
        "--to",
        "publisher",
        "--until",
        FUTURE,
        "--meta",
        META,
    ]);
    run.expect(0);

    let event = &scratch.events()[0];
    assert_eq!(
        serde_json::to_string(&event["meta"]).unwrap(),
        META,
        "meta must land on the line exactly as it was given"
    );
    // And it is last on the line, after the seven arithmetic fields.
    assert!(scratch.lines()[0].ends_with(&format!(r#""meta":{META}}}"#)));
}

#[test]
fn meta_from_a_stdin_record_rides_the_promise_line_verbatim() {
    let scratch = Scratch::new();
    // The integration seam issue #6 is about: an existing obligation row, piped in
    // whole, with its provenance still attached.
    let record = format!(
        r#"{{"promise":"ship it","by":"author","to":"publisher","until":"{FUTURE}","meta":{META}}}"#
    );
    let run = scratch.feed(&["promise"], &record);
    run.expect(0);

    let event = &scratch.events()[0];
    assert_eq!(serde_json::to_string(&event["meta"]).unwrap(), META);
}

#[test]
fn a_promise_with_no_meta_writes_no_meta_key() {
    let scratch = Scratch::new();
    scratch.promise("ship it", "author", "publisher", FUTURE);
    let line = &scratch.lines()[0];
    assert!(
        !line.contains("meta"),
        "an absent meta is absent, not an empty object: {line}"
    );

    let show = scratch.run(&["show", scratch.events()[0]["id"].as_str().unwrap()]);
    show.expect(0);
    assert!(
        !show.stdout.contains("meta"),
        "and the human block has no meta line: {:?}",
        show.stdout
    );
}

#[test]
fn meta_that_is_not_an_object_is_refused_and_the_ledger_is_byte_identical() {
    let scratch = Scratch::new();
    scratch.promise("something first", "author", "publisher", FUTURE);
    let before = fs::read(scratch.ledger()).expect("read the ledger");

    for bad in ["null", "true", "3", r#""a string""#, r#"[{"by":"x"}]"#] {
        let run = scratch.run(&[
            "promise",
            "ship it",
            "--by",
            "author",
            "--to",
            "publisher",
            "--until",
            FUTURE,
            "--meta",
            bad,
        ]);
        run.expect(2);
        assert!(run.stderr.contains("§1"), "{bad}: {:?}", run.stderr);
        assert!(run.stdout.is_empty(), "errors never go to stdout");
        assert_eq!(
            fs::read(scratch.ledger()).expect("read the ledger"),
            before,
            "the ledger must be byte-identical after refusing {bad}"
        );
    }

    // The same rule, in the same words, on the stdin path.
    let record = format!(
        r#"{{"promise":"ship it","by":"author","to":"publisher","until":"{FUTURE}","meta":[1,2]}}"#
    );
    let run = scratch.feed(&["promise"], &record);
    run.expect(2);
    assert!(run.stderr.contains("an array"), "{:?}", run.stderr);
    assert_eq!(fs::read(scratch.ledger()).expect("read"), before);

    // And an empty object, which is meta offered and left blank.
    let run = scratch.run(&[
        "promise",
        "ship it",
        "--by",
        "author",
        "--to",
        "publisher",
        "--until",
        FUTURE,
        "--meta",
        "{}",
    ]);
    run.expect(2);
    assert!(run.stderr.contains("empty object"), "{:?}", run.stderr);
    assert_eq!(fs::read(scratch.ledger()).expect("read"), before);
}

#[test]
fn meta_is_validated_before_the_ledger_is_touched_at_all() {
    let scratch = Scratch::new();
    assert!(!scratch.ledger().exists());

    // No ledger file yet: a malformed meta must still be refused, and must not bring
    // the file into existence on its way to failing.
    scratch
        .run(&[
            "promise",
            "ship it",
            "--by",
            "author",
            "--to",
            "publisher",
            "--until",
            FUTURE,
            "--meta",
            "not json at all",
        ])
        .expect(2);
    assert!(
        !scratch.ledger().exists(),
        "validation precedes I/O — nothing should have been created"
    );
}

#[test]
fn meta_over_the_cap_is_refused_and_just_under_it_is_not() {
    let scratch = Scratch::new();
    let before = scratch.lines();

    let promise_with = |meta: &str| {
        scratch.run(&[
            "promise",
            "ship it",
            "--by",
            "author",
            "--to",
            "publisher",
            "--until",
            FUTURE,
            "--meta",
            meta,
        ])
    };

    // 8 KiB is the cap; `{"pad":"…"}` costs 11 bytes around the padding.
    let over = format!(r#"{{"pad":"{}"}}"#, "x".repeat(8 * 1024));
    let run = promise_with(&over);
    run.expect(2);
    assert!(run.stderr.contains("8192"), "{:?}", run.stderr);
    assert_eq!(scratch.lines(), before, "nothing may be appended");

    let under = format!(r#"{{"pad":"{}"}}"#, "x".repeat(8 * 1024 - 11));
    promise_with(&under).expect(0);
    assert_eq!(scratch.lines().len(), 1, "just under the cap is accepted");
}

#[test]
fn meta_and_a_record_on_stdin_may_not_both_be_given() {
    let scratch = Scratch::new();
    let record =
        format!(r#"{{"promise":"ship it","by":"author","to":"publisher","until":"{FUTURE}"}}"#);
    let run = scratch.feed(&["promise", "--meta", META], &record);
    // Silently dropping the flag is the defect this whole feature exists to close, so
    // the ambiguity is refused rather than resolved by guessing.
    run.expect(2);
    assert!(run.stderr.contains("--meta"), "{:?}", run.stderr);
    assert!(scratch.lines().is_empty(), "nothing may be appended");
}

#[test]
fn nested_meta_round_trips_through_every_read_surface_byte_for_byte() {
    let scratch = Scratch::new();
    let run = scratch.run(&[
        "promise",
        "ship it",
        "--by",
        "author",
        "--to",
        "publisher",
        "--until",
        FUTURE,
        "--task",
        "a2a-task-9c1f0e",
        "--meta",
        META,
    ]);
    run.expect(0);
    let id = run.id();

    // 1. the log
    let from_log = serde_json::to_string(&scratch.events()[0]["meta"]).unwrap();

    // 2. show --json
    let show = scratch.run(&["show", &id, "--json"]);
    show.expect(0);
    let from_show = meta_bytes(&show.json());

    // 3. ls --json
    let ls = scratch.run(&["ls", "--json"]);
    ls.expect(0);
    let rows = ls.json();
    let from_ls = meta_bytes(&rows.as_array().expect("an array")[0]);

    // 4. a2a task — the record verbatim inside the extension block
    let a2a = scratch.run(&["a2a", "task", &id]);
    a2a.expect(0);
    let block = a2a.json();
    let record = &block["https://github.com/azigler/pinki/ext/promise/v0/promise"];
    let from_a2a = meta_bytes(record);

    for (surface, got) in [
        ("the log", &from_log),
        ("show --json", &from_show),
        ("ls --json", &from_ls),
        ("a2a task", &from_a2a),
    ] {
        assert_eq!(got.as_str(), META, "{surface} did not carry meta verbatim");
    }

    // The A2A block still carries the record and nothing else — meta is part of the
    // record, not a second key beside it, and no computed state came along.
    assert_eq!(block.as_object().expect("an object").len(), 1);
    assert!(record.get("state").is_none());
    // id, promise, by, to, until, task, meta — this promise has no antecedent.
    assert_eq!(record.as_object().expect("the record").len(), 7);
}

#[test]
fn resolve_and_assess_carry_their_own_meta() {
    let scratch = Scratch::new();
    let id = scratch.promise("ship it", "author", "publisher", PAST);

    let resolve_meta = r#"{"by":"expected-gap-watchdog","cites_ref":"sha:9c1f0e"}"#;
    scratch
        .run(&[
            "assess",
            &id,
            "--violated",
            "--observer",
            "publisher",
            "--meta",
            r#"{"by":"offboard","window":"w-12"}"#,
        ])
        .expect(0);
    scratch
        .run(&[
            "resolve",
            &id,
            "--satisfied",
            "--evidence",
            "https://example.org/x",
            "--meta",
            resolve_meta,
        ])
        .expect(0);

    let events = scratch.events();
    assert_eq!(events[1]["type"], "assess");
    assert_eq!(events[2]["type"], "resolve");
    assert_eq!(
        serde_json::to_string(&events[2]["meta"]).unwrap(),
        resolve_meta
    );
    // Last on the line there too, after the outcome's own payload.
    assert!(scratch.lines()[2].ends_with(&format!(r#""meta":{resolve_meta}}}"#)));

    let show = scratch.run(&["show", &id, "--json"]);
    show.expect(0);
    let detail = show.json();
    assert!(
        detail.get("meta").is_none(),
        "the record itself carried none: {detail}"
    );
    assert_eq!(
        serde_json::to_string(&detail["resolution"]["meta"]).unwrap(),
        resolve_meta
    );
    assert_eq!(detail["assessments"][0]["meta"]["by"], "offboard");

    // And the human block shows both, one line each, without pretending to know what
    // any of the keys mean.
    let text = scratch.run(&["show", &id]);
    text.expect(0);
    assert!(
        text.stdout
            .contains(&format!("    meta       {resolve_meta}\n")),
        "{:?}",
        text.stdout
    );
    assert!(
        text.stdout
            .contains("      meta     {\"by\":\"offboard\",\"window\":\"w-12\"}\n"),
        "{:?}",
        text.stdout
    );
}

#[test]
fn nested_meta_on_an_amend_round_trips_through_the_line_and_show_byte_for_byte() {
    let scratch = Scratch::new();
    let id = scratch.promise("hand back the report", "watchdog", "desk", PAST);

    scratch
        .run(&["amend", &id, "--until", FUTURE, "--reason", "nudge 1"])
        .expect(0);
    let nudge = scratch.run(&[
        "amend", &id, "--until", FUTURE, "--reason", "nudge 2", "--meta", META,
    ]);
    nudge.expect(0);

    // 1. the log — last on the line, after the reason, and byte for byte.
    let lines = scratch.lines();
    assert!(
        lines[2].ends_with(&format!(r#""meta":{META}}}"#)),
        "{:?}",
        lines[2]
    );
    let events = scratch.events();
    assert_eq!(events[2]["type"], "amend");
    assert_eq!(
        serde_json::to_string(&events[2]["meta"]).unwrap(),
        META,
        "the ledger line did not carry meta verbatim"
    );
    // The rung that said nothing writes no key at all — an absent provenance is not an
    // empty one.
    assert!(
        !events[1].contains_key("meta"),
        "an amend without --meta writes no meta key: {:?}",
        lines[1]
    );

    // 2. show --json, under the horizon the amend spoke.
    let show = scratch.run(&["show", &id, "--json"]);
    show.expect(0);
    let detail = show.json();
    let horizons = detail["horizons"].as_array().expect("horizons");
    assert_eq!(horizons.len(), 3, "{horizons:?}");
    assert!(horizons[0].get("meta").is_none(), "{}", horizons[0]);
    assert!(horizons[1].get("meta").is_none(), "{}", horizons[1]);
    assert_eq!(meta_bytes(&horizons[2]), META, "show --json lost meta");
    assert!(
        detail.get("meta").is_none(),
        "the record itself carried none: {detail}"
    );

    // 3. the human block — one line, under the horizon it belongs to, in the same
    // shape an assessment's provenance is printed in.
    let text = scratch.run(&["show", &id]);
    text.expect(0);
    assert!(
        text.stdout.contains(&format!(
            "  {FUTURE}  watchdog  nudge 2\n      meta     {META}\n"
        )),
        "{:?}",
        text.stdout
    );
}

#[test]
fn an_amends_meta_is_refused_on_the_same_three_rules_as_every_other() {
    let scratch = Scratch::new();
    let id = scratch.promise("hand back the report", "watchdog", "desk", PAST);
    let before = scratch.lines();

    for (meta, expected) in [
        ("[]", "an array"),
        ("{}", "empty object"),
        ("not json at all", "--meta is not JSON"),
    ] {
        let run = scratch.run(&["amend", &id, "--until", FUTURE, "--meta", meta]);
        run.expect(2);
        assert!(run.stderr.contains(expected), "{:?}", run.stderr);
        assert_eq!(scratch.lines(), before, "nothing may be appended");
    }

    // And the shape is checked before the ledger is consulted: an unknown id with a
    // malformed meta is malformed, not a lookup miss.
    let run = scratch.run(&["amend", "pnk_000000", "--until", FUTURE, "--meta", "[]"]);
    run.expect(2);
    assert_eq!(scratch.lines(), before);
}

#[test]
fn show_renders_the_records_meta_on_one_line_when_it_has_one() {
    let scratch = Scratch::new();
    let run = scratch.run(&[
        "promise",
        "ship it",
        "--by",
        "author",
        "--to",
        "publisher",
        "--until",
        FUTURE,
        "--meta",
        META,
    ]);
    run.expect(0);

    let show = scratch.run(&["show", &run.id()]);
    show.expect(0);
    assert!(
        show.stdout.contains(&format!("  meta         {META}\n")),
        "one line, as JSON, under the record's own fields: {:?}",
        show.stdout
    );
}

#[test]
fn the_fold_ignores_meta_on_every_event() {
    let scratch = Scratch::new();
    // A ledger with something in every state the fold computes: an antecedent that is
    // satisfied, a dependent that detaches, one overdue, one assessed, one cancelled.
    let a = scratch.promise("send the draft schema", "author", "reviewer", FUTURE);
    let b = scratch
        .run(&[
            "promise",
            "hand back a review",
            "--by",
            "reviewer",
            "--to",
            "author",
            "--until",
            FUTURE,
            "--on",
            &a,
        ])
        .id();
    let late = scratch.promise("ship it", "author", "publisher", PAST);
    let doomed = scratch.promise("index it", "publisher", "author", FUTURE);
    // One moved horizon, so `amend` is in the ledger this property is claimed over —
    // onto a rung that is also in the past, which keeps the state it computes stable
    // and the claim about `meta` the only thing under test.
    scratch
        .run(&[
            "amend",
            &late,
            "--until",
            PAST_PLUS_15,
            "--reason",
            "nudge 1",
        ])
        .expect(0);
    scratch
        .run(&[
            "resolve",
            &a,
            "--satisfied",
            "--evidence",
            "https://example.org/x",
        ])
        .expect(0);
    scratch
        .run(&["resolve", &doomed, "--cancelled", "--reason", "withdrawn"])
        .expect(0);
    scratch
        .run(&["assess", &late, "--violated", "--observer", "publisher"])
        .expect(0);

    let states = |scratch: &Scratch| -> Vec<(String, String)> {
        let run = scratch.run(&["ls", "--all", "--json"]);
        run.expect(0);
        run.json()
            .as_array()
            .expect("an array")
            .iter()
            .map(|row| {
                (
                    row["id"].as_str().expect("an id").to_string(),
                    row["state"].as_str().expect("a state").to_string(),
                )
            })
            .collect()
    };
    let detail_of = |scratch: &Scratch, id: &str| -> serde_json::Value {
        let run = scratch.run(&["show", id, "--json"]);
        run.expect(0);
        run.json()
    };

    let before = states(&scratch);
    // The baseline is only worth as much as the states it actually covers.
    assert_eq!(
        before,
        vec![
            (a, "satisfied".to_string()),
            (b, "detached".to_string()),
            (late.clone(), "overdue".to_string()),
            (doomed, "cancelled".to_string()),
        ]
    );
    let details_before: Vec<serde_json::Value> = before
        .iter()
        .map(|(id, _)| detail_of(&scratch, id))
        .collect();
    // Every event type is in the file, including the one this ledger exists to add.
    let types: Vec<String> = scratch
        .events()
        .iter()
        .map(|event| event["type"].as_str().expect("a type").to_string())
        .collect();
    for kind in ["promise", "amend", "resolve", "assess"] {
        assert!(types.iter().any(|t| t == kind), "no {kind} in {types:?}");
    }

    // Now put a different meta on every single event — including keys that look like
    // fields the fold does read.
    scratch.rewrite(|n, event| {
        let meta = serde_json::json!({
            "n": n,
            "until": "1999-01-01T00:00:00Z",
            "on": "pnk_000000",
            "state": "satisfied",
            "as": "released",
            "nested": {"deep": [n, {"deeper": true}]},
        });
        event.insert("meta".to_string(), meta);
    });

    let after = states(&scratch);
    assert_eq!(
        before, after,
        "no meta anywhere in a ledger may change a computed state"
    );

    // The same for the per-promise read: state, resolution, horizons and assessments
    // are untouched, and the meta rides along beside them. Strip the provenance back
    // off and every byte of the read surface is what it was before any of it existed.
    for ((id, _), was) in before.iter().zip(&details_before) {
        let detail = detail_of(&scratch, id);
        assert!(detail["meta"].is_object(), "{id}: {detail}");
        assert_eq!(detail["meta"]["state"], "satisfied");
        assert_eq!(
            detail["state"],
            before
                .iter()
                .find(|(other, _)| other == id)
                .map(|(_, state)| state.as_str())
                .expect("a state")
        );
        assert_eq!(
            serde_json::to_string(&without_meta(&detail)).unwrap(),
            serde_json::to_string(was).unwrap(),
            "{id}: meta changed a read surface it may only ride on"
        );
    }

    // The amended one, specifically: the horizon still reads off the event's own
    // `until`, not off the `until` key sitting inside its `meta` — the whole reason
    // that key is in the rewrite.
    let amended = detail_of(&scratch, &late);
    assert_eq!(amended["until"], PAST_PLUS_15);
    let horizons = amended["horizons"].as_array().expect("horizons");
    assert_eq!(horizons.len(), 2, "{horizons:?}");
    assert_eq!(horizons[1]["until"], PAST_PLUS_15);
    assert_eq!(horizons[1]["reason"], "nudge 1");
    assert_eq!(horizons[1]["meta"]["until"], "1999-01-01T00:00:00Z");
    // The declaration's horizon carries no meta of its own, whatever the `promise`
    // line holds: the record's provenance is handed back with the record.
    assert!(
        horizons[0].get("meta").is_none(),
        "the declared horizon speaks no provenance: {}",
        horizons[0]
    );
}

// ------------------------------------------------------------ what stays uncovered
//
// `cargo llvm-cov --summary-only --show-missing-lines` names eleven source lines that
// no test executes. Each is listed here with the reason, because an unexplained gap and
// a deliberate one look identical in a coverage report — and the deliberate ones should
// stay deliberate rather than be closed by a test that asserts nothing.
//
// Unreachable defensive code — the arm exists so the fold cannot panic on a ledger
// somebody else wrote, and the surrounding code makes it unconstructible:
//
//   src/state.rs 328   `_ => State::Conditional` when a declared id has no memo entry.
//                      Every id in `order` is solved before this runs, and `solve`
//                      only ever terminates with `Memo::Done`.
//   src/state.rs 383-385  the dangling arm in `solve`, for an id with no record.
//                      `solve` is called only for ids in `records`, and an antecedent
//                      is `contains_key`-checked before it is pushed on the stack, so
//                      `self.records.get(current)` is always `Some`.
//   src/state.rs 396   `None => State::Detached` for a resolution that is not one.
//                      Only `EventBody::Resolve` events enter `resolutions`, and
//                      `Event::resolution()` returns `Some` for exactly those.
//   src/verbs.rs 768   `_ => None` over `view.assessments`, which `fold` fills only
//                      from `EventBody::Assess` events.
//
// Unreachable from a test harness:
//
//   src/verbs.rs 267   the `io::stdin().is_terminal()` refusal. A child spawned by a
//                      test never has a tty on stdin, and giving it one would mean a
//                      pty dependency — which Cargo.toml exists to refuse. Exercised
//                      by hand: `pinki promise` at a prompt.
//
// Test-internal — the failure arm of an assertion, which by construction does not run
// while the suite is green:
//
//   src/event.rs 528      `panic!` in `every_event_type_round_trips`.
//   src/event.rs 720      `panic!` in `meta_survives_a_round_trip_on_every_event_type`.
//   src/ledger.rs 301,314 `other => panic!("expected Malformed, …")`.
//
// One more thing a reader should not have to rediscover: the summary's "Missed Lines"
// column is larger than this list (36 against 11). The difference is not a set of
// hidden gaps — no source line accounts for it. pinki is built twice under coverage,
// once as the binary the tests here drive and once as the unit-test harness, and a
// function present in both but exercised in only one is billed as missed lines against
// the copy that never ran it. `--show-missing-lines` and the annotated `--text` report
// both merge the two and agree on the eleven lines above.
