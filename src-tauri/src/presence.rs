//! Discord Rich Presence: "Playing Kakiri" in the member list.
//!
//! This talks to the **local** Discord socket — no network, no bot token, no
//! account access. Discord exposes a unix socket (or a named pipe on Windows) for
//! other programs on the same machine; we connect to it, hand over an app id, and
//! say what we want shown. If Discord is closed, or quits later, the connection
//! simply fails and we stop asking.
//!
//! A worker thread owns the connection because all of this is blocking: a connect
//! can wait, a write can fail. The UI sends a description of what to display and
//! never blocks on it; when nothing changes for a while, nothing is sent.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;
use std::thread;

use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};

/// Public app id, not a secret: it only names the application that Discord shows.
/// Set `KAKIRI_DISCORD_CLIENT_ID` to test with your own app.
const CLIENT_ID: &str = "1468089810013913207";

const RECONNECT: std::time::Duration = std::time::Duration::from_secs(20);

#[derive(Clone, Debug, PartialEq)]
pub struct Now {
    /// First line: what the user is doing right now.
    pub details: String,
    /// Second line: a little more.
    pub state: String,
    /// Unix seconds the current thing started, if it makes sense to show a timer.
    pub started: Option<i64>,
    /// Unix seconds it will end: with this Discord draws the same progress bar
    /// Spotify has instead of a plain count-up.
    pub ends: Option<i64>,
    /// Cover art URL, shown big on the left like an album.
    pub image: Option<String>,
}

enum Msg {
    Show(Now),
    Hide,
}

static TX: OnceLock<Sender<Msg>> = OnceLock::new();
/// Whether the socket is up right now, for the Settings note.
static CONNECTED: AtomicBool = AtomicBool::new(false);

pub fn connected() -> bool {
    CONNECTED.load(Ordering::Relaxed)
}

fn sender() -> &'static Sender<Msg> {
    TX.get_or_init(|| {
        let (tx, rx) = channel::<Msg>();
        thread::spawn(move || run(rx));
        tx
    })
}

fn client_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| std::env::var("KAKIRI_DISCORD_CLIENT_ID").unwrap_or_else(|_| CLIENT_ID.to_string()))
}

/// Show `now` on Discord, or clear it with `None`. Cheap: it only queues a message,
/// so the caller can invoke this on every state change.
pub fn show(now: Option<Now>) {
    let msg = match now {
        Some(n) => Msg::Show(n),
        None => Msg::Hide,
    };
    let _ = sender().send(msg);
}

fn connect() -> Option<DiscordIpcClient> {
    let mut c = DiscordIpcClient::new(client_id()).ok()?;
    c.connect().ok()?;
    Some(c)
}

/// Discord rejects a line longer than 128 characters, and a song title is not
/// always polite about that.
fn clip(s: &str) -> String {
    if s.chars().count() <= 128 {
        return s.to_string();
    }
    let mut out: String = s.chars().take(127).collect();
    out.push('…');
    out
}

/// The activity borrows its text, so the strings have to outlive the call: they are
/// built here and dropped once Discord has the payload.
fn set(client: &mut DiscordIpcClient, now: &Now) -> bool {
    let details = clip(&now.details);
    let state = clip(if now.state.is_empty() { "Kakiri" } else { &now.state });
    let image = now.image.as_deref().map(clip);

    // Listening, not Playing: this is a music player, and Discord says so.
    let mut a = activity::Activity::new()
        .activity_type(activity::ActivityType::Listening)
        .details(&details)
        .state(&state);

    if let (Some(start), Some(end)) = (now.started, now.ends) {
        a = a.timestamps(activity::Timestamps::new().start(start).end(end));
    } else if let Some(start) = now.started {
        a = a.timestamps(activity::Timestamps::new().start(start));
    }

    if let Some(image) = image.as_deref() {
        a = a.assets(
            activity::Assets::new()
                .large_image(image)
                .large_text("Kakiri"),
        );
    }

    client.set_activity(a).is_ok()
}

fn run(rx: Receiver<Msg>) {
    let mut conn: Option<DiscordIpcClient> = None;
    let mut shown: Option<Now> = None;
    let mut wanted: Option<Now> = None;

    loop {
        // Block until something happens, but wake up periodically in case Discord
        // was started after us.
        let msg = rx.recv_timeout(RECONNECT);
        match msg {
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(c) = conn.as_mut() {
                    let _ = c.clear_activity();
                    let _ = c.close();
                }
                return;
            }
            Ok(Msg::Hide) => wanted = None,
            Ok(Msg::Show(now)) => wanted = Some(now),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }

        if wanted == shown {
            continue;
        }

        if conn.is_none() {
            conn = connect();
            CONNECTED.store(conn.is_some(), Ordering::Relaxed);
            if conn.is_none() {
                // Discord is not running; try again on the next tick.
                shown = None;
                continue;
            }
        }

        let Some(client) = conn.as_mut() else { continue };
        let ok = match &wanted {
            Some(now) => set(client, now),
            None => client.clear_activity().is_ok(),
        };
        if ok {
            shown = wanted.clone();
        } else {
            // The socket died (Discord quit or restarted): drop it and retry later.
            let _ = client.close();
            conn = None;
            CONNECTED.store(false, Ordering::Relaxed);
            shown = None;
        }
    }
}


#[cfg(test)]
mod tests {
    use super::clip;

    #[test]
    fn long_titles_are_clipped_and_short_ones_are_left_alone() {
        assert_eq!(clip("Nightcall"), "Nightcall");
        let long = "a".repeat(200);
        let out = clip(&long);
        assert_eq!(out.chars().count(), 128);
        assert!(out.ends_with('…'));
    }
}
