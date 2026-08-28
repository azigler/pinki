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
        self.spawn(args, None)
    }

    fn feed(&self, args: &[&str], stdin: &str) -> Run {
        self.spawn(args, Some(stdin))
    }

    fn spawn(&self, args: &[&str], stdin: Option<&str>) -> Run {
        let mut child = Command::new(BIN)
            .args(args)
            .env("PINKI_LEDGER", self.ledger())
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
fn showing_an_unknown_id_fails_operationally() {
    let scratch = Scratch::new();
    scratch.run(&["show", "pnk_000000"]).expect(1);
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
