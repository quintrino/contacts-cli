//! End-to-end tests that run the built binary against an isolated data
//! directory. None of them reach the point of launching another app: the
//! `message` tests only cover paths that stop before the URI is opened.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A unique scratch directory under the OS temp dir, removed on drop.
struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "contacts-cli-e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn data_home(&self) -> PathBuf {
        self.0.join("data")
    }

    fn contacts_json(&self) -> PathBuf {
        self.data_home().join("contacts-cli").join("contacts.json")
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_contacts-cli"))
            .args(args)
            .env("XDG_DATA_HOME", self.data_home())
            .env("NO_COLOR", "1")
            .env_remove("CLICOLOR_FORCE")
            .output()
            .unwrap()
    }

    fn import(&self, csv: &str) -> Output {
        let file = self.write("contacts.csv", csv);
        self.run(&["import", path_str(&file)])
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const CSV: &str = "\
First Name,Middle Name,Last Name,Organization Name,E-mail 1 - Value,Phone 1 - Value
Alice,,Smith,Acme Corp,alice@acme.com,+61 412 345 678
Bob,,Jones,,,
,,,Orphans Pty Ltd,,
,,,,,
";

#[test]
fn import_stores_contacts_and_skips_nameless_rows() {
    let sandbox = Sandbox::new();

    let output = sandbox.import(CSV);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("3 contact(s) imported"), "{}", stdout(&output));
    let stored: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(sandbox.contacts_json()).unwrap()).unwrap();
    let names: Vec<&str> = stored
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Alice Smith", "Bob Jones", "Orphans Pty Ltd"]);
}

#[test]
fn first_import_reports_everything_as_new() {
    let sandbox = Sandbox::new();

    let output = sandbox.import(CSV);

    assert!(stdout(&output).contains("(3 new, 0 updated)"), "{}", stdout(&output));
}

#[test]
fn reimporting_updates_existing_contacts_instead_of_duplicating_them() {
    let sandbox = Sandbox::new();
    sandbox.import(CSV);

    let output = sandbox.import(
        "First Name,Last Name,Phone 1 - Value\n\
         alice,SMITH,+61 499 999 999\n\
         Carol,White,+61 400 111 222\n",
    );

    assert!(stdout(&output).contains("(1 new, 1 updated)"), "{}", stdout(&output));
    let stored = fs::read_to_string(sandbox.contacts_json()).unwrap();
    let contacts: Vec<serde_json::Value> = serde_json::from_str(&stored).unwrap();
    // Alice replaced (matched case-insensitively), Bob and Orphans kept, Carol added.
    assert_eq!(contacts.len(), 4);
    let alice = contacts.iter().find(|c| c["name"] == "alice SMITH").unwrap();
    assert_eq!(alice["phones"][0], "+61499999999");
}

#[test]
fn import_of_a_missing_file_fails_with_a_message() {
    let sandbox = Sandbox::new();

    let output = sandbox.run(&["import", "/definitely/not/here.csv"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("Cannot open CSV file"), "{}", stderr(&output));
}

#[test]
fn import_of_a_file_with_no_contacts_says_so_and_stores_nothing() {
    let sandbox = Sandbox::new();

    let output = sandbox.import("First Name,Last Name\n");

    assert!(output.status.success());
    assert!(stdout(&output).contains("No contacts found"), "{}", stdout(&output));
    assert!(!sandbox.contacts_json().exists());
}

#[test]
fn search_finds_contacts_by_name_email_phone_and_organisation() {
    let sandbox = Sandbox::new();
    sandbox.import(CSV);

    for query in ["smith", "ALICE@ACME", "412 345", "acme corp"] {
        let output = sandbox.run(&["search", query]);

        assert!(output.status.success(), "{query}: {}", stderr(&output));
        assert!(stdout(&output).contains("Alice Smith"), "{query}: {}", stdout(&output));
        assert!(!stdout(&output).contains("Bob Jones"), "{query}: {}", stdout(&output));
    }
}

#[test]
fn search_prints_every_match() {
    let sandbox = Sandbox::new();
    sandbox.import(CSV);

    let output = sandbox.run(&["search", "o"]);

    let text = stdout(&output);
    assert!(text.contains("Bob Jones") && text.contains("Alice Smith"), "{text}");
    assert!(text.contains("3 result(s)"), "{text}");
}

#[test]
fn search_with_no_match_says_so_and_succeeds() {
    let sandbox = Sandbox::new();
    sandbox.import(CSV);

    let output = sandbox.run(&["search", "zzz"]);

    assert!(output.status.success());
    assert!(stdout(&output).contains("No contacts matching 'zzz'"), "{}", stdout(&output));
}

#[test]
fn search_before_any_import_finds_nothing() {
    let sandbox = Sandbox::new();

    let output = sandbox.run(&["search", "alice"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("No contacts matching"), "{}", stdout(&output));
}

#[test]
fn search_with_a_corrupt_contacts_file_is_an_error() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.contacts_json().parent().unwrap()).unwrap();
    fs::write(sandbox.contacts_json(), "not json").unwrap();

    let output = sandbox.run(&["search", "alice"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("Cannot parse contacts file"), "{}", stderr(&output));
}

#[test]
fn message_without_a_transport_lists_the_options() {
    let sandbox = Sandbox::new();
    sandbox.import(CSV);

    let output = sandbox.run(&["message", "alice"]);

    assert!(output.status.success());
    let text = stdout(&output);
    assert!(text.contains("No messaging transport specified"), "{text}");
    for flag in ["--signal", "--whatsapp", "--telegram", "--imessage", "--sms", "--facetime", "--tel", "--email"] {
        assert!(text.contains(flag), "{flag} missing from: {text}");
    }
}

#[test]
fn message_with_no_matching_contact_says_so() {
    let sandbox = Sandbox::new();
    sandbox.import(CSV);

    let output = sandbox.run(&["message", "--signal", "zzz"]);

    assert!(output.status.success());
    assert!(stdout(&output).contains("No contacts matching 'zzz'"), "{}", stdout(&output));
}

#[test]
fn phone_transports_fail_for_a_contact_with_no_phone() {
    let sandbox = Sandbox::new();
    sandbox.import(CSV);

    for flag in ["--signal", "--whatsapp", "--telegram", "--imessage", "--sms", "--facetime", "--tel"] {
        let output = sandbox.run(&["message", flag, "bob"]);

        assert_eq!(output.status.code(), Some(1), "{flag}");
        let err = stderr(&output);
        assert!(err.contains("Bob Jones") && err.contains("no phone number"), "{flag}: {err}");
    }
}

#[test]
fn email_transport_fails_for_a_contact_with_no_email() {
    let sandbox = Sandbox::new();
    sandbox.import(CSV);

    let output = sandbox.run(&["message", "--email", "bob"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("no email address"), "{}", stderr(&output));
}

#[test]
fn only_one_transport_flag_is_accepted() {
    let sandbox = Sandbox::new();
    sandbox.import(CSV);

    let output = sandbox.run(&["message", "--signal", "--sms", "alice"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("cannot be used with"), "{}", stderr(&output));
}

#[test]
fn version_flag_reports_the_package_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_contacts-cli"))
        .arg("--version")
        .output()
        .unwrap();

    assert_eq!(
        stdout(&output).trim(),
        format!("contacts-cli {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn import_saves_a_doubled_mmi_prefixed_number_as_one_clean_number() {
    let sandbox = Sandbox::new();

    let output = sandbox.import(
        "First Name,Last Name,Phone 1 - Value\n\
         Alice,Smith,*31#+61 491 570 006 ::: *31#+61 491 570 006\n",
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let stored = fs::read_to_string(sandbox.contacts_json()).unwrap();
    let contacts: Vec<serde_json::Value> = serde_json::from_str(&stored).unwrap();
    assert_eq!(contacts[0]["phones"], serde_json::json!(["+61491570006"]));
}

#[test]
fn imported_numbers_can_be_found_by_a_spaced_search() {
    let sandbox = Sandbox::new();
    sandbox.import(
        "First Name,Last Name,Phone 1 - Value\n\
         Alice,Smith,*31#+61 491 570 006 ::: *31#+61 491 570 006\n",
    );

    let output = sandbox.run(&["search", "+61 491 570"]);

    assert!(stdout(&output).contains("Alice Smith"), "{}", stdout(&output));
    assert!(stdout(&output).contains("+61491570006"), "{}", stdout(&output));
    assert!(!stdout(&output).contains("*31#"), "{}", stdout(&output));
}

// ── search --format json ────────────────────────────────────────────────────

fn json_results(output: &Output) -> Vec<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("not JSON ({e}): {}", stdout(output)));
    value.as_array().expect("a JSON array").clone()
}

const JSON_CSV: &str = "\
First Name,Last Name,Organization Name,E-mail 1 - Value,Phone 1 - Value,Phone 2 - Value
Alice,Smith,Acme Corp,alice@acme.com,+61 491 570 006,+61 400 000 000
Bob,Jones,,bob@example.com,,
Carol,White,,,,
";

#[test]
fn json_format_prints_an_array_of_the_matching_contacts() {
    let sandbox = Sandbox::new();
    sandbox.import(JSON_CSV);

    let output = sandbox.run(&["search", "--format", "json", "alice"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        json_results(&output),
        [serde_json::json!({
            "name": "Alice Smith",
            "emails": ["alice@acme.com"],
            "phones": ["+61491570006", "+61400000000"],
            "organization": "Acme Corp",
            "notes": null
        })]
    );
}

#[test]
fn json_format_prints_only_json_on_stdout() {
    let sandbox = Sandbox::new();
    sandbox.import(JSON_CSV);

    let output = sandbox.run(&["search", "--format", "json", "alice"]);

    let text = stdout(&output);
    assert!(text.trim_start().starts_with('['), "{text}");
    assert!(!text.contains("result(s)") && !text.contains('\u{1b}'), "{text}");
}

#[test]
fn json_format_keeps_contacts_without_a_phone_or_an_email() {
    let sandbox = Sandbox::new();
    sandbox.import(JSON_CSV);

    let output = sandbox.run(&["search", "--format", "json", "bob"]);
    let bob = &json_results(&output)[0];
    assert_eq!(bob["phones"], serde_json::json!([]));
    assert_eq!(bob["emails"], serde_json::json!(["bob@example.com"]));

    let output = sandbox.run(&["search", "--format", "json", "carol"]);
    let carol = &json_results(&output)[0];
    assert_eq!(carol["phones"], serde_json::json!([]));
    assert_eq!(carol["emails"], serde_json::json!([]));
}

#[test]
fn json_format_with_an_empty_query_lists_every_contact() {
    let sandbox = Sandbox::new();
    sandbox.import(JSON_CSV);

    let output = sandbox.run(&["search", "--format", "json", ""]);

    let names: Vec<String> = json_results(&output)
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["Alice Smith", "Bob Jones", "Carol White"]);
}

#[test]
fn json_format_with_no_match_is_an_empty_array_and_succeeds() {
    let sandbox = Sandbox::new();
    sandbox.import(JSON_CSV);

    let output = sandbox.run(&["search", "--format", "json", "zzz"]);

    assert!(output.status.success());
    assert!(json_results(&output).is_empty());
}

#[test]
fn json_format_before_any_import_is_an_empty_array() {
    let sandbox = Sandbox::new();

    let output = sandbox.run(&["search", "--format", "json", "alice"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(json_results(&output).is_empty());
}

#[test]
fn json_format_reports_a_corrupt_contacts_file_on_stderr_with_a_failing_exit() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.contacts_json().parent().unwrap()).unwrap();
    fs::write(sandbox.contacts_json(), "not json").unwrap();

    let output = sandbox.run(&["search", "--format", "json", "alice"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("Cannot parse contacts file"), "{}", stderr(&output));
    assert!(stdout(&output).trim().is_empty(), "{}", stdout(&output));
}

#[test]
fn json_format_matches_numbers_typed_with_spaces() {
    let sandbox = Sandbox::new();
    sandbox.import(JSON_CSV);

    let output = sandbox.run(&["search", "--format", "json", "+61 491 570"]);

    assert_eq!(json_results(&output)[0]["name"], "Alice Smith");
}

#[test]
fn a_query_that_looks_like_a_flag_can_follow_double_dash() {
    let sandbox = Sandbox::new();
    sandbox.import(JSON_CSV);

    let output = sandbox.run(&["search", "--format", "json", "--", "--help"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(json_results(&output).is_empty());
}

#[test]
fn the_text_format_is_still_the_default() {
    let sandbox = Sandbox::new();
    sandbox.import(JSON_CSV);

    let output = sandbox.run(&["search", "alice"]);

    assert!(stdout(&output).contains("1 result(s)"), "{}", stdout(&output));
}

#[test]
fn an_unknown_format_is_rejected() {
    let sandbox = Sandbox::new();

    let output = sandbox.run(&["search", "--format", "xml", "alice"]);

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn the_old_message_subtitle_flag_is_gone() {
    let sandbox = Sandbox::new();

    let output = sandbox.run(&["search", "--format", "json", "--message-subtitle", "x", "alice"]);

    assert_eq!(output.status.code(), Some(2));
}
