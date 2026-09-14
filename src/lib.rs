#![forbid(unsafe_code)]

//! Streams that arrive over `WirelessHART`. One command's data is one
//! Stream: a device's published variable as it comes, or a Stream the
//! device holds, read back a chunk at a time — the same commands hart
//! defines, carried over the air.
//!
//! `WirelessHART` (IEC 62591) is HART without the wire: 802.15.4 radios in
//! a time-slotted, channel-hopping mesh, a gateway at its edge, and the
//! HART command set unchanged above it. What is here is the DLPDU with its
//! TDMA slot, the network and transport layers that carry aggregated
//! commands, and the loopback radio whose slots a gateway and a device
//! share. The device, its identity, commands 0 and 1, and the device-
//! specific commands that write and read a Stream in chunks are hart's
//! ([`hart::device`]); this crate takes them as they are, chunked to what a
//! DLPDU carries. A Send Location writes a Stream to a device; a Receive
//! Location takes what the device publishes, or reads the Stream it holds.
//!
//! The radio is a trait: [`LoopbackRadio`] is a device on an in-process
//! superframe, which every test and every box without a `WirelessHART`
//! gateway drives, the way hart drives its loopback line. The origin URI
//! names the radio and the device: `whart://loopback/0001?command=131`.

pub mod dlpdu;
pub mod loopback;
pub mod npdu;
pub mod tdma;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

pub use dlpdu::{Dlpdu, Kind};
use hart::device::{self, Identity};
pub use loopback::LoopbackRadio;
pub use npdu::{Command, Npdu};
pub use tdma::{Direction, Superframe};
use transport::error::{Result, TransportError, protocol_error};
use transport::{Arrived, Directions, Transport};

/// What one chunk of a Stream carries: the most a command carries in one
/// packet, less the flags byte and, on the way back, the two status bytes.
pub const MAX_CHUNK: usize = npdu::MAX_COMMAND_DATA - 3;

/// Where DLPDUs go and come from: the air, as the gateway hears it.
pub trait Radio: Send + Sync {
    /// The radio's name, for the origin URI.
    fn name(&self) -> &str;
    /// Put a DLPDU in the next slot linked `direction`; the slot it went in.
    ///
    /// # Errors
    /// Where the radio refused it.
    fn transmit(&self, direction: Direction, dlpdu: &[u8]) -> Result<u64>;
    /// The next DLPDU, or `None` when nothing arrived within `timeout`.
    ///
    /// # Errors
    /// Where the radio could not be read.
    fn receive(&self, timeout: Duration) -> Result<Option<Vec<u8>>>;
}

/// The gateway's side of a radio, talking to one device.
#[derive(Clone)]
pub struct WirelessHartTransport {
    radio: Arc<dyn Radio>,
    network: u16,
    nickname: u16,
    sequence: Arc<Mutex<u8>>,
    timeout: Duration,
}

impl WirelessHartTransport {
    /// A gateway on `radio`, talking to the device at `nickname` on network
    /// `network`.
    #[must_use]
    pub fn new(radio: Arc<dyn Radio>, network: u16, nickname: u16) -> Self {
        Self {
            radio,
            network,
            nickname,
            sequence: Arc::new(Mutex::new(0)),
            timeout: Duration::from_secs(1),
        }
    }

    /// Give up on a device that does not answer within `timeout`.
    #[must_use]
    pub const fn timing_out_after(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// `whart://<radio>/<nickname>`.
    #[must_use]
    pub fn origin(&self, nickname: u16) -> String {
        format!("whart://{}/{nickname:04x}", self.radio.name())
    }

    fn next_sequence(&self) -> u8 {
        let mut sequence = self.sequence.lock().unwrap_or_else(PoisonError::into_inner);
        let next = *sequence;
        *sequence = sequence.wrapping_add(1);
        next
    }

    /// Ask the device `command` with `data`; its answer past the status
    /// bytes.
    ///
    /// # Errors
    /// No answer in time, an answer to something else, or a response code
    /// that is not OK.
    pub fn request(&self, command: u16, data: &[u8]) -> Result<Vec<u8>> {
        let sequence = self.next_sequence();
        let asked = Command {
            number: command,
            data: data.to_vec(),
        };
        let packet = Npdu::new(self.nickname, dlpdu::GATEWAY, vec![asked])?.in_slot(0, sequence);
        let dlpdu = Dlpdu::new(
            Kind::Data,
            sequence,
            self.network,
            self.nickname,
            dlpdu::GATEWAY,
            &packet.encode(),
        )?;
        self.radio.transmit(Direction::Up, &dlpdu.encode())?;
        loop {
            let bytes = self
                .radio
                .receive(self.timeout)?
                .ok_or_else(|| TransportError::retryable("the device did not answer"))?;
            let answer = Npdu::decode(&Dlpdu::decode(&bytes)?.payload)?;
            if !answer.response || answer.sequence != packet.sequence {
                continue;
            }
            let answered = answer
                .commands
                .into_iter()
                .find(|c| c.number == command)
                .ok_or_else(|| protocol_error("an answer to another command"))?;
            return device::answered(&answered.data).map(<[u8]>::to_vec);
        }
    }

    /// Command 0: who the device is.
    ///
    /// # Errors
    /// As [`Self::request`].
    pub fn identify(&self) -> Result<Identity> {
        Identity::from_identify(&self.request(device::IDENTIFY, &[])?)
    }

    /// Write `bytes` to the device, a chunk per request.
    ///
    /// # Errors
    /// As [`Self::request`].
    pub fn write_stream(&self, bytes: &[u8]) -> Result<()> {
        for request in device::write_requests(bytes, MAX_CHUNK) {
            self.request(device::WRITE_STREAM, &request)?;
        }
        Ok(())
    }

    /// Read the Stream the device holds, a chunk per request.
    ///
    /// # Errors
    /// As [`Self::request`], or a device that never says "last".
    pub fn read_stream(&self) -> Result<Arrived> {
        let mut bytes = Vec::new();
        for index in 0..u32::MAX {
            let answer = self.request(device::READ_STREAM, &device::read_request(index))?;
            let (last, chunk) = device::read_answer(&answer)?;
            bytes.extend_from_slice(chunk);
            if last {
                let origin = format!(
                    "{}?command={}",
                    self.origin(self.nickname),
                    device::READ_STREAM
                );
                return Ok(Arrived::new(origin, bytes));
            }
        }
        Err(protocol_error("a Stream that never ends"))
    }

    /// The next packet on the radio as a Stream — what a device published —
    /// or `None` when the air is quiet.
    ///
    /// # Errors
    /// Where the radio could not be read or carried something that is not a
    /// packet.
    pub fn receive_one(&self) -> Result<Option<Arrived>> {
        let Some(bytes) = self.radio.receive(self.timeout)? else {
            return Ok(None);
        };
        let dlpdu = Dlpdu::decode(&bytes)?;
        let packet = Npdu::decode(&dlpdu.payload)?;
        let command = packet
            .commands
            .first()
            .ok_or_else(|| protocol_error("a packet with no command"))?;
        let origin = format!(
            "{}?command={}&asn={}",
            self.origin(dlpdu.source),
            command.number,
            packet.asn
        );
        Ok(Some(Arrived::new(
            origin,
            device::answered(&command.data)?.to_vec(),
        )))
    }
}

impl Transport for WirelessHartTransport {
    fn name(&self) -> &'static str {
        "wireless-hart"
    }

    fn directions(&self) -> Directions {
        Directions::BOTH
    }

    /// Nothing on the air is not an error: an empty vector.
    fn receive(&self) -> Result<Vec<Arrived>> {
        Ok(self.receive_one()?.into_iter().collect())
    }

    /// `target` may name a device, `whart://radio/0001`, overriding the
    /// transport's.
    fn send(&self, target: &str, bytes: &[u8]) -> Result<()> {
        let nickname = match transport::socket::target("whart", target) {
            Some((_, nickname)) if !nickname.is_empty() => u16::from_str_radix(nickname, 16)
                .map_err(|_| protocol_error(format!("{nickname:?} is not a nickname")))?,
            _ => self.nickname,
        };
        Self::new(Arc::clone(&self.radio), self.network, nickname)
            .timing_out_after(self.timeout)
            .write_stream(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hart::device::Device;
    use transport::loopback::Loopback;
    use transport::payload::edge_payloads;

    /// The shapes a protocol breaks on, as the Playground lists them.
    fn payloads() -> Vec<(&'static str, Vec<u8>)> {
        let mut payloads = edge_payloads();
        payloads.extend([(
            "sixty-four kibibytes plus one",
            (0..65_537u32)
                .map(|n| u8::try_from(n * 31 % 256).unwrap_or(0))
                .collect(),
        )]);
        payloads
    }

    fn radio(loopback: &WirelessHartTransport) -> Arc<LoopbackRadio> {
        let device = Device::new(Identity {
            manufacturer: 1,
            device_type: 2,
            device_id: 3,
        });
        let _ = loopback;
        Arc::new(LoopbackRadio::new(device, 0x0001, 0x1234))
    }

    #[test]
    fn a_loopback_round_writes_a_stream_over_the_air_and_reads_it_back() {
        let loopback = WirelessHartTransport::loopback();
        let arrived = loopback.round(b"totaliser log").expect("round");
        assert_eq!(arrived.bytes, b"totaliser log");
        assert_eq!(arrived.origin_uri, "whart://loopback/0001?command=131");
        let long = vec![7; 1000];
        assert_eq!(loopback.round(&long).expect("chunks").bytes, long);
        assert!(loopback.ceiling().is_none());
        assert!(loopback.refuses(b"anything").is_none());
        assert_eq!(loopback.name(), "wireless-hart");
        assert!(loopback.directions().receives() && loopback.directions().sends());
        assert!(loopback.claims().is_none());
    }

    #[test]
    fn the_loopback_returns_the_edges_whole() {
        let loopback = WirelessHartTransport::loopback();
        for (name, bytes) in payloads() {
            let arrived = loopback
                .round(&bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(arrived.bytes, bytes, "{name}");
        }
    }

    #[test]
    fn the_device_identifies_itself_and_publishes_in_its_slot() {
        let radio = radio(&WirelessHartTransport::loopback());
        let gateway = WirelessHartTransport::new(Arc::clone(&radio) as Arc<dyn Radio>, 0x1234, 1);
        let identity = gateway.identify().expect("identify");
        assert_eq!(identity, *radio.device().identity());
        assert_eq!(radio.asn(), 2, "one slot up, one down");
        assert!(
            gateway.receive().expect("quiet").is_empty(),
            "nothing is not an error"
        );
        radio.publish().expect("publish");
        let arrived = gateway.receive().expect("published");
        assert_eq!(arrived.len(), 1);
        assert_eq!(arrived[0].bytes, [6, 0x41, 0xa0, 0, 0], "20.0 psi");
        assert_eq!(
            arrived[0].origin_uri,
            "whart://loopback/0001?command=1&asn=3"
        );
    }

    #[test]
    fn a_device_that_is_not_addressed_does_not_answer() {
        let radio = radio(&WirelessHartTransport::loopback());
        let gateway = WirelessHartTransport::new(Arc::clone(&radio) as Arc<dyn Radio>, 0x1234, 1)
            .timing_out_after(Duration::from_millis(10));
        let error = gateway
            .send("whart://loopback/0002", b"x")
            .expect_err("silence");
        assert!(error.retryable, "a device may be slow to answer");
        assert!(
            gateway.send("whart://loopback/zz", b"x").is_err(),
            "not a nickname"
        );
        let elsewhere = WirelessHartTransport::new(radio, 0x9999, 1);
        assert!(elsewhere.identify().is_err(), "another network");
    }
}
