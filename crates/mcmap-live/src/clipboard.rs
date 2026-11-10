//! F3+C through the clipboard.
//!
//! On Wayland a window that isn't focused can read the clipboard only through a data-control
//! protocol (`ext-data-control-v1` or `wlr-data-control-unstable-v1`), which wlroots compositors
//! and KDE offer and GNOME does not. [`probe`] says which case this session is. When the
//! protocol is there, [`ClipboardSource`] polls the clipboard and emits each new F3+C line; when
//! it isn't, the map reads the clipboard itself whenever its window gains focus (and on Ctrl+V),
//! so F3+C followed by a switch to the map still lands.

use crate::fix::{FixSink, PositionSource, StopToken};
use crate::protocol::parse_f3c;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum Watch {
    /// The compositor offers data control: the map can watch in the background.
    Available,
    /// Background watching isn't possible; the reason, for the status line.
    Unavailable(String),
}

/// Whether a background clipboard watch works in this session.
pub fn probe() -> Watch {
    if std::env::var_os("MCMAP_CLIPBOARD_WATCH").is_some_and(|v| v == "0") {
        return Watch::Unavailable("turned off by MCMAP_CLIPBOARD_WATCH=0".into());
    }
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Watch::Unavailable("not a Wayland session".into());
        }
        use wl_clipboard_rs::paste::{get_contents, ClipboardType, Error, MimeType, Seat};
        match get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Text) {
            Ok(_) | Err(Error::ClipboardEmpty) | Err(Error::NoMimeType) | Err(Error::NoSeats) => Watch::Available,
            Err(Error::MissingProtocol { name, .. }) => Watch::Unavailable(format!("the compositor has no {name}")),
            Err(e) => Watch::Unavailable(e.to_string()),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        Watch::Unavailable("background clipboard watching is Linux-only".into())
    }
}

/// Read the clipboard's text once through data control.
#[cfg(target_os = "linux")]
pub fn read_text() -> Option<String> {
    use std::io::Read;
    use wl_clipboard_rs::paste::{get_contents, ClipboardType, MimeType, Seat};
    let (mut pipe, _) = get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Text).ok()?;
    let mut s = String::new();
    pipe.read_to_string(&mut s).ok()?;
    Some(s)
}

#[cfg(not(target_os = "linux"))]
pub fn read_text() -> Option<String> {
    None
}

/// Polls the clipboard and emits a fix for each new F3+C line.
pub struct ClipboardSource {
    pub period: Duration,
    /// Where the text comes from (the clipboard; tests substitute their own).
    pub read: Box<dyn FnMut() -> Option<String> + Send>,
}

impl Default for ClipboardSource {
    fn default() -> Self {
        ClipboardSource { period: Duration::from_millis(300), read: Box::new(read_text) }
    }
}

impl PositionSource for ClipboardSource {
    fn run(mut self: Box<Self>, out: FixSink, stop: StopToken) {
        // The clipboard as the map found it is old news, not a new position.
        let mut last = (self.read)();
        while !stop.stopped() {
            let now = (self.read)();
            if now != last {
                if let Some(f) = now.as_deref().and_then(|t| parse_f3c(t.trim())) {
                    if !out.send(f) {
                        return;
                    }
                }
                last = now;
            }
            let mut slept = Duration::ZERO;
            while slept < self.period && !stop.stopped() {
                let d = Duration::from_millis(50).min(self.period - slept);
                std::thread::sleep(d);
                slept += d;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawn;
    use std::sync::{Arc, Mutex};

    #[test]
    fn emits_only_new_f3c_lines() {
        let text = Arc::new(Mutex::new(Some("/execute in minecraft:overworld run tp @s 1.00 2.00 3.00 0.00 0.00".to_string())));
        let t = text.clone();
        let src = ClipboardSource { period: Duration::from_millis(20), read: Box::new(move || t.lock().unwrap().clone()) };
        let (rx, stop) = spawn(Box::new(src));
        std::thread::sleep(Duration::from_millis(80));
        assert!(rx.try_recv().is_err(), "what was on the clipboard at start is not a fix");
        *text.lock().unwrap() = Some("hello".into());
        std::thread::sleep(Duration::from_millis(80));
        assert!(rx.try_recv().is_err(), "other text is ignored");
        *text.lock().unwrap() = Some("/execute in minecraft:the_nether run tp @s -10.50 70.00 20.25 90.00 -5.00\n".into());
        let f = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(f.pos, [-10.5, 70.0, 20.25]);
        assert_eq!(f.dimension, Some(mcmap_data::Dimension::Nether));
        stop.stop();
    }
}
