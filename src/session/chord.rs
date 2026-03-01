/// Chord actions that can be triggered by key sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChordAction {
    /// No chord matched.
    None,
    /// Detach from session (Ctrl+] then q).
    Detach,
    /// Show diagnostic info (Ctrl+] then d).
    Diagnostic,
}

/// Detach chord (Ctrl+] then q/d) detector.
///
/// Handles both split reads (Ctrl+] in one read, second key in the next) and
/// batched reads (both bytes in a single read).
pub(crate) struct ChordDetector {
    pending: bool,
    last_input: [u8; 64],
    last_input_len: usize,
    last_input_pos: usize,
}

/// Result of processing an input buffer through the chord detector.
pub(crate) struct ChordResult {
    /// Bytes to forward to the PTY (may be empty).
    pub forward: Vec<u8>,
    /// The chord action triggered, if any.
    pub action: ChordAction,
}

const CTRL_BRACKET: u8 = 0x1D;

impl ChordDetector {
    pub fn new() -> Self {
        Self {
            pending: false,
            last_input: [0u8; 64],
            last_input_len: 0,
            last_input_pos: 0,
        }
    }

    /// Record bytes into the 64-byte ring buffer.
    fn record_input(&mut self, input: &[u8]) {
        for &b in input {
            self.last_input[self.last_input_pos] = b;
            self.last_input_pos = (self.last_input_pos + 1) % 64;
            if self.last_input_len < 64 {
                self.last_input_len += 1;
            }
        }
    }

    /// Returns the last N input bytes (up to 64) in order.
    pub fn last_input_bytes(&self) -> Vec<u8> {
        if self.last_input_len == 0 {
            return vec![];
        }
        let mut result = Vec::with_capacity(self.last_input_len);
        let start = if self.last_input_len < 64 {
            0
        } else {
            self.last_input_pos
        };
        for i in 0..self.last_input_len {
            result.push(self.last_input[(start + i) % 64]);
        }
        result
    }

    /// Returns a human-readable description of the current state.
    pub fn state_description(&self) -> &'static str {
        if self.pending {
            "pending (waiting for second key after Ctrl+])"
        } else {
            "idle"
        }
    }

    /// Match the second key of a chord. Returns the action for the given byte.
    fn match_second_key(byte: u8) -> ChordAction {
        match byte {
            b'q' => ChordAction::Detach,
            b'd' => ChordAction::Diagnostic,
            _ => ChordAction::None,
        }
    }

    /// Process an input buffer. Returns bytes to forward and any triggered action.
    pub fn process(&mut self, input: &[u8]) -> ChordResult {
        self.record_input(input);

        if input.is_empty() {
            return ChordResult {
                forward: vec![],
                action: ChordAction::None,
            };
        }

        if self.pending {
            self.pending = false;
            let action = Self::match_second_key(input[0]);
            if action != ChordAction::None {
                return ChordResult {
                    forward: vec![],
                    action,
                };
            }
            // False alarm: prepend the held Ctrl+] and forward all
            let mut combined = Vec::with_capacity(1 + input.len());
            combined.push(CTRL_BRACKET);
            combined.extend_from_slice(input);
            return ChordResult {
                forward: combined,
                action: ChordAction::None,
            };
        }

        if let Some(pos) = input.iter().position(|&b| b == CTRL_BRACKET) {
            let before = &input[..pos];
            let after = &input[pos + 1..];

            if after.is_empty() {
                // Ctrl+] is the last byte — enter pending state
                self.pending = true;
                return ChordResult {
                    forward: before.to_vec(),
                    action: ChordAction::None,
                };
            }

            let action = Self::match_second_key(after[0]);
            if action != ChordAction::None {
                return ChordResult {
                    forward: before.to_vec(),
                    action,
                };
            }

            // False alarm: Ctrl+] followed by non-chord key, forward everything
            return ChordResult {
                forward: input.to_vec(),
                action: ChordAction::None,
            };
        }

        ChordResult {
            forward: input.to_vec(),
            action: ChordAction::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Bug reproduction: the old algorithm ===

    /// Simulates the old (buggy) chord detection from attach.rs.
    /// Returns (forward_bytes, should_detach, new_saw_ctrl_bracket).
    fn old_algorithm(input: &[u8], saw_ctrl_bracket: bool) -> (Vec<u8>, bool, bool) {
        let n = input.len();
        if saw_ctrl_bracket {
            if input.first() == Some(&b'q') {
                return (vec![], true, false);
            } else {
                let mut combined = vec![0x1D];
                combined.extend_from_slice(input);
                return (combined, false, false);
            }
        } else if n == 1 && input[0] == 0x1D {
            // BUG: requires n == 1, misses batched reads
            return (vec![], false, true);
        } else {
            return (input.to_vec(), false, false);
        }
    }

    #[test]
    fn bug_old_algorithm_misses_batched_chord() {
        // When Ctrl+] and 'q' arrive in the same read, the old algorithm
        // fails to detect the chord and forwards both bytes as normal input.
        let input = [0x1D, b'q'];
        let (forward, detach, pending) = old_algorithm(&input, false);

        // BUG: should detach, but doesn't
        assert!(!detach, "old algorithm does NOT detect batched chord");
        assert!(!pending, "old algorithm does NOT enter pending state");
        assert_eq!(forward, vec![0x1D, b'q'], "old algorithm forwards raw bytes");
    }

    #[test]
    fn bug_old_algorithm_misses_ctrl_bracket_in_middle_of_buffer() {
        // Ctrl+] arrives with preceding bytes
        let input = [b'x', 0x1D, b'q'];
        let (forward, detach, pending) = old_algorithm(&input, false);

        // BUG: should forward [b'x'] then detach, but forwards everything
        assert!(!detach);
        assert!(!pending);
        assert_eq!(forward, vec![b'x', 0x1D, b'q']);
    }

    // === New algorithm: ChordDetector ===

    #[test]
    fn split_reads_ctrl_bracket_then_q() {
        let mut det = ChordDetector::new();

        let r1 = det.process(&[0x1D]);
        assert!(r1.forward.is_empty());
        assert_eq!(r1.action, ChordAction::None);

        let r2 = det.process(&[b'q']);
        assert!(r2.forward.is_empty());
        assert_eq!(r2.action, ChordAction::Detach);
    }

    #[test]
    fn batched_chord_in_single_read() {
        let mut det = ChordDetector::new();

        let r = det.process(&[0x1D, b'q']);
        assert!(r.forward.is_empty());
        assert_eq!(r.action, ChordAction::Detach);
    }

    #[test]
    fn chord_in_middle_of_buffer() {
        let mut det = ChordDetector::new();

        let r = det.process(&[b'x', 0x1D, b'q']);
        assert_eq!(r.forward, vec![b'x']);
        assert_eq!(r.action, ChordAction::Detach);
    }

    #[test]
    fn ctrl_bracket_at_end_of_buffer() {
        let mut det = ChordDetector::new();

        let r1 = det.process(&[b'x', b'y', 0x1D]);
        assert_eq!(r1.forward, vec![b'x', b'y']);
        assert_eq!(r1.action, ChordAction::None);

        let r2 = det.process(&[b'q']);
        assert!(r2.forward.is_empty());
        assert_eq!(r2.action, ChordAction::Detach);
    }

    #[test]
    fn false_alarm_split_reads() {
        let mut det = ChordDetector::new();

        let r1 = det.process(&[0x1D]);
        assert!(r1.forward.is_empty());
        assert_eq!(r1.action, ChordAction::None);

        let r2 = det.process(&[b'x']);
        assert_eq!(r2.forward, vec![0x1D, b'x']);
        assert_eq!(r2.action, ChordAction::None);
    }

    #[test]
    fn false_alarm_in_buffer() {
        let mut det = ChordDetector::new();

        let r = det.process(&[0x1D, b'x']);
        assert_eq!(r.forward, vec![0x1D, b'x']);
        assert_eq!(r.action, ChordAction::None);
    }

    #[test]
    fn normal_input_no_chord_byte() {
        let mut det = ChordDetector::new();

        let r = det.process(b"hello");
        assert_eq!(r.forward, b"hello");
        assert_eq!(r.action, ChordAction::None);
    }

    #[test]
    fn empty_input() {
        let mut det = ChordDetector::new();

        let r = det.process(&[]);
        assert!(r.forward.is_empty());
        assert_eq!(r.action, ChordAction::None);
    }

    #[test]
    fn chord_after_false_alarm() {
        let mut det = ChordDetector::new();

        // False alarm
        let r1 = det.process(&[0x1D]);
        assert_eq!(r1.action, ChordAction::None);
        let r2 = det.process(&[b'x']);
        assert_eq!(r2.action, ChordAction::None);

        // Real chord
        let r3 = det.process(&[0x1D]);
        assert_eq!(r3.action, ChordAction::None);
        let r4 = det.process(&[b'q']);
        assert_eq!(r4.action, ChordAction::Detach);
    }

    #[test]
    fn ctrl_bracket_alone_is_pending() {
        let mut det = ChordDetector::new();

        let r = det.process(&[0x1D]);
        assert!(r.forward.is_empty());
        assert_eq!(r.action, ChordAction::None);
        // Internal state: pending
    }

    // === Diagnostic chord tests ===

    #[test]
    fn split_reads_ctrl_bracket_then_d() {
        let mut det = ChordDetector::new();

        let r1 = det.process(&[0x1D]);
        assert_eq!(r1.action, ChordAction::None);

        let r2 = det.process(&[b'd']);
        assert!(r2.forward.is_empty());
        assert_eq!(r2.action, ChordAction::Diagnostic);
    }

    #[test]
    fn batched_diagnostic_chord() {
        let mut det = ChordDetector::new();

        let r = det.process(&[0x1D, b'd']);
        assert!(r.forward.is_empty());
        assert_eq!(r.action, ChordAction::Diagnostic);
    }

    #[test]
    fn diagnostic_chord_in_middle_of_buffer() {
        let mut det = ChordDetector::new();

        let r = det.process(&[b'a', b'b', 0x1D, b'd']);
        assert_eq!(r.forward, vec![b'a', b'b']);
        assert_eq!(r.action, ChordAction::Diagnostic);
    }

    // === last_input_bytes tests ===

    #[test]
    fn last_input_bytes_records_input() {
        let mut det = ChordDetector::new();
        det.process(b"hello");
        assert_eq!(det.last_input_bytes(), b"hello");
    }

    #[test]
    fn last_input_bytes_records_across_calls() {
        let mut det = ChordDetector::new();
        det.process(b"abc");
        det.process(b"def");
        assert_eq!(det.last_input_bytes(), b"abcdef");
    }

    #[test]
    fn last_input_bytes_wraps_at_64() {
        let mut det = ChordDetector::new();
        // Write 70 bytes, should keep last 64
        let data: Vec<u8> = (0..70).collect();
        det.process(&data);
        let result = det.last_input_bytes();
        assert_eq!(result.len(), 64);
        assert_eq!(result, &data[6..70]);
    }

    #[test]
    fn state_description_idle_and_pending() {
        let mut det = ChordDetector::new();
        assert_eq!(det.state_description(), "idle");

        det.process(&[0x1D]);
        assert_eq!(
            det.state_description(),
            "pending (waiting for second key after Ctrl+])"
        );

        det.process(&[b'x']); // false alarm, back to idle
        assert_eq!(det.state_description(), "idle");
    }

    // === PTY integration test ===

    #[test]
    fn chord_through_real_pty() {
        use nix::pty::openpty;
        use nix::sys::termios;
        use std::io::{Read, Write};

        let pty = openpty(None, None).expect("openpty failed");

        // Configure slave to raw mode
        let mut attrs = termios::tcgetattr(&pty.slave).expect("tcgetattr failed");
        termios::cfmakeraw(&mut attrs);
        termios::tcsetattr(&pty.slave, termios::SetArg::TCSANOW, &attrs)
            .expect("tcsetattr failed");

        // Convert OwnedFd to File (OwnedFd implements Into<File> via From<OwnedFd>)
        let mut master: std::fs::File = pty.master.into();
        let mut slave: std::fs::File = pty.slave.into();

        // Write chord bytes through the PTY
        master.write_all(&[0x1D, b'q']).expect("write failed");
        master.flush().expect("flush failed");

        // Read from slave side
        let mut buf = [0u8; 64];
        let n = slave.read(&mut buf).expect("read failed");

        // Feed through ChordDetector
        let mut det = ChordDetector::new();
        let result = det.process(&buf[..n]);

        assert_eq!(
            result.action,
            ChordAction::Detach,
            "chord not detected through PTY; got {} bytes: {:02x?}",
            n,
            &buf[..n]
        );
        assert!(
            result.forward.is_empty(),
            "expected no forwarded bytes, got: {:02x?}",
            result.forward
        );
    }
}
