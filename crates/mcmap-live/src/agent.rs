//! The agent socket: the map listens on loopback, the in-game agent connects out and streams
//! protocol lines. A connection must open with a `hello` carrying the token the map handed the
//! agent, so other local processes cannot inject fixes.

use crate::fix::{FixSink, PositionSource, StopToken};
use crate::protocol::{parse_line, Message};
use std::io::{BufRead, BufReader, ErrorKind};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

/// Default port; the agent's options carry the actual one.
pub const DEFAULT_PORT: u16 = 47_123;

pub struct AgentSource {
    listener: TcpListener,
    token: String,
    /// Status and hello lines go here (for the CLI and logs).
    pub on_status: Option<Box<dyn Fn(&str) + Send>>,
}

impl AgentSource {
    /// Bind `127.0.0.1:port` (0 picks a free port; see [`Self::port`]).
    pub fn bind(port: u16, token: impl Into<String>) -> std::io::Result<AgentSource> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        listener.set_nonblocking(true)?;
        Ok(AgentSource { listener, token: token.into(), on_status: None })
    }

    pub fn port(&self) -> u16 {
        self.listener.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    fn note(&self, s: &str) {
        if let Some(f) = &self.on_status {
            f(s)
        }
    }

    /// Serve one connection until it closes, the sink closes or `stop` is set. False when the
    /// sink has closed.
    fn serve(&self, stream: TcpStream, out: &FixSink, stop: &StopToken) -> bool {
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        let mut greeted = false;
        while !stop.stopped() {
            match reader.read_line(&mut line) {
                Ok(0) => {
                    self.note("agent disconnected");
                    return true;
                }
                Ok(_) => {
                    match parse_line(line.trim_end()) {
                        Ok(Message::Hello { token, agent, game }) => {
                            if token != self.token {
                                self.note("agent sent the wrong token; dropping the connection");
                                return true;
                            }
                            greeted = true;
                            self.note(&format!("agent connected: {agent} on {}", game.as_deref().unwrap_or("?")));
                        }
                        Ok(Message::Pos(fix)) if greeted => {
                            if !out.send(fix) {
                                return false;
                            }
                        }
                        Ok(Message::Status { state, msg }) if greeted => {
                            self.note(&format!("agent status {state}{}", msg.map(|m| format!(": {m}")).unwrap_or_default()));
                        }
                        Ok(_) => {
                            self.note("agent spoke before hello; dropping the connection");
                            return true;
                        }
                        Err(e) => self.note(&e),
                    }
                    line.clear();
                }
                // A timeout may leave a partial line in `line`; the next read appends to it.
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(e) => {
                    self.note(&format!("agent connection: {e}"));
                    return true;
                }
            }
        }
        true
    }
}

impl PositionSource for AgentSource {
    fn run(self: Box<Self>, out: FixSink, stop: StopToken) {
        while !stop.stopped() {
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    if !peer.ip().is_loopback() {
                        continue;
                    }
                    if !self.serve(stream, &out, &stop) {
                        return;
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => {
                    self.note(&format!("agent listener: {e}"));
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawn;
    use std::io::Write;

    #[test]
    fn accepts_fixes_after_a_good_hello_only() {
        let src = AgentSource::bind(0, "secret").unwrap();
        let port = src.port();
        let (rx, stopper) = spawn(Box::new(src));

        // Wrong token: nothing comes through.
        let mut bad = TcpStream::connect(("127.0.0.1", port)).unwrap();
        writeln!(bad, r#"{{"v":1,"type":"hello","token":"nope","agent":"x"}}"#).unwrap();
        writeln!(bad, r#"{{"v":1,"type":"pos","dim":"minecraft:overworld","x":1,"y":2,"z":3}}"#).unwrap();
        drop(bad);

        let mut good = TcpStream::connect(("127.0.0.1", port)).unwrap();
        writeln!(good, r#"{{"v":1,"type":"hello","token":"secret","agent":"test"}}"#).unwrap();
        // Split one line across two writes, as TCP may.
        write!(good, r#"{{"v":1,"type":"pos","dim":"minecraft:overworld","#).unwrap();
        good.flush().unwrap();
        std::thread::sleep(Duration::from_millis(150));
        writeln!(good, r#""x":10.5,"y":64,"z":-3,"yaw":45,"pitch":0}}"#).unwrap();
        let fix = rx.recv_timeout(Duration::from_secs(5)).expect("a fix");
        assert_eq!(fix.pos, [10.5, 64.0, -3.0]);
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err(), "the bad connection's fix never arrives");
        stopper.stop();
    }
}
