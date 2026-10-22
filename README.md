# Platypus Telemetry

An MQTT telemetry logger with a Ratatui terminal interface.

## Run

Configure the broker credentials and subscribed topics in `.env.toml`, then run:

```sh
cargo run
```

Enter the participant's name on the first screen and press **Enter**. The session
is saved to `logs/<participant_name>.log`: for example, `Jane Doe` becomes
`logs/Jane_Doe.log`. Existing files are appended to. To retain the optional
timestamp prefix, use `cargo run -- --time-stamp`.

The session screen shows live telemetry and connection messages in its upper
two-thirds, with a multiline observation editor below. Observations are written
to the same file as telemetry using the normal timestamped record format, with
device `Observer`, topic `observations`, and tag `Observation`. The record's data
is the text you entered, with line breaks replaced by spaces so each record stays
on a single line. Connection/status messages are displayed only.

### Controls

| Key | Action |
| --- | --- |
| Shift+Enter | Save an observation and clear the editor |
| Ctrl+S | Alternative save shortcut |
| Enter | Insert a newline in an observation |
| Page Up / Page Down | Browse logs; reaching the newest entry resumes live scrolling |
| Esc / Ctrl+C | Quit |

Shift+Enter requires a terminal that supports enhanced keyboard reporting (for
example, Kitty or a suitably configured compatible terminal). If your terminal
sends ordinary Enter for both combinations, use Ctrl+S to save instead.

The on-screen log history is limited to 2,000 entries; the log file retains all
telemetry and saved observations. Empty observations are not saved. On a save
failure, the editor keeps your text so you can retry.
