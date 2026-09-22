//! The data link protocol data unit as the 802.15.4 radio carries it: the
//! MAC header — frame control, sequence, PAN, two short addresses — then
//! the DLPDU header of IEC 62591 — an address specifier, a sequence, the
//! network identifier, destination and source nicknames, the DLPDU
//! specifier — the payload, a four-byte MIC, and the CRC-16 the MAC ends
//! with. Data and acknowledgement are the two kinds this crate carries.
//!
//! The MIC is AES-CCM* under the network key the network manager hands out
//! at join. This crate carries the field; a loopback radio, holding no key,
//! leaves it zero, and a keyed link fills and checks it.

use transport::error::{Result, protocol_error};

/// The most a PHY packet holds.
pub const MAX_PHY: usize = 127;
/// The MAC header with short addresses and its check sequence.
pub const MAC_OVERHEAD: usize = 9 + 2;
/// The DLPDU header and its MIC.
pub const DLPDU_OVERHEAD: usize = 9 + 4;
/// What one DLPDU carries: the network layer's whole packet.
pub const MAX_PAYLOAD: usize = MAX_PHY - MAC_OVERHEAD - DLPDU_OVERHEAD;

/// The gateway's well-known nickname.
pub const GATEWAY: u16 = 0xf981;
/// The network manager's well-known nickname.
pub const NETWORK_MANAGER: u16 = 0xf980;

/// Frame control: data frame, PAN identifier compression, short addresses.
const DATA_FRAME: [u8; 2] = [0x41, 0x88];
/// Address specifier: both nicknames short.
const SHORT_ADDRESSES: u8 = 0x00;
/// DLPDU specifier: a data DLPDU at normal priority.
const SPECIFIER_DATA: u8 = 0x07;
/// DLPDU specifier: an acknowledgement.
const SPECIFIER_ACK: u8 = 0x00;

/// What a DLPDU is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Data,
    Ack,
}

/// One DLPDU between two nicknames on one network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dlpdu {
    pub kind: Kind,
    pub sequence: u8,
    pub network: u16,
    pub destination: u16,
    pub source: u16,
    pub payload: Vec<u8>,
}

impl Dlpdu {
    /// A DLPDU, refusing more than the PHY holds.
    ///
    /// # Errors
    /// A payload over [`MAX_PAYLOAD`].
    pub fn new(
        kind: Kind,
        sequence: u8,
        network: u16,
        destination: u16,
        source: u16,
        payload: &[u8],
    ) -> Result<Self> {
        if payload.len() > MAX_PAYLOAD {
            return Err(protocol_error("more than one DLPDU carries"));
        }
        Ok(Self {
            kind,
            sequence,
            network,
            destination,
            source,
            payload: payload.to_vec(),
        })
    }

    /// The DLPDU as the radio carries it, check sequence last.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = DATA_FRAME.to_vec();
        out.push(self.sequence);
        out.extend_from_slice(&self.network.to_le_bytes());
        out.extend_from_slice(&self.destination.to_le_bytes());
        out.extend_from_slice(&self.source.to_le_bytes());
        out.push(SHORT_ADDRESSES);
        out.push(self.sequence);
        out.extend_from_slice(&self.network.to_be_bytes());
        out.extend_from_slice(&self.destination.to_be_bytes());
        out.extend_from_slice(&self.source.to_be_bytes());
        out.push(match self.kind {
            Kind::Data => SPECIFIER_DATA,
            Kind::Ack => SPECIFIER_ACK,
        });
        out.extend_from_slice(&self.payload);
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&crc16(&out).to_le_bytes());
        out
    }

    /// Exactly one DLPDU, its check sequence checked.
    ///
    /// # Errors
    /// More than the PHY holds, a frame cut off, not a data frame, a check
    /// sequence that does not check, or a specifier this crate does not
    /// carry.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_PHY {
            return Err(protocol_error("more than the PHY holds"));
        }
        if bytes.len() < MAC_OVERHEAD + DLPDU_OVERHEAD {
            return Err(protocol_error("a DLPDU cut off inside its headers"));
        }
        let (body, check) = bytes.split_at(bytes.len() - 2);
        if crc16(body) != u16::from_le_bytes([check[0], check[1]]) {
            return Err(protocol_error("a check sequence that does not check"));
        }
        if body[..2] != DATA_FRAME {
            return Err(protocol_error("not a data frame with short addresses"));
        }
        let header = &body[9..18];
        if header[0] != SHORT_ADDRESSES {
            return Err(protocol_error("long addresses this crate does not carry"));
        }
        let kind = match header[8] & 0x07 {
            SPECIFIER_DATA => Kind::Data,
            SPECIFIER_ACK => Kind::Ack,
            other => {
                return Err(protocol_error(format!(
                    "a DLPDU this crate does not carry: {other}"
                )));
            }
        };
        Ok(Self {
            kind,
            sequence: header[1],
            network: u16::from_be_bytes([header[2], header[3]]),
            destination: u16::from_be_bytes([header[4], header[5]]),
            source: u16::from_be_bytes([header[6], header[7]]),
            payload: body[18..body.len() - 4].to_vec(),
        })
    }
}

/// The 802.15.4 frame check sequence, which every technology on that radio
/// shares.
pub use transport::crc::kermit as crc16;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_data_dlpdu_reads_back_with_its_check_sequence() {
        assert_eq!(crc16(b"123456789"), 0x2189, "CRC-16/KERMIT");
        let dlpdu = Dlpdu::new(Kind::Data, 5, 0x1234, GATEWAY, 0x0001, b"hello").expect("dlpdu");
        let bytes = dlpdu.encode();
        assert_eq!(bytes.len(), MAC_OVERHEAD + DLPDU_OVERHEAD + 5);
        assert_eq!(&bytes[..2], &DATA_FRAME);
        assert_eq!(&bytes[11..18], &[0x12, 0x34, 0xf9, 0x81, 0x00, 0x01, 0x07]);
        assert_eq!(&bytes[23..27], &[0; 4], "the MIC a keyed link fills");
        assert_eq!(Dlpdu::decode(&bytes).expect("decode"), dlpdu);
        let ack = Dlpdu::new(Kind::Ack, 5, 0x1234, 0x0001, GATEWAY, &[]).expect("ack");
        assert_eq!(Dlpdu::decode(&ack.encode()).expect("decode"), ack);
    }

    #[test]
    fn what_is_not_a_dlpdu_is_refused() {
        let bytes = Dlpdu::new(Kind::Data, 1, 1, GATEWAY, 2, b"x")
            .expect("dlpdu")
            .encode();
        assert!(Dlpdu::decode(&bytes[..20]).is_err(), "cut off");
        assert!(Dlpdu::decode(&[0; MAX_PHY + 1]).is_err(), "too long");
        let mut bad = bytes.clone();
        bad[19] ^= 1;
        assert!(Dlpdu::decode(&bad).is_err(), "check sequence");
        let resealed = |mut frame: Vec<u8>| {
            frame.truncate(frame.len() - 2);
            let crc = crc16(&frame).to_le_bytes();
            frame.extend_from_slice(&crc);
            frame
        };
        let mut bad = bytes.clone();
        bad[0] = 0x40;
        assert!(Dlpdu::decode(&resealed(bad)).is_err(), "not data");
        let mut bad = bytes.clone();
        bad[9] = 0x88;
        assert!(Dlpdu::decode(&resealed(bad)).is_err(), "long addresses");
        let mut bad = bytes;
        bad[17] = 0x01;
        assert!(Dlpdu::decode(&resealed(bad)).is_err(), "an advertisement");
        assert!(Dlpdu::new(Kind::Data, 1, 1, 1, 1, &[0; MAX_PAYLOAD + 1]).is_err());
        assert!(Dlpdu::new(Kind::Data, 1, 1, 1, 1, &[0; MAX_PAYLOAD]).is_ok());
    }
}
