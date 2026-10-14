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

Contacts are stored in `$XDG_DATA_HOME/contacts-cli/contacts.json`
(defaults to `~/.local/share/contacts-cli/contacts.json`).

---

### `contacts-cli search <QUERY>`

Search by name, email, phone, or organisation. Case-insensitive, substring match.

```
contacts-cli search alice
contacts-cli search "@acme.com"
contacts-cli search "+61 4"
```

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

The URI is opened with `xdg-open` on Linux, `open` on macOS, or `start` on
Windows. On macOS, the app is then brought to the front (`open -a <app>`), since
opening a URI doesn't always raise the app's window. Signal, WhatsApp, Telegram,
FaceTime and Messages (for `--imessage` and `--sms`) are switched to this way;
`--tel` and `--email` go to whatever app the system has chosen as the handler.
On Linux and Windows, the URI handler decides which window comes forward.

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
  "phones": ["+61 412 345 678"],
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
