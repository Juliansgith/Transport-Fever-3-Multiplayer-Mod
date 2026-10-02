use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::bytes::{FixedBytes, write_hex};

/// The letters of a [`Code`]: digits and upper-case letters, without the
/// look-alikes 0, 1, I, L and O.
const CODE_ALPHABET: &[u8; 31] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
/// Characters in a [`Code`].
pub const CODE_LEN: usize = 6;

/// Six letters and digits, such as `K7QM2X`, that players read out, type
/// and paste: a room's invite, and a connection's support code. Upper case
/// from [`CODE_ALPHABET`], with at least one letter and one digit, so an
/// ordinary word in a message is never taken for one. There are some 740
/// million. Decoding enforces all of this.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "FixedBytes<CODE_LEN>", into = "FixedBytes<CODE_LEN>")]
pub struct Code([u8; CODE_LEN]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("a code is six letters and digits, such as K7QM2X")]
pub struct CodeError;

impl Code {
    /// A code from the operating system's random source, each one equally
    /// likely.
    pub fn random() -> Self {
        loop {
            let mut bytes = [0; CODE_LEN];
            for slot in &mut bytes {
                *slot = loop {
                    let mut byte = [0];
                    getrandom::fill(&mut byte)
                        .expect("the operating system's random source is available");
                    // 248 is the largest multiple of 31 up to 256: taking
                    // only bytes below it keeps every letter equally likely.
                    if byte[0] < 248 {
                        break CODE_ALPHABET[usize::from(byte[0] % 31)];
                    }
                };
            }
            if let Ok(code) = Self::try_from(FixedBytes(bytes)) {
                return code;
            }
        }
    }

    pub fn as_str(&self) -> &str {
        // Only ASCII from the alphabet gets in.
        std::str::from_utf8(&self.0).unwrap_or_default()
    }
}

impl TryFrom<FixedBytes<CODE_LEN>> for Code {
    type Error = CodeError;

    fn try_from(bytes: FixedBytes<CODE_LEN>) -> Result<Self, Self::Error> {
        let bytes = bytes.0;
        let valid = bytes.iter().all(|byte| CODE_ALPHABET.contains(byte))
            && bytes.iter().any(u8::is_ascii_digit)
            && bytes.iter().any(u8::is_ascii_uppercase);
        valid.then_some(Self(bytes)).ok_or(CodeError)
    }
}

impl From<Code> for FixedBytes<CODE_LEN> {
    fn from(code: Code) -> Self {
        FixedBytes(code.0)
    }
}

impl FromStr for Code {
    type Err = CodeError;

    /// Takes a code as players type it: in either case, with spaces around.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bytes: [u8; CODE_LEN] = text.trim().as_bytes().try_into().map_err(|_| CodeError)?;
        Self::try_from(FixedBytes(bytes.map(|byte| byte.to_ascii_uppercase())))
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Non-secret identifier of one connection, shown to the player as their
/// support code and safe to quote in bug reports. The server gives each
/// connection one no other has had within its diagnostics' keeping time.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Code);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// One run of a player's launcher, as its diagnostics name it: a code like
/// a support code (D13), chosen by the launcher when it starts and kept
/// until it closes, across every connection it makes meanwhile, where the
/// support code names one connection. Every diagnostics line carries it
/// (`Request::Telemetry`), so an operator finds a whole run's lines by it
/// (proposed D10 amendment). Like a support code it lets nobody into
/// anything, so players may post it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LogSession(pub Code);

impl LogSession {
    /// A new one, for a launcher that starts.
    pub fn random() -> Self {
        Self(Code::random())
    }
}

impl fmt::Display for LogSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for LogSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A player's identity: their per-install Ed25519 public key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PlayerId(pub FixedBytes<32>);

impl PlayerId {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0.0
    }
}

impl fmt::Display for PlayerId {
    /// A short fingerprint for logs and UI; the full key is in `Debug`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("p-")?;
        write_hex(f, &self.0.0[..8])
    }
}

impl fmt::Debug for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("p-")?;
        write_hex(f, &self.0.0)
    }
}

/// An Ed25519 signature.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature(pub FixedBytes<64>);

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Signature(..)")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RoomId(pub FixedBytes<16>);

impl fmt::Display for RoomId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("r-")?;
        write_hex(f, &self.0.0)
    }
}

impl fmt::Debug for RoomId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A room's invite: its code, which the server looks the room up by. Only
/// the server knows which room a code belongs to. `Debug` never shows the
/// code, so invites cannot leak into logs through formatting.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite(pub Code);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("an invite is six letters and digits, such as K7QM2X")]
pub struct InviteError;

impl fmt::Display for Invite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for Invite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Invite(..)")
    }
}

impl FromStr for Invite {
    type Err = InviteError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse().map(Self).map_err(|_| InviteError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(text: &str) -> Code {
        text.parse().unwrap()
    }

    #[test]
    fn a_code_is_six_letters_and_digits_as_players_type_them() {
        assert_eq!(code("K7QM2X").to_string(), "K7QM2X");
        // Pasting often adds whitespace; typing, lower case.
        assert_eq!(code(" k7qm2x\n"), code("K7QM2X"));
        for bad in [
            "", "K7QM2", "K7QM2XA", "K7QM 2X", "K7QM0X", "K7QM1X", "K7QMIX", "K7QMLX", "K7QMOX",
            "K7QM2!", "K7QMÄX", // An ordinary word, or only digits, is no code.
            "THANKS", "234567",
        ] {
            assert_eq!(bad.parse::<Code>(), Err(CodeError), "{bad:?}");
        }
    }

    #[test]
    fn random_codes_are_codes_and_differ() {
        let codes: std::collections::HashSet<Code> = (0..1000).map(|_| Code::random()).collect();
        // Two alike among a thousand of 740 million happens once in some
        // 1,500 runs; five, never.
        assert!(codes.len() >= 995, "{}", codes.len());
        for code in codes {
            assert_eq!(code.to_string().parse::<Code>(), Ok(code));
        }
    }

    #[test]
    fn a_code_off_the_wire_is_checked() {
        let good = postcard::to_stdvec(&code("K7QM2X")).unwrap();
        assert_eq!(good, b"K7QM2X");
        assert_eq!(postcard::from_bytes::<Code>(&good), Ok(code("K7QM2X")));
        for bad in [
            b"k7qm2x",
            b"K7QM0X",
            b"THANKS",
            b"K7QM\x1b[",
            b"\xff\xff\xff\xff\xff\xff",
        ] {
            assert!(postcard::from_bytes::<Code>(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn invite_debug_hides_the_code() {
        let invite = Invite(code("K7QM2X"));
        assert_eq!(invite.to_string(), "K7QM2X");
        assert_eq!("k7qm2x".parse::<Invite>(), Ok(invite));
        assert!(!format!("{invite:?}").contains("K7QM2X"));
    }

    #[test]
    fn player_display_is_a_short_fingerprint() {
        let mut key = [0; 32];
        key[0] = 0x12;
        let player = PlayerId(FixedBytes(key));
        assert_eq!(player.to_string(), "p-1200000000000000");
        assert_eq!(format!("{player:?}").len(), 2 + 64);
    }
}
