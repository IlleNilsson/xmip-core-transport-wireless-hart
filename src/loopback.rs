//! A field device on an in-process superframe, and both ends of one
//! `WirelessHART` exchange on it (ADR-0051).
//!
//! [`LoopbackRadio`] is the air every test and every box without a gateway
//! drives: what the gateway transmits in an up slot the device answers in
//! the next down slot, and the answer is what the gateway receives next.
//! The loopback pair is a gateway on that air and the device it writes a
//! Stream to; the far end is the device holding it, read back a chunk at a
//! time. One air, one thread: the round goes in order, as hart's does.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use hart::device::{self, Device, Identity};
use transport::error::Result;
use transport::held::Held;
use transport::loopback::{FarEnd, LOOPBACK_TIMEOUT, Loopback};
use transport::{Arrived, Transport};

use crate::dlpdu::{self, Dlpdu, Kind};
use crate::npdu::{Command, Npdu};
use crate::tdma::{Direction, Superframe};
use crate::{MAX_CHUNK, Radio, WirelessHartTransport};

/// A field device on an in-process superframe: what the gateway transmits
/// in an up slot the device answers in the next down slot, and the answer
/// is what the gateway receives next.
pub struct LoopbackRadio {
    device: Arc<Device>,
    nickname: u16,
    network: u16,
    superframe: Mutex<Superframe>,
    to_gateway: Mutex<VecDeque<Vec<u8>>>,
}

impl LoopbackRadio {
    /// `device` at `nickname` on network `network`, in a superframe of a
    /// hundred slots, answering reads in chunks a DLPDU carries.
    #[must_use]
    pub fn new(device: Device, nickname: u16, network: u16) -> Self {
        Self {
            device: Arc::new(device.chunking(MAX_CHUNK)),
            nickname,
            network,
            superframe: Mutex::new(Superframe::new(100)),
            to_gateway: Mutex::new(VecDeque::new()),
        }
    }

    #[must_use]
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// The absolute slot number the network's clock is at.
    #[must_use]
    pub fn asn(&self) -> u64 {
        self.superframe
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .asn()
    }

    fn slot(&self, direction: Direction) -> u64 {
        self.superframe
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next_slot(direction)
    }

    fn queue(&self, packet: &Npdu, sequence: u8) -> Result<()> {
        let asn = self.slot(Direction::Down);
        let packet = packet
            .clone()
            .in_slot(Superframe::snippet(asn), packet.sequence);
        let dlpdu = Dlpdu::new(
            Kind::Data,
            sequence,
            self.network,
            dlpdu::GATEWAY,
            self.nickname,
            &packet.encode(),
        )?;
        self.to_gateway
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(dlpdu.encode());
        Ok(())
    }

    /// The device publishes its primary variable, asked by nobody: command
    /// 1's answer in the next down slot.
    ///
    /// # Errors
    /// Never on this radio; the signature is the trait's.
    pub fn publish(&self) -> Result<()> {
        let answer = Command {
            number: device::READ_PRIMARY_VARIABLE,
            data: self.device.answer(device::READ_PRIMARY_VARIABLE, &[]),
        };
        let mut packet = Npdu::new(dlpdu::GATEWAY, self.nickname, vec![answer])?;
        packet.response = true;
        self.queue(&packet, 0)
    }
}

impl Radio for LoopbackRadio {
    fn name(&self) -> &'static str {
        "loopback"
    }

    fn transmit(&self, direction: Direction, bytes: &[u8]) -> Result<u64> {
        let asn = self.slot(direction);
        let dlpdu = Dlpdu::decode(bytes)?;
        if dlpdu.kind != Kind::Data
            || dlpdu.network != self.network
            || dlpdu.destination != self.nickname
        {
            return Ok(asn);
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
        self.queue(&packet.answering(answers)?, dlpdu.sequence)?;
        Ok(asn)
    }

    fn receive(&self, _timeout: Duration) -> Result<Option<Vec<u8>>> {
        Ok(self
            .to_gateway
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front())
    }
}

impl WirelessHartTransport {
    /// Both ends on one superframe: a gateway and the device at nickname
    /// `0001` that answers it, on a fresh [`LoopbackRadio`], the loopback
    /// timeout on the gateway.
    #[must_use]
    pub fn loopback() -> Self {
        let device = Device::new(Identity {
            manufacturer: 0x26,
            device_type: 0xe5,
            device_id: 0x0a_1b2c,
        });
        let radio = Arc::new(LoopbackRadio::new(device, 0x0001, 0x1234));
        Self::new(radio, 0x1234, 0x0001).timing_out_after(LOOPBACK_TIMEOUT)
    }
}

impl Loopback for WirelessHartTransport {
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

    fn unblock(&self, _address: &str) {
        // The air is in-process; nothing listens on a socket.
    }

    /// In order on one thread: the device lives in the radio and answers in
    /// the slot after each request, so the write goes first and the
    /// read-back finds what it left.
    fn round(&self, payload: &[u8]) -> Result<Arrived> {
        self.round_in_order(payload)
    }
}
