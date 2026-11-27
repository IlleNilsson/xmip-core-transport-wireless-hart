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
//! commands, and the device on the air, whose slots a gateway and a device
//! share. The device, its identity, commands 0 and 1, and the device-
//! specific commands that write and read a Stream in chunks are hart's
//! ([`hart::device`]); this crate takes them as they are, chunked to what a
//! DLPDU carries. A Send Location writes a Stream to a device; a Receive
//! Location takes what the device publishes, or reads the Stream it holds.
//!
//! **Acceptance is at-most-once here** ([`AT_MOST_ONCE`]): a receive takes
//! what a device publishes, whole, and published data is answered by nobody
//! above the link layer, whose acknowledgement the slot already carried.
//!
//! The radio is the capability's `ieee802154::Radio`, and the DLPDU's
//! header is the frame's MAC header, the network the PAN and the nicknames
//! short addresses: a Location's address names an interface of Linux's
//! kernel, `whart://wpan0`, its `network` the interface's PAN and the
//! gateway the interface's own short address; or `loopback`, a device on an
//! in-process superframe ([`LoopbackRadio`]), which every test and every box
//! without a `WirelessHART` gateway drives, the way hart drives its loopback
//! line. The kernel's MAC sends when the air is clear, not in a slot: the
//! slots are the device's own count (open problem 233). The origin URI names
//! the radio and the device: `whart://loopback/0001?command=131`.
//!
//! **Built with its `declaration` feature alone** the crate is
//! [`declaration`].

#[cfg(feature = "declaration")]
pub mod declaration;
#[cfg(feature = "default")]
pub mod dlpdu;
#[cfg(feature = "default")]
pub mod loopback;
#[cfg(feature = "default")]
pub mod npdu;
#[cfg(feature = "default")]
pub mod tdma;

#[cfg(feature = "default")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "default")]
use std::time::Duration;
#[cfg(feature = "default")]
use xcore::lock::Locked;

#[cfg(feature = "default")]
use context::property::WIRELESS_HART_NICKNAME;
#[cfg(feature = "declaration")]
pub use declaration::TIMEOUT;
#[cfg(feature = "default")]
pub use dlpdu::{Dlpdu, Kind};
#[cfg(feature = "default")]
use hart::device::{self, Identity};
#[cfg(feature = "default")]
pub use loopback::{LoopbackRadio, OnTheAir};
#[cfg(feature = "default")]
use net::Target;
#[cfg(feature = "default")]
pub use npdu::{Command, Npdu};
#[cfg(feature = "default")]
pub use tdma::{Direction, Superframe};
#[cfg(feature = "default")]
use transport::error::{Result, TransportError, protocol_error};
#[cfg(feature = "default")]
use transport::ieee802154::{self, Radio};
#[cfg(feature = "default")]
use transport::{Acknowledgement, Arrived, Configured, Directions, Taken, Transport, Waiting};
#[cfg(feature = "default")]
use xcore::settings::{Read, Settings};

/// Why a published packet cannot be acknowledged after the receive cycle.
#[cfg(feature = "default")]
pub const AT_MOST_ONCE: &str = "WirelessHART published data is answered by nobody above the \
                                link layer, whose acknowledgement the slot already carried";

/// What one chunk of a Stream carries: the most a command carries in one
/// packet, less the flags byte and, on the way back, the two status bytes.
#[cfg(feature = "default")]
pub const MAX_CHUNK: usize = npdu::MAX_COMMAND_DATA - 3;

/// The gateway's side of a radio, talking to one device.
#[cfg(feature = "default")]
#[derive(Clone)]
pub struct WirelessHartTransport {
    radio: Arc<dyn Radio>,
    /// The gateway's own nickname: [`dlpdu::GATEWAY`], or its interface's
    /// short address.
    own: u16,
    nickname: u16,
    sequence: Arc<Mutex<u8>>,
    timeout: Duration,
}

#[cfg(feature = "default")]
impl WirelessHartTransport {
    /// A gateway on `radio` at the well-known [`dlpdu::GATEWAY`], talking to
    /// the device at `nickname` on the radio's network.
    #[must_use]
    pub fn new(radio: Arc<dyn Radio>, nickname: u16) -> Self {
        Self {
            radio,
            own: dlpdu::GATEWAY,
            nickname,
            sequence: Arc::new(Mutex::new(0)),
            timeout: TIMEOUT,
        }
    }

    /// The gateway at `own` on the air: an interface's own short address.
    #[must_use]
    pub const fn at(mut self, own: u16) -> Self {
        self.own = own;
        self
    }

    /// `whart://<radio>/<nickname>`.
    #[must_use]
    pub fn origin(&self, nickname: u16) -> String {
        format!("whart://{}/{nickname:04x}", self.radio.name())
    }

    fn next_sequence(&self) -> u8 {
        let mut sequence = self.sequence.locked();
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
        let packet = Npdu::new(self.nickname, self.own, vec![asked])?.in_slot(0, sequence);
        let dlpdu = Dlpdu::new(Kind::Data, &packet.encode())?;
        self.radio.transmit(self.nickname, &dlpdu.encode())?;
        loop {
            let heard = self
                .radio
                .receive(self.timeout)?
                .ok_or_else(|| TransportError::retryable("the device did not answer"))?;
            let answer = Npdu::decode(&Dlpdu::decode(&heard.payload)?.payload)?;
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

    /// Read the Stream the device holds, whole, a chunk per request. The
    /// device keeps holding it.
    ///
    /// # Errors
    /// As [`Self::request`], or a device that never says "last".
    pub fn read_stream(&self) -> Result<Taken> {
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
                let nickname = format!("{:04x}", self.nickname);
                return Ok(Taken::new(origin, bytes).observing(WIRELESS_HART_NICKNAME, nickname));
            }
        }
        Err(protocol_error("a Stream that never ends"))
    }

    /// The next packet on the radio as a Stream, whole — what a device
    /// published — or `None` when the air is quiet. Acceptance is
    /// at-most-once ([`AT_MOST_ONCE`]).
    ///
    /// # Errors
    /// Where the radio could not be read or carried something that is not a
    /// packet.
    pub fn receive_one(&self) -> Result<Option<Arrived>> {
        let Some(heard) = self.radio.receive(self.timeout)? else {
            return Ok(None);
        };
        let packet = Npdu::decode(&Dlpdu::decode(&heard.payload)?.payload)?;
        let command = packet
            .commands
            .first()
            .ok_or_else(|| protocol_error("a packet with no command"))?;
        let origin = format!(
            "{}?command={}&asn={}",
            self.origin(heard.source),
            command.number,
            packet.asn
        );
        Ok(Some(
            Arrived::whole(
                origin,
                device::answered(&command.data)?.to_vec(),
                Acknowledgement::at_most_once(AT_MOST_ONCE),
            )
            .observing(WIRELESS_HART_NICKNAME, format!("{:04x}", heard.source)),
        ))
    }
}

#[cfg(feature = "default")]
impl Transport for WirelessHartTransport {
    fn name(&self) -> &'static str {
        "wireless-hart"
    }

    fn directions(&self) -> Directions {
        Directions::BOTH
    }

    fn arrivals(&self) -> transport::Arrivals {
        transport::Arrivals::Ordered("one line or bus, answered in the order it speaks")
    }

    fn waiting(&self) -> Waiting {
        self.radio.waiting()
    }

    /// Nothing on the air is not an error: an empty vector. Acceptance is
    /// at-most-once here: published data is answered by nobody
    /// ([`AT_MOST_ONCE`]).
    fn receive_up_to(&self, _most: usize) -> Result<Vec<Arrived>> {
        Ok(self.receive_one()?.into_iter().collect())
    }

    fn sending(&self) -> transport::Sending {
        transport::Sending::Gathered("one WirelessHART frame carries the whole body")
    }

    /// `target` may name a device, `whart://radio/0001`, overriding the
    /// transport's.
    fn send_from(&self, target: &str, outgoing: transport::Outgoing<'_>) -> Result<()> {
        let bytes: &[u8] = &outgoing.gather()?;
        let nickname = match Target::under(Self::SCHEMES, target)
            .map(|named| (named.authority(), named.path()))
        {
            Some((_, nickname)) if !nickname.is_empty() => codec::hex::number(nickname)
                .map_err(|_| protocol_error(format!("{nickname:?} is not a nickname")))?,
            _ => self.nickname,
        };
        Self::new(Arc::clone(&self.radio), nickname)
            .at(self.own)
            .timing_out_after(self.timeout)
            .write_stream(bytes)
    }
}

/// The address is the radio: an interface, whose PAN the Location's
/// `network` must be, or `loopback`.
#[cfg(feature = "default")]
impl Configured for WirelessHartTransport {
    const SETTINGS: &'static Settings = declaration::SETTINGS;
    const SCHEMES: &'static [&'static str] = declaration::SCHEMES;

    fn timeout_kept(&mut self) -> Option<&mut dyn transport::configured::TimeoutKept> {
        Some(&mut self.timeout)
    }

    fn configured(address: &str, settings: &Read) -> Result<Self> {
        let sixteen = |name| {
            u16::try_from(settings.integer(name))
                .map_err(|_| protocol_error(format!("a {name} over 0xffff")))
        };
        let (network, nickname) = (sixteen("network")?, sixteen("nickname")?);
        if address == "loopback" {
            return Ok(Self::new(LoopbackRadio::at(nickname), nickname));
        }
        let (radio, at) = ieee802154::interface(address)?;
        ieee802154::agreeing(address, "network", network, at.pan)?;
        Ok(Self::new(radio, nickname).at(at.short))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hart::device::Device;
    use transport::loopback::Loopback;
    use transport::payload::edge_payloads;
    use xcore::settings::{Applies, Given};

    #[test]
    fn wireless_hart_declares_its_settings_and_reads_through_them() {
        assert_eq!(
            WirelessHartTransport::SETTINGS.problems(),
            Vec::<String>::new()
        );
        let given = [
            ("network".to_string(), Given::Integer(0x1234)),
            ("nickname".to_string(), Given::Integer(1)),
        ];
        let built = WirelessHartTransport::open("loopback", Applies::Send, &given).expect("built");
        assert_eq!((built.own, built.nickname), (dlpdu::GATEWAY, 1));
        assert_eq!(built.timeout, TIMEOUT);
        assert!(
            built.waiting().polled(),
            "the loopback radio returns at once"
        );
        let Err(refused) = WirelessHartTransport::open("loopback", Applies::Receive, &given[..1])
        else {
            panic!("the nickname is required");
        };
        assert!(refused.message.contains("nickname"), "{}", refused.message);
        assert!(WirelessHartTransport::open("gateway-1", Applies::Send, &given).is_err());
    }

    /// The shapes a protocol breaks on, as the System Tests list them.
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

    fn radio() -> Arc<LoopbackRadio> {
        let device = Device::new(Identity {
            manufacturer: 1,
            device_type: 2,
            device_id: 3,
        });
        Arc::new(LoopbackRadio::new(device, 0x0001))
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
        let radio = radio();
        let gateway = WirelessHartTransport::new(Arc::clone(&radio) as Arc<dyn Radio>, 1);
        let identity = gateway.identify().expect("identify");
        assert_eq!(identity, *radio.device().identity());
        assert_eq!(radio.asn(), 2, "one slot up, one down");
        assert!(
            gateway.receive().expect("quiet").is_empty(),
            "nothing is not an error"
        );
        radio.publish().expect("publish");
        let mut arrived = gateway.receive().expect("published");
        assert_eq!(arrived.len(), 1);
        let arrived = arrived.remove(0);
        assert!(!arrived.defers(), "published data is at-most-once");
        let arrived = arrived.taken().expect("taken");
        assert_eq!(arrived.bytes, [6, 0x41, 0xa0, 0, 0], "20.0 psi");
        assert_eq!(arrived.origin_uri, "whart://loopback/0001?command=1&asn=3");
    }

    #[test]
    fn a_device_that_is_not_addressed_does_not_answer() {
        let radio = radio();
        let gateway = WirelessHartTransport::new(Arc::clone(&radio) as Arc<dyn Radio>, 1)
            .timing_out_after(Duration::from_millis(10));
        let error = gateway
            .send("whart://loopback/0002", b"x")
            .expect_err("silence");
        assert!(error.retryable, "a device may be slow to answer");
        assert!(
            gateway.send("whart://loopback/zz", b"x").is_err(),
            "not a nickname"
        );
        assert_eq!(radio.asn(), 1, "the up slot passed unanswered");
    }

    /// The device on the air answers what a gateway sends it, and only
    /// that: what a far end on an interface does with each frame it hears.
    #[test]
    fn a_device_on_the_air_answers_a_request_sent_to_it() {
        let air = OnTheAir::new(
            Device::new(Identity {
                manufacturer: 1,
                device_type: 2,
                device_id: 3,
            }),
            0x0002,
        );
        let asked = Npdu::new(
            2,
            1,
            vec![Command {
                number: device::IDENTIFY,
                data: Vec::new(),
            }],
        )
        .expect("packet");
        let bytes = Dlpdu::new(Kind::Data, &asked.encode())
            .expect("dlpdu")
            .encode();
        assert_eq!(
            air.answer(3, &bytes).expect("heard"),
            None,
            "another nickname"
        );
        let answer = air.answer(2, &bytes).expect("heard").expect("answered");
        let answer = Npdu::decode(&Dlpdu::decode(&answer).expect("dlpdu").payload).expect("npdu");
        assert!(answer.response);
        assert_eq!((answer.destination, answer.source), (1, 2));
        let ack = Dlpdu::new(Kind::Ack, &[]).expect("ack").encode();
        assert_eq!(air.answer(2, &ack).expect("heard"), None, "no data");
    }
}
