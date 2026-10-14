# contacts-cli

A fast terminal contact manager with Google Contacts import.

## Install

```sh
brew tap quintrino/tap
brew install quintrino/tap/contacts-cli
```

This installs a prebuilt binary for Apple silicon, Intel macOS or x86_64 Linux. Or copy the binary somewhere on your `$PATH`:

```sh
cp contacts-cli-linux-x86_64 ~/.local/bin/contacts-cli
chmod +x ~/.local/bin/contacts-cli
```

Or build from source (requires Rust ≥ 1.75):

```sh
cargo build --release
cp target/release/contacts-cli ~/.local/bin/
```

---

## Usage

### `contacts-cli import <FILE>`

Import a Google Contacts CSV export. Re-running merges and updates existing contacts.

```
contacts-cli import ~/Downloads/contacts.csv
```

Export your contacts from Google Contacts → **Export** → **Google CSV**.

Phone numbers are cleaned on import:

- A leading MMI (supplementary service) code such as `*31#` or `#31#` is removed.
- Numbers are saved as digits with a leading `+` kept, so `*31#+61 491 570 006` is saved as `+61491570006`.
- Google joins several values in one cell with ` ::: `, and sometimes repeats a number. Each number is saved once, across all of a contact's phone columns.
- Numbers with letters (extensions like `x12`) are only trimmed, not reduced to digits.

Different formats of one number are only merged when they clean to the same string, so `0491 570 006` and `+61 491 570 006` stay as two entries. Re-run `import` to clean numbers already stored from an earlier import.

Contacts are stored in `$XDG_DATA_HOME/contacts-cli/contacts.json`
(defaults to `~/.local/share/contacts-cli/contacts.json`).

---

### `contacts-cli search <QUERY>`

Search by name, email, phone, or organisation. Case-insensitive, substring match. A query that looks like a number (`+61 4`, `(0412) 345`) is compared by digits, so spacing and punctuation don't matter.

```
contacts-cli search alice
contacts-cli search "@acme.com"
contacts-cli search "+61 4"
```

`--format json` prints the matching contacts as a JSON array instead, for use by other tools. Only the data goes to stdout. No matches, or no contacts imported yet, is `[]`, and errors (such as a corrupt contacts file) go to stderr with a non-zero exit status.

```
contacts-cli search --format json alice
```

```json
[
  {
    "name": "Alice Smith",
    "emails": ["alice@acme.com"],
    "phones": ["+61491570006", "+61400000000"],
    "organization": "Acme Corp",
    "notes": null
  }
]
```

Each contact has the fields it is stored with (see [Data format](#data-format)): `emails` and `phones` are arrays that may be empty, and `organization` and `notes` may be `null`. Phone numbers are cleaned when imported (see above). Use `--` before a query that starts with a dash: `contacts-cli search --format json -- "-x"`.

[alfred-contacts-cli-workflow](https://github.com/quintrino/alfred-contacts-cli-workflow) is an Alfred workflow built on this output.

---

### `contacts-cli message --<transport> <QUERY>`

Opens a messaging app, call or email for the top result matching QUERY. Pass exactly one transport flag:

| Flag | URI | Uses |
|------|-----|------|
| `--signal` | `sgnl://signal.me/#p/+61412345678` | phone |
| `--whatsapp` | `whatsapp://send?phone=61412345678` | phone |
| `--telegram` | `tg://resolve?phone=61412345678` | phone |
| `--imessage` | `imessage://+61412345678` | phone |
| `--sms` | `sms:+61412345678` | phone |
| `--facetime` | `facetime://+61412345678` | phone |
| `--tel` | `tel:+61412345678` | phone |
| `--email` | `mailto:alice@example.com` | email |

```
contacts-cli message --signal alice
contacts-cli message --email alice
```

The phone transports use the contact's first phone number, cleaned to digits
with a leading `+` kept (`+61 (412) 345-678` becomes `+61412345678`). WhatsApp
and Telegram take the number without the `+`. Store numbers in international
format (`+<country code>…`): Signal, WhatsApp and Telegram don't resolve
local-format numbers like `0412 345 678`. `--email` uses the first email address.

On macOS the URI is opened in a specific app, which also brings it to the front
(`open -a "<app>" <uri>`). The app is the first one found in `/Applications`,
`/System/Applications` or `~/Applications`:

| Flag | Apps tried, in order |
|------|----------------------|
| `--signal` | `Signal`, then `Signal Beta` |
| `--whatsapp` | `WhatsApp` |
| `--telegram` | `Telegram` |
| `--imessage`, `--sms` | `Messages` |
| `--facetime` | `FaceTime` |

So with only Signal Beta installed, `--signal` opens Signal Beta. If none of the
apps is found, or one fails to open the link, the URI goes to the system's
default handler instead, as it always does for `--tel` and `--email`. On Linux
the URI is opened with `xdg-open` and on Windows with `start`, and the URI
handler decides which window comes forward.

---

## Data format

Contacts are stored as a JSON array at:

```
$XDG_DATA_HOME/contacts-cli/contacts.json
```

Each contact looks like:

```json
{
  "name": "Alice Smith",
  "emails": ["alice@example.com"],
  "phones": ["+61412345678"],
  "organization": "Acme Corp",
  "notes": null
}
```

You can edit this file directly.

---

## Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` | CLI argument parsing |
| `serde` / `serde_json` | JSON serialisation |
| `csv` | CSV parsing |
| `dirs` | XDG/home directory resolution |
| `colored` | Terminal colour output |
| `anyhow` | Error handling |

## Development

```sh
cargo test
```
