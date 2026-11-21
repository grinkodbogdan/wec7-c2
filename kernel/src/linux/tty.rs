//! TTYs of the Linux personality: the console and pseudo-terminals.
//!
//! The console's input arrives from the user-space console driver
//! (`TTY_INPUT`); its output goes to the kernel log, which that same driver
//! prints. A pseudo-terminal's input is what its master writes; its output
//! (and echo) is queued for the master to read, with `\n` sent as `\r\n`
//! (OPOST|ONLCR) as terminal emulators expect. This file is the line
//! discipline: canonical editing, echo, erase/kill, ^C and ^D.

use super::abi::*;
use crate::klog;
use alloc::collections::VecDeque;
use alloc::rc::Rc;
use core::cell::RefCell;
use alloc::vec::Vec;

/// struct termios (asm-generic): 4 flag words, line, 19 control chars.
#[derive(Clone, Copy)]
pub struct Termios {
    pub iflag: u32,
    pub oflag: u32,
    pub cflag: u32,
    pub lflag: u32,
    pub line: u8,
    pub cc: [u8; 19],
}

pub const TERMIOS_SIZE: usize = 36;
const VINTR: usize = 0;
const VERASE: usize = 2;
const VKILL: usize = 3;
const VEOF: usize = 4;

impl Termios {
    fn default_cooked() -> Termios {
        let mut cc = [0u8; 19];
        cc[VINTR] = 3;
        cc[1] = 28; // VQUIT
        cc[VERASE] = 0x7f;
        cc[VKILL] = 21;
        cc[VEOF] = 4;
        cc[6] = 1; // VMIN
        Termios {
            iflag: ICRNL,
            oflag: OPOST | ONLCR,
            cflag: 0o277, // B38400 | CS8 | CREAD
            lflag: ISIG | ICANON | ECHO | ECHOE | ECHOK | ECHOCTL | ECHOKE | IEXTEN,
            line: 0,
            cc,
        }
    }

    pub fn to_bytes(self) -> [u8; TERMIOS_SIZE] {
        let mut b = [0u8; TERMIOS_SIZE];
        b[0..4].copy_from_slice(&self.iflag.to_le_bytes());
        b[4..8].copy_from_slice(&self.oflag.to_le_bytes());
        b[8..12].copy_from_slice(&self.cflag.to_le_bytes());
        b[12..16].copy_from_slice(&self.lflag.to_le_bytes());
        b[16] = self.line;
        b[17..36].copy_from_slice(&self.cc);
        b
    }

    pub fn from_bytes(b: &[u8]) -> Termios {
        let w = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let mut cc = [0u8; 19];
        cc.copy_from_slice(&b[17..36]);
        Termios { iflag: w(0), oflag: w(4), cflag: w(8), lflag: w(12), line: b[16], cc }
    }
}

pub struct Tty {
    pub termios: Termios,
    /// Bytes ready for `read`.
    ready: VecDeque<u8>,
    /// Canonical-mode line being edited.
    edit: Vec<u8>,
    /// Pending end-of-file (^D on an empty line).
    eof: bool,
    pub fg_pgrp: u32,
    pub rows: u16,
    pub cols: u16,
    /// ^C typed: the kernel delivers SIGINT to the foreground group.
    pub interrupt: bool,
    /// A pseudo-terminal's output, for its master (None: the console).
    pub out: Option<VecDeque<u8>>,
}

impl Tty {
    pub fn new() -> Tty {
        Tty {
            termios: Termios::default_cooked(),
            ready: VecDeque::new(),
            edit: Vec::new(),
            eof: false,
            fg_pgrp: 1,
            rows: 24,
            cols: 80,
            interrupt: false,
            out: None,
        }
    }

    /// A pseudo-terminal's line discipline (output queued for the master).
    pub fn new_pty() -> Tty {
        let mut t = Tty::new();
        t.out = Some(VecDeque::new());
        t.fg_pgrp = 0;
        t
    }

    fn emit(&mut self, bytes: &[u8]) {
        let onlcr = self.termios.oflag & (OPOST | ONLCR) == OPOST | ONLCR;
        match self.out.as_mut() {
            None => klog::write(bytes),
            Some(q) => {
                for &b in bytes {
                    if b == b'\n' && onlcr {
                        q.push_back(b'\r');
                    }
                    q.push_back(b);
                }
            }
        }
    }

    fn echo(&mut self, bytes: &[u8]) {
        if self.termios.lflag & ECHO != 0 {
            self.emit(bytes);
        }
    }

    /// Feed bytes typed on the console.
    pub fn input(&mut self, bytes: &[u8]) {
        for &raw in bytes {
            let mut c = raw;
            if c == b'\r' && self.termios.iflag & ICRNL != 0 {
                c = b'\n';
            }
            let t = self.termios;
            if t.lflag & ISIG != 0 && c == t.cc[VINTR] {
                self.edit.clear();
                self.interrupt = true;
                self.echo(b"^C\n");
                continue;
            }
            if t.lflag & ICANON == 0 {
                self.ready.push_back(c);
                self.echo(&[c]);
                continue;
            }
            if c == t.cc[VERASE] || c == 8 {
                if self.edit.pop().is_some() {
                    self.echo(b"\x08 \x08");
                }
            } else if c == t.cc[VKILL] {
                for _ in 0..self.edit.len() {
                    self.echo(b"\x08 \x08");
                }
                self.edit.clear();
            } else if c == t.cc[VEOF] {
                if self.edit.is_empty() {
                    self.eof = true;
                } else {
                    self.ready.extend(self.edit.drain(..));
                }
            } else if c == b'\n' {
                self.edit.push(c);
                self.ready.extend(self.edit.drain(..));
                self.echo(b"\n");
            } else {
                self.edit.push(c);
                self.echo(&[c]);
            }
        }
    }

    pub fn readable(&self) -> bool {
        !self.ready.is_empty() || self.eof
    }

    /// Read up to `max` bytes; `None` if the caller must wait.
    pub fn read(&mut self, max: usize) -> Option<Vec<u8>> {
        if self.ready.is_empty() {
            if self.eof {
                self.eof = false;
                return Some(Vec::new());
            }
            return None;
        }
        let canon = self.termios.lflag & ICANON != 0;
        let mut out = Vec::new();
        while out.len() < max {
            let Some(b) = self.ready.pop_front() else { break };
            out.push(b);
            if canon && b == b'\n' {
                break;
            }
        }
        Some(out)
    }

    pub fn pending(&self) -> usize {
        self.ready.len()
    }

    pub fn write(&mut self, bytes: &[u8]) {
        self.emit(bytes);
    }

    /// A pseudo-terminal's output waiting for its master.
    pub fn take_output(&mut self, max: usize) -> Vec<u8> {
        match self.out.as_mut() {
            Some(q) => {
                let n = max.min(q.len());
                q.drain(..n).collect()
            }
            None => Vec::new(),
        }
    }

    pub fn output_pending(&self) -> usize {
        self.out.as_ref().map_or(0, |q| q.len())
    }
}

/// A pseudo-terminal: `/dev/ptmx` opens a master, `/dev/pts/N` its slave.
pub struct Pty {
    pub tty: Tty,
    pub index: u32,
    /// Slaves cannot be opened until the master unlocks (unlockpt).
    pub locked: bool,
    pub masters: u32,
    pub slaves: u32,
    /// A slave has been opened (once all are closed, the master reads EOF).
    pub had_slave: bool,
}

pub type PtyRef = Rc<RefCell<Pty>>;

impl Pty {
    pub fn new(index: u32) -> PtyRef {
        Rc::new(RefCell::new(Pty { tty: Tty::new_pty(), index, locked: true, masters: 1, slaves: 0, had_slave: false }))
    }
}
