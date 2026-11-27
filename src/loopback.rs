//! A field device on the air, and both ends of one `WirelessHART` exchange
//! in process (ADR-0051).
//!
//! [`OnTheAir`] is the device as a radio hears it: what a gateway sends it
//! in an up slot it answers in the next down slot, on a superframe of its
//! own. [`LoopbackRadio`] is the air every test and every box without a
//! gateway drives, the device on it, and its answer what the gateway
//! receives next; a far end on an 802.15.4 interface answers through the
//! same device over the kernel's MAC. The loopback pair is a gateway on that
//! air and the device it writes a Stream to; the far end is the device
//! holding it, read back a chunk at a time. One air, one thread: the round
//! goes in order, as hart's does.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xcore::lock::Locked;

use hart::device::{self, Device, Identity};
use net::ieee802154::Heard;
use transport::ArrivalIdentity;
use transport::Configured;
use transport::Transport;
use transport::Waiting;
use transport::error::Result;
use transport::held::Held;
use transport::ieee802154::Radio;
use transport::loopback::{FarEnd, LOOPBACK_TIMEOUT, Loopback};

use crate::dlpdu::{self, Dlpdu, Kind};
use crate::npdu::{Command, Npdu};
use crate::tdma::{Direction, Superframe};
use crate::{MAX_CHUNK, WirelessHartTransport};

/// A field device on the air at `nickname`: what a gateway sends it in an
/// up slot it answers in the next down slot, on a superframe of a hundred
/// slots, a read answered in chunks a DLPDU carries.
pub struct OnTheAir {
    device: Arc<Device>,
    nickname: u16,
    superframe: Mutex<Superframe>,
}

impl OnTheAir {
    #[must_use]
    pub fn new(device: Device, nickname: u16) -> Self {
        Self {
            device: Arc::new(device.chunking(MAX_CHUNK)),
            nickname,
            superframe: Mutex::new(Superframe::new(100)),
        }
    }

    /// The device a Location whose radio is `loopback` talks to, and a far
    /// end on an interface stands, at `nickname`.
    #[must_use]
    pub fn at(nickname: u16) -> Self {
        let device = Device::new(Identity {
            manufacturer: 0x26,
            device_type: 0xe5,
            device_id: 0x0a_1b2c,
        });
        Self::new(device, nickname)
    }

    #[must_use]
    pub fn device(&self) -> &Device {
        &self.device
    }

    #[must_use]
    pub const fn nickname(&self) -> u16 {
        self.nickname
    }

    /// The absolute slot number the network's clock is at.
    #[must_use]
    pub fn asn(&self) -> u64 {
        self.superframe.locked().asn()
    }

    fn slot(&self, direction: Direction) -> u64 {
        self.superframe.locked().next_slot(direction)
    }

    /// What the device answers `bytes`, a DLPDU sent to `destination` in the
    /// next up slot: the answering DLPDU, in the next down slot; `None`
    /// where it was sent to another nickname or carries no data.
    ///
    /// # Errors
    /// A DLPDU or a packet that does not read.
    pub fn answer(&self, destination: u16, bytes: &[u8]) -> Result<Option<Vec<u8>>> {
        self.slot(Direction::Up);
        let dlpdu = Dlpdu::decode(bytes)?;
        if dlpdu.kind != Kind::Data || destination != self.nickname {
            return Ok(None);
        }
        let packet = Npdu::decode(&dlpdu.payload)?;
        let answers = packet
            .commands
            .iter()
            .map(|command| Command {
                number: command.number,
                data: self.device.answer(command.number, &command.data),
            })
            .collect();
        self.in_next_down_slot(&packet.answering(answers)?)
            .map(Some)
    }

    /// The device's primary variable, published asked by nobody: command
    /// 1's answer to the gateway, in the next down slot.
    ///
    /// # Errors
    /// Never for one command; the signature is the packet's.
    pub fn published(&self) -> Result<Vec<u8>> {
        let answer = Command {
            number: device::READ_PRIMARY_VARIABLE,
            data: self.device.answer(device::READ_PRIMARY_VARIABLE, &[]),
        };
        let mut packet = Npdu::new(dlpdu::GATEWAY, self.nickname, vec![answer])?;
        packet.response = true;
        self.in_next_down_slot(&packet)
    }

    fn in_next_down_slot(&self, packet: &Npdu) -> Result<Vec<u8>> {
        let asn = self.slot(Direction::Down);
        let packet = packet
            .clone()
            .in_slot(Superframe::snippet(asn), packet.sequence);
        Ok(Dlpdu::new(Kind::Data, &packet.encode())?.encode())
    }
}

/// A field device on an in-process superframe: what the gateway transmits
/// it answers, and the answer is what the gateway receives next, from the
/// device's nickname.
pub struct LoopbackRadio {
    air: OnTheAir,
    to_gateway: Mutex<VecDeque<Heard>>,
}

impl LoopbackRadio {
    /// `device` at `nickname`, on a superframe of its own.
    #[must_use]
    pub fn new(device: Device, nickname: u16) -> Self {
        Self {
            air: OnTheAir::new(device, nickname),
            to_gateway: Mutex::new(VecDeque::new()),
        }
    }

    #[must_use]
    pub fn device(&self) -> &Device {
        self.air.device()
    }

    /// The absolute slot number the network's clock is at.
    #[must_use]
    pub fn asn(&self) -> u64 {
        self.air.asn()
    }

    /// The device publishes its primary variable, asked by nobody.
    ///
    /// # Errors
    /// Never on this radio; the signature is the packet's.
    pub fn publish(&self) -> Result<()> {
        let published = self.air.published()?;
        self.heard(published);
        Ok(())
    }

    fn heard(&self, payload: Vec<u8>) {
        self.to_gateway.locked().push_back(Heard {
            source: self.air.nickname(),
            payload,
        });
    }

    /// The loopback's device at `nickname`, on a fresh superframe: what a
    /// Location whose radio is `loopback` talks to ([`OnTheAir::at`]).
    #[must_use]
    pub fn at(nickname: u16) -> Arc<dyn Radio> {
        Arc::new(Self {
            air: OnTheAir::at(nickname),
            to_gateway: Mutex::new(VecDeque::new()),
        })
    }
}

impl Radio for LoopbackRadio {
    fn name(&self) -> &'static str {
        "loopback"
    }

    fn transmit(&self, destination: u16, bytes: &[u8]) -> Result<()> {
        if let Some(answer) = self.air.answer(destination, bytes)? {
            self.heard(answer);
        }
        Ok(())
    }

    fn receive(&self, _timeout: Duration) -> Result<Option<Heard>> {
        Ok(self.to_gateway.locked().pop_front())
    }

    fn waiting(&self) -> Waiting {
        Waiting::Polled("a read of the loopback radio returns at once")
    }
}

impl WirelessHartTransport {
    /// Both ends on one superframe: a gateway and the device at nickname
    /// `0001` that answers it, on a fresh [`LoopbackRadio`], the loopback
    /// timeout on the gateway.
    #[must_use]
    pub fn loopback() -> Self {
        Self::new(LoopbackRadio::at(0x0001), 0x0001).timing_out_after(LOOPBACK_TIMEOUT)
    }
}

impl Loopback for WirelessHartTransport {
    fn arrival_identity(&self) -> ArrivalIdentity {
        ArrivalIdentity::Named(&[context::property::WIRELESS_HART_NICKNAME])
    }

    /// The device on the air, holding what the gateway wrote until it is read
    /// back.
    fn far_end(&self) -> Result<Box<dyn FarEnd>> {
        let gateway = self.clone();
        Ok(Box::new(Held::new(self.origin(self.nickname), move || {
            gateway.read_stream()
        })))
    }

    fn send_to(&self, address: &str, payload: &[u8]) -> Result<()> {
        self.clone().send(address, payload)
    }

    /// In order on one thread: the device lives in the radio and answers in
    /// the slot after each request, so the write goes first and the
    /// read-back finds what it left.
    fn exchanges_in_order(&self) -> bool {
        true
    }
}
