//! The data link protocol data unit of IEC 62591 as it rides an 802.15.4
//! frame. Its header is the MAC header — frame control with its address
//! specifier, the sequence, the network identifier as the PAN, destination
//! and source nicknames as short addresses — and its CRC-16 the MAC's check
//! sequence, both the radio's (`transport::ieee802154`); what the frame
//! carries between them is the DLPDU specifier, the payload and a four-byte
//! MIC. Data and acknowledgement are the two kinds this crate carries.
//!
//! The MIC is AES-CCM* under the network key the network manager hands out
//! at join. This crate carries the field; a radio holding no key leaves it
//! zero, and a keyed link fills and checks it. Until 2026-11-27 the crate
//! wrote a MAC header of its own and then the same fields again.

use transport::error::{Result, protocol_error};

/// The DLPDU specifier and the MIC.
const DLPDU_OVERHEAD: usize = 1 + 4;
/// What one DLPDU carries: the network layer's whole packet, in what one
/// 802.15.4 frame carries above its MAC.
pub const MAX_PAYLOAD: usize = net::ieee802154::MAX_PAYLOAD - DLPDU_OVERHEAD;

/// The gateway's well-known nickname.
pub const GATEWAY: u16 = 0xf981;

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

/// One DLPDU: its kind and what it carries, the addresses its frame's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dlpdu {
    pub kind: Kind,
    pub payload: Vec<u8>,
}

impl Dlpdu {
    /// A DLPDU, refusing more than one frame carries.
    ///
    /// # Errors
    /// A payload over [`MAX_PAYLOAD`].
    pub fn new(kind: Kind, payload: &[u8]) -> Result<Self> {
        if payload.len() > MAX_PAYLOAD {
            return Err(protocol_error("more than one DLPDU carries"));
        }
        Ok(Self {
            kind,
            payload: payload.to_vec(),
        })
    }

    /// The DLPDU as its frame carries it, after the MAC header.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![match self.kind {
            Kind::Data => SPECIFIER_DATA,
            Kind::Ack => SPECIFIER_ACK,
        }];
        out.extend_from_slice(&self.payload);
        out.extend_from_slice(&[0; 4]);
        out
    }

    /// Exactly one DLPDU, as a frame carried it.
    ///
    /// # Errors
    /// More than a frame carries, a DLPDU cut off, or a specifier this
    /// crate does not carry.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_PAYLOAD + DLPDU_OVERHEAD {
            return Err(protocol_error("more than one frame carries"));
        }
        let (specifier, rest) = bytes
            .split_first()
            .filter(|(_, rest)| rest.len() >= 4)
            .ok_or_else(|| protocol_error("a DLPDU cut off inside its specifier or MIC"))?;
        let kind = match specifier & 0x07 {
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
            payload: rest[..rest.len() - 4].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_data_dlpdu_reads_back_with_its_mic() {
        let dlpdu = Dlpdu::new(Kind::Data, b"hello").expect("dlpdu");
        let bytes = dlpdu.encode();
        assert_eq!(bytes.len(), DLPDU_OVERHEAD + 5);
        assert_eq!(bytes[0], 0x07, "data at normal priority");
        assert_eq!(&bytes[6..], &[0; 4], "the MIC a keyed link fills");
        assert_eq!(Dlpdu::decode(&bytes).expect("decode"), dlpdu);
        let ack = Dlpdu::new(Kind::Ack, &[]).expect("ack");
        assert_eq!(Dlpdu::decode(&ack.encode()).expect("decode"), ack);
        assert_eq!(MAX_PAYLOAD, 111, "116 above the MAC, less five");
    }

    #[test]
    fn what_is_not_a_dlpdu_is_refused() {
        let bytes = Dlpdu::new(Kind::Data, b"x").expect("dlpdu").encode();
        assert!(Dlpdu::decode(&bytes[..4]).is_err(), "cut off");
        assert!(Dlpdu::decode(&[]).is_err(), "empty");
        assert!(
            Dlpdu::decode(&[0; MAX_PAYLOAD + DLPDU_OVERHEAD + 1]).is_err(),
            "too long"
        );
        let mut bad = bytes;
        bad[0] = 0x01;
        assert!(Dlpdu::decode(&bad).is_err(), "an advertisement");
        assert!(Dlpdu::new(Kind::Data, &[0; MAX_PAYLOAD + 1]).is_err());
        assert!(Dlpdu::new(Kind::Data, &[0; MAX_PAYLOAD]).is_ok());
    }
}
