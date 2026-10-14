use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

// ─── CLI ────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "contacts-cli",
    about = "A fast terminal contact manager",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Import contacts from a Google Contacts CSV file
    Import {
        /// Path to the CSV file exported from Google Contacts
        file: PathBuf,
    },
    /// Search contacts by name, email, or phone
    Search {
        /// Query string to search for
        query: String,
    },
    /// Open a messaging app for the top search result
    Message {
        /// Query string to find contact
        query: String,
        #[command(flatten)]
        transport: TransportFlags,
    },
}

/// Which app or URI scheme to open. At most one may be given.
#[derive(Args)]
#[group(multiple = false)]
struct TransportFlags {
    /// Open in Signal (sgnl://signal.me/#p/<number>)
    #[arg(long)]
    signal: bool,
    /// Open in WhatsApp (whatsapp://send?phone=<number>)
    #[arg(long)]
    whatsapp: bool,
    /// Open in Telegram (tg://resolve?phone=<number>)
    #[arg(long)]
    telegram: bool,
    /// Open in Messages as an iMessage (imessage://<number>)
    #[arg(long)]
    imessage: bool,
    /// Open a text message in Messages (sms:<number>)
    #[arg(long)]
    sms: bool,
    /// Start a FaceTime call (facetime://<number>)
    #[arg(long)]
    facetime: bool,
    /// Start a phone call (tel:<number>)
    #[arg(long)]
    tel: bool,
    /// Write an email to the first address on record (mailto:<address>)
    #[arg(long)]
    email: bool,
}

impl TransportFlags {
    fn selected(&self) -> Option<Transport> {
        [
            (self.signal, Transport::Signal),
            (self.whatsapp, Transport::WhatsApp),
            (self.telegram, Transport::Telegram),
            (self.imessage, Transport::IMessage),
            (self.sms, Transport::Sms),
            (self.facetime, Transport::FaceTime),
            (self.tel, Transport::Tel),
            (self.email, Transport::Email),
        ]
        .into_iter()
        .find(|(on, _)| *on)
        .map(|(_, transport)| transport)
    }
}

// ─── DATA MODEL ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Contact {
    name: String,
    emails: Vec<String>,
    phones: Vec<String>,
    organization: Option<String>,
    notes: Option<String>,
}

impl Contact {
    fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        if self.name.to_lowercase().contains(&q) {
            return true;
        }
        if self.emails.iter().any(|e| e.to_lowercase().contains(&q)) {
            return true;
        }
        if self.phones.iter().any(|p| p.to_lowercase().contains(&q)) {
            return true;
        }
        if let Some(org) = &self.organization {
            if org.to_lowercase().contains(&q) {
                return true;
            }
        }
        false
    }

    fn display(&self) {
        println!("{}", self.name.bold().cyan());
        if let Some(org) = &self.organization {
            if !org.is_empty() {
                println!("  {} {}", "Org:".dimmed(), org.yellow());
            }
        }
        for email in &self.emails {
            println!("  {} {}", "✉".dimmed(),  email);
        }
        for phone in &self.phones {
            println!("  {} {}", "📱".dimmed(), phone);
        }
        println!();
    }

    /// The first phone number on record, cleaned for use in a URI
    fn first_phone(&self) -> Option<String> {
        self.phones.first().map(|p| clean_phone(p))
    }

    fn first_email(&self) -> Option<&str> {
        self.emails.first().map(String::as_str)
    }
}

/// Keep digits and a leading `+`, dropping spaces, dashes, dots and brackets,
/// so "+61 (412) 345-678" becomes "+61412345678".
fn clean_phone(raw: &str) -> String {
    let raw = raw.trim();
    let plus = if raw.starts_with('+') { "+" } else { "" };
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    format!("{plus}{digits}")
}

// ─── GOOGLE CONTACTS CSV PARSING ────────────────────────────────────────────

/// Real Google Contacts CSV (as exported today) uses columns:
///   First Name, Middle Name, Last Name, Organization Name,
///   E-mail 1 - Label, E-mail 1 - Value, E-mail 2 - Label, E-mail 2 - Value, ...
///   Phone 1 - Label, Phone 1 - Value, Phone 2 - Label, Phone 2 - Value, ...
///
/// The "Name" column does NOT exist; name is assembled from First/Last.
/// Organization is "Organization Name" (not "Organization 1 - Name").
/// Email values are real addresses; the label columns (e.g. "* Other") are skipped.
/// Some rows have no name and belong to Google internal entries — those are skipped.
fn parse_google_csv(path: &PathBuf) -> Result<Vec<Contact>> {
    let mut rdr = csv::Reader::from_path(path)
        .with_context(|| format!("Cannot open CSV file: {}", path.display()))?;

    // Collect headers, stripping a UTF-8 BOM if present on the first header
    let headers: Vec<String> = rdr
        .headers()
        .context("Cannot read CSV headers")?
        .iter()
        .enumerate()
        .map(|(i, h)| {
            if i == 0 {
                h.trim_start_matches('\u{feff}').trim().to_string()
            } else {
                h.trim().to_string()
            }
        })
        .collect();

    let mut contacts = Vec::new();

    for result in rdr.records() {
        let record = result.context("Error reading CSV row")?;

        // Build header→value map; duplicate headers are last-wins (rare in practice)
        let row: HashMap<&str, &str> = headers
            .iter()
            .enumerate()
            .filter_map(|(i, h)| record.get(i).map(|v| (h.as_str(), v)))
            .collect();

        // ── Name ──────────────────────────────────────────────────────────────
        // Real export: "First Name" / "Middle Name" / "Last Name"
        // Also accept legacy "Given Name" / "Family Name" and "Name" for safety.
        let name = {
            let first  = row.get("First Name").or_else(|| row.get("Given Name"))
                            .copied().unwrap_or("").trim();
            let middle = row.get("Middle Name").or_else(|| row.get("Additional Name"))
                            .copied().unwrap_or("").trim();
            let last   = row.get("Last Name").or_else(|| row.get("Family Name"))
                            .copied().unwrap_or("").trim();

            let parts: Vec<&str> = [first, middle, last]
                .iter().copied().filter(|s| !s.is_empty()).collect();

            if parts.is_empty() {
                // Fall back to a single "Name" column if present
                row.get("Name").copied().unwrap_or("").trim().to_string()
            } else {
                parts.join(" ")
            }
        };

        // For org-only rows (no first/last name), use the organisation name
        // as the contact name, but skip truly nameless internal Google rows.
        let name = if name.is_empty() {
            let org = row
                .get("Organization Name")
                .or_else(|| row.get("Organization 1 - Name"))
                .copied()
                .unwrap_or("")
                .trim()
                .to_string();
            if org.is_empty() {
                continue;
            }
            org
        } else {
            name
        };

        // ── Phones ────────────────────────────────────────────────────────────
        // Columns: "Phone 1 - Value", "Phone 2 - Value", … up to however many exist
        let phones: Vec<String> = (1..=20)
            .filter_map(|i| {
                let key = format!("Phone {} - Value", i);
                row.get(key.as_str())
                    .copied()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(String::from)
            })
            .collect();

        // ── Emails ────────────────────────────────────────────────────────────
        // Columns: "E-mail 1 - Value", "E-mail 2 - Value", …
        // Skip values that look like Google calendar IDs (contain "@group.calendar")
        let emails: Vec<String> = (1..=20)
            .filter_map(|i| {
                let key = format!("E-mail {} - Value", i);
                row.get(key.as_str())
                    .copied()
                    .map(str::trim)
                    .filter(|v| !v.is_empty() && !v.contains("@group.calendar") && !v.contains("@group.v.calendar"))
                    .map(String::from)
            })
            .collect();

        // ── Organisation ──────────────────────────────────────────────────────
        // Real export uses "Organization Name"; old export used "Organization 1 - Name"
        let organization = row
            .get("Organization Name")
            .or_else(|| row.get("Organization 1 - Name"))
            .or_else(|| row.get("Organization"))
            .copied()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(String::from);

        // ── Notes ─────────────────────────────────────────────────────────────
        let notes = row
            .get("Notes")
            .copied()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(String::from);

        contacts.push(Contact {
            name,
            emails,
            phones,
            organization,
            notes,
        });
    }

    Ok(contacts)
}

// ─── STORAGE ────────────────────────────────────────────────────────────────

fn data_dir() -> Result<PathBuf> {
    // Respect $XDG_DATA_HOME, fall back to ~/.local/share
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .expect("Cannot determine home directory")
                .join(".local/share")
        });
    let dir = base.join("contacts-cli");
    std::fs::create_dir_all(&dir).context("Cannot create data directory")?;
    Ok(dir)
}

fn contacts_file() -> Result<PathBuf> {
    Ok(data_dir()?.join("contacts.json"))
}

fn load_contacts() -> Result<Vec<Contact>> {
    let path = contacts_file()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = std::fs::read_to_string(&path)
        .with_context(|| format!("Cannot read contacts file: {}", path.display()))?;
    serde_json::from_str(&data).context("Cannot parse contacts file")
}

fn save_contacts(contacts: &[Contact]) -> Result<()> {
    let path = contacts_file()?;
    let data = serde_json::to_string_pretty(contacts).context("Cannot serialise contacts")?;
    std::fs::write(&path, data)
        .with_context(|| format!("Cannot write contacts file: {}", path.display()))?;
    Ok(())
}

// ─── COMMANDS ───────────────────────────────────────────────────────────────

fn cmd_import(file: &PathBuf) -> Result<()> {
    println!("{} {}", "Importing".bold(), file.display().to_string().cyan());

    let new_contacts = parse_google_csv(file)?;
    if new_contacts.is_empty() {
        println!("{}", "No contacts found in file.".yellow());
        return Ok(());
    }

    // Merge: keep existing contacts not in import, then add all new ones.
    // Deduplicate by name (case-insensitive).
    let mut existing = load_contacts()?;
    let new_names: std::collections::HashSet<String> = new_contacts
        .iter()
        .map(|c| c.name.to_lowercase())
        .collect();

    let total_before = existing.len();
    existing.retain(|c| !new_names.contains(&c.name.to_lowercase()));
    let updated = (total_before - existing.len()).min(new_contacts.len());
    let added = new_contacts.len() - updated;
    existing.extend(new_contacts.iter().cloned());
    existing.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    save_contacts(&existing)?;

    let dir = data_dir()?;
    println!(
        "{} {} contact(s) imported  ({} new, {} updated)",
        "✓".green().bold(),
        new_contacts.len(),
        added.to_string().green(),
        updated.to_string().yellow()
    );
    println!("  Stored in: {}", dir.display().to_string().dimmed());
    Ok(())
}

fn cmd_search(query: &str) -> Result<Vec<Contact>> {
    let contacts = load_contacts()?;
    let results: Vec<Contact> = contacts
        .into_iter()
        .filter(|c| c.matches(query))
        .collect();

    if results.is_empty() {
        println!("{}", format!("No contacts matching '{}'", query).yellow());
    } else {
        println!(
            "{} {} result(s) for '{}'\n",
            "→".cyan().bold(),
            results.len(),
            query.bold()
        );
        for contact in &results {
            contact.display();
        }
    }

    Ok(results)
}

// ─── MESSAGING TRANSPORTS ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transport {
    Signal,
    WhatsApp,
    Telegram,
    IMessage,
    Sms,
    FaceTime,
    Tel,
    Email,
}

impl Transport {
    fn label(self) -> &'static str {
        match self {
            Transport::Signal => "Signal",
            Transport::WhatsApp => "WhatsApp",
            Transport::Telegram => "Telegram",
            Transport::IMessage => "iMessage",
            Transport::Sms => "SMS",
            Transport::FaceTime => "FaceTime",
            Transport::Tel => "phone call",
            Transport::Email => "email",
        }
    }

    /// macOS app to bring to the front after the URI is opened, if any.
    fn app(self) -> Option<&'static str> {
        match self {
            Transport::Signal => Some("Signal"),
            Transport::WhatsApp => Some("WhatsApp"),
            Transport::Telegram => Some("Telegram"),
            Transport::IMessage | Transport::Sms => Some("Messages"),
            Transport::FaceTime => Some("FaceTime"),
            // These go to whatever app the system has chosen as the handler.
            Transport::Tel | Transport::Email => None,
        }
    }

    /// The address on the contact this transport needs: a phone number for
    /// everything except email.
    fn address(self, contact: &Contact) -> Result<String> {
        match self {
            Transport::Email => contact
                .first_email()
                .map(String::from)
                .with_context(|| format!("Contact '{}' has no email address on record", contact.name)),
            _ => contact
                .first_phone()
                .filter(|p| !p.is_empty())
                .with_context(|| format!("Contact '{}' has no phone number on record", contact.name)),
        }
    }

    /// Build the URI for an address returned by `address`.
    fn url(self, address: &str) -> String {
        // WhatsApp and Telegram want the international number without the `+`.
        let bare = address.trim_start_matches('+');
        match self {
            Transport::Signal => format!("sgnl://signal.me/#p/{address}"),
            Transport::WhatsApp => format!("whatsapp://send?phone={bare}"),
            Transport::Telegram => format!("tg://resolve?phone={bare}"),
            Transport::IMessage => format!("imessage://{address}"),
            Transport::Sms => format!("sms:{address}"),
            Transport::FaceTime => format!("facetime://{address}"),
            Transport::Tel => format!("tel:{address}"),
            Transport::Email => format!("mailto:{address}"),
        }
    }
}

const TRANSPORT_FLAGS: &str =
    "--signal, --whatsapp, --telegram, --imessage, --sms, --facetime, --tel, --email";

fn cmd_message(query: &str, transport: Option<Transport>) -> Result<()> {
    let contacts = load_contacts()?;
    let results: Vec<&Contact> = contacts.iter().filter(|c| c.matches(query)).collect();

    if results.is_empty() {
        println!("{}", format!("No contacts matching '{}'", query).yellow());
        return Ok(());
    }

    let Some(transport) = transport else {
        println!(
            "{}",
            format!("No messaging transport specified. Use one of: {TRANSPORT_FLAGS}.").yellow()
        );
        return Ok(());
    };

    let top = results[0];
    let address = transport.address(top)?;
    let url = transport.url(&address);

    println!("{}", url);
    println!(
        "{} Opening {} for {} ({})",
        "→".cyan().bold(),
        transport.label(),
        top.name.bold(),
        address.dimmed()
    );
    open_url(&url)?;
    if let Some(app) = transport.app() {
        focus_app(app);
    }

    Ok(())
}

fn open_url(url: &str) -> Result<()> {
    // Try xdg-open (Linux), open (macOS), start (Windows) in that order
    #[cfg(target_os = "linux")]
    Command::new("xdg-open")
        .arg(url)
        .status()
        .context("Failed to launch xdg-open")?;

    #[cfg(target_os = "macos")]
    Command::new("open")
        .arg(url)
        .status()
        .context("Failed to launch open")?;

    #[cfg(target_os = "windows")]
    Command::new("cmd")
        .args(["/C", "start", "", url])
        .status()
        .context("Failed to launch start")?;

    Ok(())
}

/// Switch to the app once its URI has been handed over. Opening a URI does not
/// always raise the app's window (Electron apps like Signal often stay in the
/// background), so on macOS ask the system to activate it explicitly. Linux and
/// Windows have no equivalent, so the URI handler decides there.
#[cfg(target_os = "macos")]
fn focus_app(app: &str) {
    match Command::new("open").args(["-a", app]).status() {
        Ok(status) if status.success() => {}
        _ => eprintln!("{}", format!("Could not switch to {app}").yellow()),
    }
}

#[cfg(not(target_os = "macos"))]
fn focus_app(_app: &str) {}

// ─── MAIN ───────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Import { file } => cmd_import(file),
        Commands::Search { query } => cmd_search(query).map(|_| ()),
        Commands::Message { query, transport } => cmd_message(query, transport.selected()),
    };

    if let Err(e) = result {
        eprintln!("{} {}", "Error:".red().bold(), e);
        std::process::exit(1);
    }
}

// ─── TESTS ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(phones: &[&str], emails: &[&str]) -> Contact {
        Contact {
            name: "Alice Smith".to_string(),
            emails: emails.iter().map(|s| s.to_string()).collect(),
            phones: phones.iter().map(|s| s.to_string()).collect(),
            organization: None,
            notes: None,
        }
    }

    #[test]
    fn clean_phone_keeps_digits_and_a_leading_plus() {
        assert_eq!(clean_phone("+61 412 345 678"), "+61412345678");
        assert_eq!(clean_phone("+61 (412) 345-678"), "+61412345678");
        assert_eq!(clean_phone(" 0412.345.678 "), "0412345678");
    }

    #[test]
    fn clean_phone_drops_a_plus_that_is_not_leading() {
        assert_eq!(clean_phone("61+412"), "61412");
    }

    #[test]
    fn signal_uses_the_sgnl_scheme_with_the_plus_kept() {
        assert_eq!(
            Transport::Signal.url("+61412345678"),
            "sgnl://signal.me/#p/+61412345678"
        );
    }

    #[test]
    fn whatsapp_and_telegram_drop_the_plus() {
        assert_eq!(
            Transport::WhatsApp.url("+61412345678"),
            "whatsapp://send?phone=61412345678"
        );
        assert_eq!(
            Transport::Telegram.url("+61412345678"),
            "tg://resolve?phone=61412345678"
        );
    }

    #[test]
    fn apple_and_phone_schemes() {
        assert_eq!(Transport::IMessage.url("+61412345678"), "imessage://+61412345678");
        assert_eq!(Transport::Sms.url("+61412345678"), "sms:+61412345678");
        assert_eq!(Transport::FaceTime.url("+61412345678"), "facetime://+61412345678");
        assert_eq!(Transport::Tel.url("+61412345678"), "tel:+61412345678");
    }

    #[test]
    fn email_uses_mailto() {
        assert_eq!(Transport::Email.url("a@example.com"), "mailto:a@example.com");
    }

    #[test]
    fn phone_transports_use_the_first_phone_cleaned() {
        let c = contact(&["+61 412 345 678", "+61 400 000 000"], &["a@example.com"]);

        assert_eq!(Transport::Signal.address(&c).unwrap(), "+61412345678");
    }

    #[test]
    fn email_transport_uses_the_first_email() {
        let c = contact(&["+61 412 345 678"], &["a@example.com", "b@example.com"]);

        assert_eq!(Transport::Email.address(&c).unwrap(), "a@example.com");
    }

    #[test]
    fn missing_phone_or_email_is_an_error_naming_the_contact() {
        let c = contact(&[], &[]);

        let phone = Transport::Signal.address(&c).unwrap_err().to_string();
        let email = Transport::Email.address(&c).unwrap_err().to_string();

        assert!(phone.contains("Alice Smith") && phone.contains("phone"), "{phone}");
        assert!(email.contains("Alice Smith") && email.contains("email"), "{email}");
    }

    #[test]
    fn app_transports_are_brought_to_the_front_and_system_handlers_are_not() {
        assert_eq!(Transport::Signal.app(), Some("Signal"));
        assert_eq!(Transport::Sms.app(), Some("Messages"));
        assert_eq!(Transport::IMessage.app(), Some("Messages"));
        assert_eq!(Transport::Tel.app(), None);
        assert_eq!(Transport::Email.app(), None);
    }

    fn selected(args: &[&str]) -> Result<Option<Transport>, clap::Error> {
        let mut argv = vec!["contacts-cli", "message"];
        argv.extend_from_slice(args);
        argv.push("alice");
        let cli = Cli::try_parse_from(argv)?;
        match cli.command {
            Commands::Message { transport, .. } => Ok(transport.selected()),
            _ => unreachable!(),
        }
    }

    #[test]
    fn each_flag_selects_its_transport() {
        for (flag, expected) in [
            ("--signal", Transport::Signal),
            ("--whatsapp", Transport::WhatsApp),
            ("--telegram", Transport::Telegram),
            ("--imessage", Transport::IMessage),
            ("--sms", Transport::Sms),
            ("--facetime", Transport::FaceTime),
            ("--tel", Transport::Tel),
            ("--email", Transport::Email),
        ] {
            assert_eq!(selected(&[flag]).unwrap(), Some(expected), "{flag}");
        }
    }

    #[test]
    fn no_flag_selects_nothing() {
        assert_eq!(selected(&[]).unwrap(), None);
    }

    #[test]
    fn transport_flags_are_mutually_exclusive() {
        assert!(selected(&["--signal", "--sms"]).is_err());
    }

    // ── Contact::matches ────────────────────────────────────────────────────

    fn full_contact() -> Contact {
        Contact {
            name: "Alice Smith".to_string(),
            emails: vec!["Alice@Acme.com".to_string()],
            phones: vec!["+61 412 345 678".to_string()],
            organization: Some("Acme Corp".to_string()),
            notes: Some("met at a conference".to_string()),
        }
    }

    #[test]
    fn matches_name_email_phone_and_organization() {
        let c = full_contact();

        assert!(c.matches("smith"));
        assert!(c.matches("@acme.com"));
        assert!(c.matches("412 345"));
        assert!(c.matches("corp"));
    }

    #[test]
    fn matching_is_case_insensitive_both_ways() {
        let c = full_contact();

        assert!(c.matches("ALICE"));
        assert!(c.matches("alice@acme.COM"));
    }

    #[test]
    fn matches_nothing_unrelated_and_ignores_notes() {
        let c = full_contact();

        assert!(!c.matches("bob"));
        assert!(!c.matches("conference"));
    }

    // ── parse_google_csv ────────────────────────────────────────────────────

    /// A scratch file in the OS temp dir, removed on drop.
    struct TempCsv(PathBuf);

    impl TempCsv {
        fn new(contents: &str) -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "contacts-cli-test-{}-{}.csv",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::write(&path, contents).unwrap();
            Self(path)
        }
    }

    impl Drop for TempCsv {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    const HEADER: &str = "First Name,Middle Name,Last Name,Organization Name,\
E-mail 1 - Label,E-mail 1 - Value,E-mail 2 - Value,Phone 1 - Value,Phone 2 - Value,Notes";

    fn parse(rows: &[&str]) -> Vec<Contact> {
        let csv = TempCsv::new(&format!("{HEADER}\n{}\n", rows.join("\n")));
        parse_google_csv(&csv.0).unwrap()
    }

    #[test]
    fn csv_name_is_assembled_from_first_middle_and_last() {
        let contacts = parse(&["Alice,Jane,Smith,,,,,,,"]);

        assert_eq!(contacts[0].name, "Alice Jane Smith");
    }

    #[test]
    fn csv_collects_every_phone_and_email() {
        let contacts = parse(&["Alice,,Smith,Acme,Work,a@x.com,b@x.com,+61 412 345 678,+61 400 000 000,hello"]);
        let c = &contacts[0];

        assert_eq!(c.emails, ["a@x.com", "b@x.com"]);
        assert_eq!(c.phones, ["+61 412 345 678", "+61 400 000 000"]);
        assert_eq!(c.organization.as_deref(), Some("Acme"));
        assert_eq!(c.notes.as_deref(), Some("hello"));
    }

    #[test]
    fn csv_blank_fields_become_empty_or_none() {
        let contacts = parse(&["Bob,,Jones,,,,,,,"]);
        let c = &contacts[0];

        assert!(c.emails.is_empty());
        assert!(c.phones.is_empty());
        assert_eq!(c.organization, None);
        assert_eq!(c.notes, None);
    }

    #[test]
    fn csv_organisation_only_row_is_named_after_the_organisation() {
        let contacts = parse(&[",,,Acme Pty Ltd,,,,,,"]);

        assert_eq!(contacts[0].name, "Acme Pty Ltd");
    }

    #[test]
    fn csv_rows_with_no_name_and_no_organisation_are_skipped() {
        let contacts = parse(&[",,,,,x@y.com,,+61 400 000 000,,", "Alice,,Smith,,,,,,,"]);

        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].name, "Alice Smith");
    }

    #[test]
    fn csv_calendar_addresses_are_not_treated_as_emails() {
        let contacts = parse(&[
            "Alice,,Smith,,,abc@group.calendar.google.com,real@x.com,,,",
            "Bob,,Jones,,,abc@group.v.calendar.google.com,,,,",
        ]);

        assert_eq!(contacts[0].emails, ["real@x.com"]);
        assert!(contacts[1].emails.is_empty());
    }

    #[test]
    fn csv_utf8_bom_on_the_first_header_is_ignored() {
        let csv = TempCsv::new(&format!("\u{feff}{HEADER}\nAlice,,Smith,,,,,,,\n"));

        let contacts = parse_google_csv(&csv.0).unwrap();

        assert_eq!(contacts[0].name, "Alice Smith");
    }

    #[test]
    fn csv_legacy_column_names_are_accepted() {
        let csv = TempCsv::new(
            "Given Name,Family Name,Organization 1 - Name,Phone 1 - Value\n\
             Alice,Smith,Acme,+61 412 345 678\n",
        );

        let contacts = parse_google_csv(&csv.0).unwrap();

        assert_eq!(contacts[0].name, "Alice Smith");
        assert_eq!(contacts[0].organization.as_deref(), Some("Acme"));
        assert_eq!(contacts[0].phones, ["+61 412 345 678"]);
    }

    #[test]
    fn csv_missing_file_is_a_clear_error() {
        let error = parse_google_csv(&PathBuf::from("/definitely/not/here.csv")).unwrap_err();

        assert!(error.to_string().contains("Cannot open CSV file"), "{error}");
    }
}
