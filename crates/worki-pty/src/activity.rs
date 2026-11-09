//! What an agent in a session is doing, derived from its terminal output: recent output means
//! working; a bell or desktop-notification escape means it wants attention; worki's own
//! `OSC 777;worki;…` markers (printed by injected agent hooks) override both when present.

use std::time::{Duration, Instant};

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Activity {
    Working,
    Waiting,
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Signal {
    Bell,
    Notify(String),
    HookWorking,
    HookWaiting,
}

/// Output within this window counts as the agent working.
const WORKING_WINDOW: Duration = Duration::from_millis(1500);
/// Output this soon after typing is treated as echo, not as the agent working.
const ECHO_WINDOW: Duration = Duration::from_millis(400);
const MAX_OSC: usize = 1024;

#[derive(Default)]
enum State {
    #[default]
    Ground,
    Esc,
    Osc,
    OscEsc,
}

/// Streaming scanner for BEL and OSC sequences; keeps its state across output chunks.
#[derive(Default)]
pub(crate) struct Scanner {
    state: State,
    osc: Vec<u8>,
}

impl Scanner {
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Signal>) {
        for &b in bytes {
            self.state = match self.state {
                State::Ground => match b {
                    0x1b => State::Esc,
                    0x07 => {
                        out.push(Signal::Bell);
                        State::Ground
                    }
                    _ => State::Ground,
                },
                State::Esc if b == b']' => {
                    self.osc.clear();
                    State::Osc
                }
                State::Esc => State::Ground,
                State::Osc => match b {
                    0x07 => {
                        self.finish(out);
                        State::Ground
                    }
                    0x1b => State::OscEsc,
                    _ => {
                        if self.osc.len() < MAX_OSC {
                            self.osc.push(b);
                        }
                        State::Osc
                    }
                },
                State::OscEsc => {
                    if b == b'\\' {
                        self.finish(out);
                    }
                    State::Ground
                }
            };
        }
    }

    fn finish(&mut self, out: &mut Vec<Signal>) {
        let payload = String::from_utf8_lossy(&self.osc).into_owned();
        if let Some(rest) = payload.strip_prefix("777;worki;") {
            match rest {
                "working" => out.push(Signal::HookWorking),
                "waiting" => out.push(Signal::HookWaiting),
                _ => {}
            }
        } else if let Some(rest) = payload.strip_prefix("777;notify;") {
            let mut parts = rest.splitn(2, ';');
            let title = parts.next().unwrap_or_default();
            let body = parts.next().unwrap_or_default();
            out.push(Signal::Notify(if body.is_empty() {
                title.into()
            } else {
                format!("{title}: {body}")
            }));
        } else if let Some(text) = payload.strip_prefix("9;") {
            // OSC 9;4;… is a progress report (ConEmu), not a notification.
            if !text.starts_with("4;") {
                out.push(Signal::Notify(text.into()));
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct Tracker {
    last_output: Option<Instant>,
    last_input: Option<Instant>,
    waiting: bool,
    message: Option<String>,
    /// The agent reports its own state through worki hooks; output timing is then ignored.
    hooked: bool,
    hook_working: bool,
}

impl Tracker {
    pub fn on_output(&mut self, now: Instant, signals: &[Signal]) {
        let echo = self
            .last_input
            .is_some_and(|i| now.duration_since(i) < ECHO_WINDOW);
        if !echo {
            self.last_output = Some(now);
        }
        for signal in signals {
            match signal {
                Signal::Bell => self.waiting = true,
                Signal::Notify(text) => {
                    self.waiting = true;
                    self.message = Some(text.clone()).filter(|t| !t.is_empty());
                }
                Signal::HookWorking => {
                    self.hooked = true;
                    self.hook_working = true;
                    self.waiting = false;
                    self.message = None;
                }
                Signal::HookWaiting => {
                    self.hooked = true;
                    self.hook_working = false;
                    self.waiting = true;
                }
            }
        }
    }

    /// Typing answers whatever the agent was waiting for.
    pub fn on_input(&mut self, now: Instant) {
        self.last_input = Some(now);
        self.waiting = false;
        self.message = None;
    }

    pub fn activity(&self, now: Instant) -> Activity {
        if self.waiting {
            return Activity::Waiting;
        }
        let busy = if self.hooked {
            self.hook_working
        } else {
            self.last_output
                .is_some_and(|t| now.duration_since(t) < WORKING_WINDOW)
        };
        if busy {
            Activity::Working
        } else {
            Activity::Idle
        }
    }

    pub fn message(&self) -> Option<String> {
        self.message.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Vec<Signal> {
        let mut s = Scanner::default();
        let mut out = Vec::new();
        for c in chunks {
            s.feed(c, &mut out);
        }
        out
    }

    #[test]
    fn finds_bell_but_not_title_terminators() {
        assert_eq!(scan(&[b"done\x07"]), [Signal::Bell]);
        assert_eq!(scan(&[b"\x1b]0;my title\x07text"]), []);
        assert_eq!(scan(&[b"\x1b[31mred\x1b[0m"]), []);
    }

    #[test]
    fn parses_notifications_and_worki_markers_across_chunks() {
        assert_eq!(
            scan(&[b"\x1b]9;Build fin", b"ished\x07"]),
            [Signal::Notify("Build finished".into())]
        );
        assert_eq!(
            scan(&[b"\x1b]777;notify;Claude;Needs approval\x1b\\"]),
            [Signal::Notify("Claude: Needs approval".into())]
        );
        assert_eq!(
            scan(&[b"\x1b]777;worki;work", b"ing\x07\x1b]777;worki;waiting\x07"]),
            [Signal::HookWorking, Signal::HookWaiting]
        );
        assert_eq!(scan(&[b"\x1b]9;4;1;50\x07"]), []);
    }

    #[test]
    fn output_means_working_then_idle() {
        let t0 = Instant::now();
        let mut t = Tracker::default();
        assert_eq!(t.activity(t0), Activity::Idle);
        t.on_output(t0, &[]);
        assert_eq!(
            t.activity(t0 + Duration::from_millis(500)),
            Activity::Working
        );
        assert_eq!(t.activity(t0 + Duration::from_secs(3)), Activity::Idle);
    }

    #[test]
    fn echo_of_typing_is_not_working() {
        let t0 = Instant::now();
        let mut t = Tracker::default();
        t.on_input(t0);
        t.on_output(t0 + Duration::from_millis(50), &[]);
        assert_eq!(t.activity(t0 + Duration::from_millis(100)), Activity::Idle);
    }

    #[test]
    fn bell_waits_until_input() {
        let t0 = Instant::now();
        let mut t = Tracker::default();
        t.on_output(t0, &[Signal::Notify("Needs approval".into())]);
        assert_eq!(t.activity(t0 + Duration::from_secs(10)), Activity::Waiting);
        assert_eq!(t.message().as_deref(), Some("Needs approval"));
        t.on_input(t0 + Duration::from_secs(11));
        assert_eq!(t.activity(t0 + Duration::from_secs(12)), Activity::Idle);
        assert_eq!(t.message(), None);
    }

    #[test]
    fn hooks_override_output_timing() {
        let t0 = Instant::now();
        let mut t = Tracker::default();
        t.on_output(t0, &[Signal::HookWorking]);
        assert_eq!(
            t.activity(t0 + Duration::from_secs(60)),
            Activity::Working,
            "quiet but still working"
        );
        t.on_output(t0 + Duration::from_secs(61), &[Signal::HookWaiting]);
        assert_eq!(t.activity(t0 + Duration::from_secs(62)), Activity::Waiting);
        t.on_input(t0 + Duration::from_secs(63));
        t.on_output(t0 + Duration::from_secs(64), &[]);
        assert_eq!(
            t.activity(t0 + Duration::from_secs(64)),
            Activity::Idle,
            "redraws don't count once hooked"
        );
    }
}
