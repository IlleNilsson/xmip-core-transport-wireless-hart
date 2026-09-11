//! What a DLPDU carries: the network layer's packet — a control byte, the
//! time to live, a snippet of the absolute slot number, the graph, the
//! destination and source nicknames — the security sublayer's control,
//! counter and MIC, and the transport layer's byte followed by the HART
//! commands it aggregates, each a sixteen-bit number, a byte count and its
//! data. A response carries the two status bytes hart's device answers
//! with.
//!
//! The session MIC is AES-CCM* under a session key; as with the DLPDU's,
//! this crate carries the field and a keyed link fills it.

use transport::error::{Result, protocol_error};

use crate::dlpdu;

/// The network header, the security sublayer and the transport byte.
pub const OVERHEAD: usize = 10 + 6 + 1;
/// What one packet's commands may total: numbers, counts and data.
pub const MAX_COMMANDS: usize = dlpdu::MAX_PAYLOAD - OVERHEAD;
/// The most data one command carries in one packet: the commands less one
/// number and one count.
pub const MAX_COMMAND_DATA: usize = MAX_COMMANDS - 3;

/// Network control: short nicknames, no proxy route.
const CONTROL: u8 = 0x00;
const TTL: u8 = 0x20;
/// Security control: a session key, a one-byte counter.
const SECURITY: u8 = 0x00;
/// Transport byte: acknowledged service.
const ACKNOWLEDGED: u8 = 0x80;
/// Transport byte: a response.
const RESPONSE: u8 = 0x40;

/// One HART command as the transport layer aggregates it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub number: u16,
    pub data: Vec<u8>,
}

/// One network packet: where it goes and the commands it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Npdu {
    pub asn: u16,
    pub graph: u16,
    pub destination: u16,
    pub source: u16,
    pub response: bool,
    pub sequence: u8,
    pub commands: Vec<Command>,
}

impl Npdu {
    /// A packet, refusing more commands than one DLPDU carries.
    ///
    /// # Errors
    /// Commands over [`MAX_COMMANDS`] in all.
    pub fn new(destination: u16, source: u16, commands: Vec<Command>) -> Result<Self> {
        let total: usize = commands.iter().map(|c| 3 + c.data.len()).sum();
        if total > MAX_COMMANDS {
            return Err(protocol_error("more commands than one packet carries"));
        }
        Ok(Self {
            asn: 0,
            graph: 0,
            destination,
            source,
            response: false,
            sequence: 0,
            commands,
        })
    }

    /// Stamped with the slot it goes in and the transport sequence.
    #[must_use]
    pub const fn in_slot(mut self, asn: u16, sequence: u8) -> Self {
        self.asn = asn;
        self.sequence = sequence & 0x1f;
        self
    }

    /// The answer to this packet, back the way it came.
    ///
    /// # Errors
    /// As [`Self::new`].
    pub fn answering(&self, commands: Vec<Command>) -> Result<Self> {
        let mut answer = Self::new(self.source, self.destination, commands)?;
        answer.response = true;
        answer.sequence = self.sequence;
        answer.graph = self.graph;
        Ok(answer)
    }

    /// The packet as the DLPDU carries it.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![CONTROL, TTL];
        out.extend_from_slice(&self.asn.to_be_bytes());
        out.extend_from_slice(&self.graph.to_be_bytes());
        out.extend_from_slice(&self.destination.to_be_bytes());
        out.extend_from_slice(&self.source.to_be_bytes());
        out.extend_from_slice(&[SECURITY, self.sequence, 0, 0, 0, 0]);
        let transport = ACKNOWLEDGED | if self.response { RESPONSE } else { 0 } | self.sequence;
        out.push(transport);
        for command in &self.commands {
            out.extend_from_slice(&command.number.to_be_bytes());
            out.push(u8::try_from(command.data.len()).unwrap_or(u8::MAX));
            out.extend_from_slice(&command.data);
        }
        out
    }

    /// The packet `bytes` carry.
    ///
    /// # Errors
    /// A packet cut off, a control byte this crate does not carry, or a
    /// command cut off before its byte count is met.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (head, mut rest) = bytes
            .split_at_checked(OVERHEAD)
            .ok_or_else(|| protocol_error("a packet cut off inside its headers"))?;
        if head[0] != CONTROL || head[10] != SECURITY {
            return Err(protocol_error("a control byte this crate does not carry"));
        }
        let transport = head[16];
        let mut commands = Vec::new();
        while !rest.is_empty() {
            let (command, after) = rest
                .split_at_checked(3)
                .ok_or_else(|| protocol_error("a command cut off inside its number"))?;
            let (data, after) = after
                .split_at_checked(usize::from(command[2]))
                .ok_or_else(|| protocol_error("a command cut off before its byte count"))?;
            commands.push(Command {
                number: u16::from_be_bytes([command[0], command[1]]),
                data: data.to_vec(),
            });
            rest = after;
        }
        Ok(Self {
            asn: u16::from_be_bytes([head[2], head[3]]),
            graph: u16::from_be_bytes([head[4], head[5]]),
            destination: u16::from_be_bytes([head[6], head[7]]),
            source: u16::from_be_bytes([head[8], head[9]]),
            response: transport & RESPONSE != 0,
            sequence: transport & 0x1f,
            commands,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packet_carries_its_commands_and_reads_back() {
        let packet = Npdu::new(
            dlpdu::GATEWAY,
            0x0001,
            vec![
                Command {
                    number: 0,
                    data: Vec::new(),
                },
                Command {
                    number: 130,
                    data: vec![0xc0, 1, 2, 3],
                },
            ],
        )
        .expect("packet")
        .in_slot(0x1234, 7);
        let bytes = packet.encode();
        assert_eq!(&bytes[..10], &[0, 0x20, 0x12, 0x34, 0, 0, 0xf9, 0x81, 0, 1]);
        assert_eq!(bytes[16], 0x87, "acknowledged, sequence seven");
        assert_eq!(&bytes[17..20], &[0, 0, 0]);
        assert_eq!(&bytes[20..23], &[0, 130, 4]);
        assert_eq!(Npdu::decode(&bytes).expect("decode"), packet);
        let answer = packet
            .answering(vec![Command {
                number: 130,
                data: vec![0, 0],
            }])
            .expect("answer");
        assert!(answer.response && answer.sequence == 7);
        assert_eq!((answer.destination, answer.source), (1, dlpdu::GATEWAY));
        assert_eq!(
            Npdu::decode(&answer.encode()).expect("decode").commands,
            answer.commands
        );
        assert!(bytes.len() <= dlpdu::MAX_PAYLOAD);
    }

    #[test]
    fn what_is_not_a_packet_is_refused() {
        let bytes = Npdu::new(
            1,
            2,
            vec![Command {
                number: 1,
                data: vec![9],
            }],
        )
        .expect("packet")
        .encode();
        assert!(Npdu::decode(&bytes[..10]).is_err(), "cut off");
        assert!(Npdu::decode(&bytes[..19]).is_err(), "inside a number");
        assert!(Npdu::decode(&bytes[..20]).is_err(), "before the count");
        let mut bad = bytes;
        bad[0] = 0x80;
        assert!(Npdu::decode(&bad).is_err(), "a proxy route");
        let full = Command {
            number: 130,
            data: vec![0; MAX_COMMAND_DATA],
        };
        assert!(Npdu::new(1, 2, vec![full.clone()]).is_ok());
        let over = Command {
            number: 130,
            data: vec![0; MAX_COMMAND_DATA + 1],
        };
        assert!(Npdu::new(1, 2, vec![over]).is_err());
        assert!(
            Npdu::new(1, 2, vec![full.clone(), full]).is_err(),
            "two full"
        );
    }
}
