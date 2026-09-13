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
            child
                .stdin
                .as_mut()
                .expect("stdin pipe")
                .write_all(text.as_bytes())
                .expect("write stdin");
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

#[test]
fn an_explicit_id_that_is_not_well_formed_is_a_usage_error() {
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
        "promise-1",
    ]);
    run.expect(2);
    assert!(
        run.stderr.contains("six lowercase hex digits"),
        "the error should say what a pinki id looks like: {:?}",
        run.stderr
    );
    assert!(scratch.lines().is_empty(), "nothing may be appended");
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
fn showing_an_id_that_is_not_even_well_formed_says_so() {
    let scratch = Scratch::new();
    let run = scratch.run(&["show", "bogus"]);
    run.expect(1);
    assert!(
        run.stderr.contains("not even a well-formed pinki id"),
        "an id that could never have been minted earns a hint: {:?}",
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

    // The same for the per-promise read: state, resolution and assessments are
    // untouched, and the meta rides along beside them.
    for (id, _) in &before {
        let run = scratch.run(&["show", id, "--json"]);
        run.expect(0);
        let detail = run.json();
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
    }
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
//   src/state.rs 206   `_ => State::Conditional` when a declared id has no memo entry.
//                      Every id in `order` is solved before this runs, and `solve`
//                      only ever terminates with `Memo::Done`.
//   src/state.rs 257-259  the dangling arm in `solve`, for an id with no record.
//                      `solve` is called only for ids in `records`, and an antecedent
//                      is `contains_key`-checked before it is pushed on the stack, so
//                      `self.records.get(current)` is always `Some`.
//   src/state.rs 270   `None => State::Detached` for a resolution that is not one.
//                      Only `EventBody::Resolve` events enter `resolutions`, and
//                      `Event::resolution()` returns `Some` for exactly those.
//   src/verbs.rs 636   `_ => None` over `view.assessments`, which `fold` fills only
//                      from `EventBody::Assess` events.
//
// Unreachable from a test harness:
//
//   src/verbs.rs 253   the `io::stdin().is_terminal()` refusal. A child spawned by a
//                      test never has a tty on stdin, and giving it one would mean a
//                      pty dependency — which Cargo.toml exists to refuse. Exercised
//                      by hand: `pinki promise` at a prompt.
//
// Test-internal — the failure arm of an assertion, which by construction does not run
// while the suite is green:
//
//   src/event.rs 397      `panic!` in `every_event_type_round_trips`.
//   src/event.rs 517      `panic!` in `meta_survives_a_round_trip_on_every_event_type`.
//   src/ledger.rs 280,293 `other => panic!("expected Malformed, …")`.
//
// One more thing a reader should not have to rediscover: the summary's "Missed Lines"
// column is larger than this list (39 against 11). The difference is not a set of
// hidden gaps — no source line accounts for it. pinki is built twice under coverage,
// once as the binary the tests here drive and once as the unit-test harness, and a
// function present in both but exercised in only one is billed as missed lines against
// the copy that never ran it. `--show-missing-lines` and the annotated `--text` report
// both merge the two and agree on the eleven lines above.
