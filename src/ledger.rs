//! The ledger file — DESIGN.md §4, §5.
//!
//! Append-only JSONL at `PINKI_LEDGER`, default `./.pinki/ledger.jsonl`. "The log is
//! the whole system. Copy the file and you have copied the state." So this module is
//! deliberately small: find the file, read every line, append one line.
//!
//! Two behaviours are load-bearing:
//!
//! - **A missing file is an empty ledger, not an error.** Nobody has promised
//!   anything yet; that is a legitimate state and the first `promise` will create the
//!   file.
//! - **A malformed line is loud.** §4 notes that truncating the log is lying to
//!   yourself — silently skipping a line you could not parse is the same lie told one
//!   line at a time, and it would make the fold quietly wrong. The error names the
//!   file and the 1-based line number so you can go and look at it.
//! - **An event of an unknown `type` is loud but survivable.** That is a different
//!   thing from a line that is not an event: it is a well-formed event this build has
//!   not heard of, almost certainly written by a newer pinki. Refusing the whole file
//!   over it would mean any future event type breaks every older reader on a ledger it
//!   is otherwise perfectly able to read — so the line is kept as
//!   [`crate::event::EventBody::Unknown`], warned about on stderr with its line number,
//!   and ignored by the fold. Loud, not fatal, and never silent.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::event::{Event, EventBody};

/// The environment variable that relocates the ledger.
pub const ENV_VAR: &str = "PINKI_LEDGER";

/// Where the ledger lives when `PINKI_LEDGER` says nothing.
pub const DEFAULT_PATH: &str = "./.pinki/ledger.jsonl";

/// Anything that can go wrong touching the ledger.
#[derive(Debug)]
pub enum Error {
    /// The file could not be read or written.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A line was not valid JSON, or was not a pinki event.
    Malformed {
        path: PathBuf,
        line: usize,
        source: serde_json::Error,
    },
    /// An event could not be turned into JSON. Practically unreachable; kept so the
    /// append path has no `unwrap`.
    Encode { source: serde_json::Error },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => {
                write!(f, "ledger {}: {source}", path.display())
            }
            Error::Malformed { path, line, source } => write!(
                f,
                "ledger {}: line {line} is not a valid pinki event: {source}",
                path.display()
            ),
            Error::Encode { source } => write!(f, "could not encode event as JSON: {source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            Error::Malformed { source, .. } | Error::Encode { source } => Some(source),
        }
    }
}

/// The ledger path for this process.
pub fn path() -> PathBuf {
    resolve_path(std::env::var_os(ENV_VAR))
}

/// The path rule, pulled out of the environment so it can be tested without mutating
/// process-global state from a threaded test harness.
fn resolve_path(from_env: Option<OsString>) -> PathBuf {
    match from_env {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(DEFAULT_PATH),
    }
}

/// Read the whole ledger from the configured path.
pub fn read() -> Result<Vec<Event>, Error> {
    read_at(&path())
}

/// Read the whole ledger from an explicit path.
pub fn read_at(path: &Path) -> Result<Vec<Event>, Error> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        // No file means nobody has promised anything yet.
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(Error::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    };

    let mut events = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Event>(line) {
            Ok(event) => {
                // Warned per line, with its number, because "which line was that?" is
                // the only question a reader has here — and because a ledger a newer
                // pinki wrote should say so once per surprising line rather than once
                // per file.
                if event.body == EventBody::Unknown {
                    eprintln!(
                        "pinki: warning: {} line {}: unknown event type, skipped — written by a \
                         newer pinki? The fold ignores it (§4); nothing was lost from the file",
                        path.display(),
                        index + 1
                    );
                }
                events.push(event);
            }
            Err(source) => {
                return Err(Error::Malformed {
                    path: path.to_path_buf(),
                    line: index + 1,
                    source,
                })
            }
        }
    }
    Ok(events)
}

/// Append one event to the configured ledger, creating it (and its directory) if this
/// is the first promise anyone has made here.
pub fn append(event: &Event) -> Result<(), Error> {
    append_at(&path(), event)
}

/// Append one event to an explicit path.
pub fn append_at(path: &Path, event: &Event) -> Result<(), Error> {
    let mut line = serde_json::to_string(event).map_err(|source| Error::Encode { source })?;
    line.push('\n');

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;

    file.write_all(line.as_bytes()).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventBody;
    use crate::record::Promise;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A scratch directory that removes itself. No `tempfile` dependency — see
    /// Cargo.toml on why the dependency list stays this short.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("pinki-test-{}-{tag}-{n}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn promise_event(id: &str) -> Event {
        Event::new(
            "2026-08-28T20:14:03Z",
            EventBody::promise(Promise {
                id: id.into(),
                promise: "hand back a reviewed schema".into(),
                by: "…/reviewer".into(),
                to: "…/author".into(),
                on: None,
                until: "2026-09-01T17:00:00Z".into(),
                task: None,
            }),
        )
    }

    #[test]
    fn default_path_when_env_is_unset_or_empty() {
        assert_eq!(resolve_path(None), PathBuf::from(DEFAULT_PATH));
        assert_eq!(
            resolve_path(Some(OsString::from(""))),
            PathBuf::from(DEFAULT_PATH)
        );
    }

    #[test]
    fn env_var_relocates_the_ledger() {
        assert_eq!(
            resolve_path(Some(OsString::from("/tmp/elsewhere.jsonl"))),
            PathBuf::from("/tmp/elsewhere.jsonl")
        );
    }

    #[test]
    fn missing_file_is_an_empty_ledger_not_an_error() {
        let scratch = Scratch::new("missing");
        let events = read_at(&scratch.join("nope/ledger.jsonl")).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn append_creates_parents_then_reads_back() {
        let scratch = Scratch::new("append");
        let path = scratch.join("deep/nested/.pinki/ledger.jsonl");

        append_at(&path, &promise_event("pnk_000001")).unwrap();
        append_at(&path, &promise_event("pnk_000002")).unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        assert_eq!(raw.lines().count(), 2, "one compact line per event: {raw}");
        assert!(raw.ends_with('\n'), "every line is newline-terminated");
        assert!(
            !raw.contains("\n  "),
            "lines are compact, not pretty: {raw}"
        );

        let events = read_at(&path).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].subject(), "pnk_000001");
        assert_eq!(events[1].subject(), "pnk_000002");
    }

    #[test]
    fn blank_and_whitespace_lines_are_skipped() {
        let scratch = Scratch::new("blank");
        let path = scratch.join("ledger.jsonl");
        let line = serde_json::to_string(&promise_event("pnk_000001")).unwrap();
        fs::write(&path, format!("\n{line}\n   \n\t\n{line}\n\n")).unwrap();

        let events = read_at(&path).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn malformed_line_is_loud_and_names_file_and_line_number() {
        let scratch = Scratch::new("malformed");
        let path = scratch.join("ledger.jsonl");
        let good = serde_json::to_string(&promise_event("pnk_000001")).unwrap();
        fs::write(&path, format!("{good}\n{{not json\n{good}\n")).unwrap();

        let err = read_at(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("line 2"), "{msg}");
        assert!(msg.contains("ledger.jsonl"), "{msg}");
        match err {
            Error::Malformed { line, .. } => assert_eq!(line, 2),
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn blank_lines_do_not_shift_the_reported_line_number() {
        let scratch = Scratch::new("lineno");
        let path = scratch.join("ledger.jsonl");
        let good = serde_json::to_string(&promise_event("pnk_000001")).unwrap();
        fs::write(&path, format!("\n\n{good}\n\n{{ nope\n")).unwrap();

        match read_at(&path).unwrap_err() {
            Error::Malformed { line, .. } => assert_eq!(line, 5),
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn a_parent_that_is_already_a_file_is_an_io_error_naming_it() {
        let scratch = Scratch::new("parentfile");
        let blocker = scratch.join("blocker");
        fs::write(&blocker, "").unwrap();

        // `.pinki/` cannot be created under something that is not a directory.
        let err =
            append_at(&blocker.join("ledger.jsonl"), &promise_event("pnk_000001")).unwrap_err();
        assert!(err.to_string().contains("blocker"), "{err}");
        assert!(
            matches!(&err, Error::Io { path, .. } if path == &blocker),
            "the error should name the directory it could not create: {err:?}"
        );
    }

    #[test]
    fn a_ledger_path_that_is_a_directory_cannot_be_opened_for_append() {
        let scratch = Scratch::new("appenddir");
        let path = scratch.join("ledger.jsonl");
        fs::create_dir_all(&path).unwrap();

        let err = append_at(&path, &promise_event("pnk_000001")).unwrap_err();
        assert!(err.to_string().contains("ledger.jsonl"), "{err}");
        assert!(matches!(err, Error::Io { .. }), "{err:?}");
    }

    /// `/dev/full` accepts the open and refuses the bytes, which is the only way to
    /// reach a failing write without filling a real disk. Linux-only because the
    /// device is: elsewhere there is nothing to ask.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_write_that_cannot_land_is_an_io_error() {
        let err = append_at(Path::new("/dev/full"), &promise_event("pnk_000001")).unwrap_err();
        assert!(err.to_string().contains("/dev/full"), "{err}");
        assert!(matches!(err, Error::Io { .. }), "{err:?}");
    }

    #[test]
    fn a_ledger_path_with_no_parent_creates_no_directory() {
        // A path with no parent at all. Nothing to create, so append goes straight to
        // the open — which `/` refuses, proving the directory step was skipped.
        let err = append_at(Path::new("/"), &promise_event("pnk_000001")).unwrap_err();
        assert!(matches!(err, Error::Io { .. }), "{err:?}");
    }

    #[test]
    fn an_io_error_names_the_path_it_was_touching() {
        let err = Error::Io {
            path: PathBuf::from("/tmp/ledger.jsonl"),
            source: std::io::Error::new(ErrorKind::PermissionDenied, "denied"),
        };
        assert_eq!(err.to_string(), "ledger /tmp/ledger.jsonl: denied");
    }

    #[test]
    fn an_encode_error_says_the_event_could_not_be_encoded() {
        let err = Error::Encode {
            source: not_an_event(),
        };
        assert!(
            err.to_string()
                .starts_with("could not encode event as JSON: "),
            "{err}"
        );
    }

    #[test]
    fn every_error_variant_hands_back_its_source() {
        use std::error::Error as _;

        let io = Error::Io {
            path: PathBuf::from("/tmp/ledger.jsonl"),
            source: std::io::Error::new(ErrorKind::PermissionDenied, "denied"),
        };
        let malformed = Error::Malformed {
            path: PathBuf::from("/tmp/ledger.jsonl"),
            line: 2,
            source: not_an_event(),
        };
        let encode = Error::Encode {
            source: not_an_event(),
        };

        assert!(io.source().is_some(), "an Io error keeps the io::Error");
        assert!(
            malformed.source().is_some(),
            "a Malformed error keeps the parse failure"
        );
        assert!(
            encode.source().is_some(),
            "an Encode error keeps the serde failure"
        );
    }

    /// A `serde_json::Error`, for the variants that carry one. There is no constructor
    /// for these, so one is made the only way it can be: by failing a parse.
    fn not_an_event() -> serde_json::Error {
        serde_json::from_str::<Event>("{}").unwrap_err()
    }

    #[test]
    fn a_json_line_that_is_not_a_pinki_event_is_also_malformed() {
        let scratch = Scratch::new("notevent");
        let path = scratch.join("ledger.jsonl");
        // No `ts`, and a `promise` body missing every field it needs. This is not an
        // event of an unknown type; it is not an event.
        fs::write(&path, "{\"type\":\"promise\"}\n").unwrap();
        assert!(matches!(
            read_at(&path).unwrap_err(),
            Error::Malformed { line: 1, .. }
        ));
    }

    #[test]
    fn an_unknown_event_type_is_kept_and_costs_nothing_else_in_the_file() {
        let scratch = Scratch::new("unknowntype");
        let path = scratch.join("ledger.jsonl");
        let good = serde_json::to_string(&promise_event("pnk_000001")).unwrap();
        // A line a newer pinki might write. An older reader must still be able to read
        // the promise above and below it — refusing the whole file would make every
        // future event type a breaking change for every older reader.
        fs::write(
            &path,
            format!(
                "{good}\n{{\"ts\":\"2026-08-28T20:14:03Z\",\"type\":\"frobnicate\",\"promise\":\"pnk_000001\",\"wat\":1}}\n{good}\n"
            ),
        )
        .unwrap();

        let events = read_at(&path).unwrap();
        assert_eq!(events.len(), 3, "every line is read");
        assert_eq!(events[1].body, EventBody::Unknown);
        // It claims no subject, so it can never be mistaken for an event about a
        // promise — including by the id minter, which asks the log what is spoken for.
        assert_eq!(events[1].subject(), "");
        assert_eq!(events[1].ts, "2026-08-28T20:14:03Z");
    }
}
