//! Time division: the network's clock counts ten-millisecond slots from
//! when it started — the absolute slot number — and a superframe is a
//! repeating cycle of them, each slot a link between two nicknames in one
//! direction. A DLPDU goes in the next slot linked its way, and the packet
//! inside it carries the slot's number.

/// One slot: ten milliseconds.
pub const SLOT: std::time::Duration = std::time::Duration::from_millis(10);

/// Which way a slot's link goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// From the device to the gateway.
    Up,
    /// From the gateway to the device.
    Down,
}

/// A superframe of alternating links, and where the clock is in it.
#[derive(Clone, Debug)]
pub struct Superframe {
    slots: u16,
    asn: u64,
}

impl Superframe {
    /// A superframe of `slots` slots — at least two, one each way — with
    /// the even slots linked up and the odd ones down, starting at the
    /// network's first slot.
    #[must_use]
    pub const fn new(slots: u16) -> Self {
        Self {
            slots: if slots < 2 { 2 } else { slots },
            asn: 0,
        }
    }

    /// The absolute slot number the clock is at.
    #[must_use]
    pub const fn asn(&self) -> u64 {
        self.asn
    }

    /// Which way the link in slot `asn` goes.
    #[must_use]
    pub const fn direction_of(&self, asn: u64) -> Direction {
        if (asn % self.slots as u64).is_multiple_of(2) {
            Direction::Up
        } else {
            Direction::Down
        }
    }

    /// Take the next slot linked `direction`: the clock moves past it.
    pub fn next_slot(&mut self, direction: Direction) -> u64 {
        while self.direction_of(self.asn) != direction {
            self.asn += 1;
        }
        let taken = self.asn;
        self.asn += 1;
        taken
    }

    /// The low sixteen bits of `asn`: what a packet carries of it.
    #[must_use]
    pub const fn snippet(asn: u64) -> u16 {
        (asn & 0xffff) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_alternate_and_the_clock_only_moves_forward() {
        let mut superframe = Superframe::new(4);
        assert_eq!(superframe.next_slot(Direction::Up), 0);
        assert_eq!(superframe.next_slot(Direction::Up), 2, "slot one goes down");
        assert_eq!(superframe.next_slot(Direction::Down), 3);
        assert_eq!(superframe.next_slot(Direction::Down), 5);
        assert_eq!(superframe.asn(), 6);
        assert_eq!(Superframe::new(1).slots, 2, "one each way at least");
        assert_eq!(Superframe::snippet(0x1_2345), 0x2345);
        assert_eq!(SLOT.as_millis(), 10);
    }
}
