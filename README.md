# xmip-core-transport-wireless-hart

WirelessHART transport: the HART command set in TDMA-framed DLPDUs over IEEE 802.15.4 — a Stream written and read in chunks through the device-specific commands hart defines, the network and transport layers around them; an 802.15.4 interface of Linux's kernel, or a loopback radio and its slots in process. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

A send target is read by `net::Target` in [xmip-core-library-net](https://github.com/IlleNilsson/xmip-core-library-net), the one reading of a URI every technology calls: scheme, authority, path and decoded query. Until 2026-09-28 it was read through the transport capability's `socket::target`, which split it on its first slash and left the query in the path.

A `0x` number in a target is read by `codec::hex::prefixed_number` in [xmip-core-library-codec](https://github.com/IlleNilsson/xmip-core-library-codec), which refuses a sign; until 2026-09-28 it was read with `from_str_radix`, which took `0x+7e8`.

## The radio

The radio is the transport capability's `ieee802154::Radio`. IEC 62591's
DLPDU header is the 802.15.4 MAC header — the network identifier its PAN,
the nicknames its short addresses — and its CRC the MAC's check sequence,
so both are the radio's: this crate hands it the DLPDU specifier, the
network packet and the MIC (`src/dlpdu.rs`), 111 bytes of packet where the
crate's own second header left 103, to the device's nickname, and takes
the answer back with the nickname that sent it (ADR-0051, amendment
2026-11-27). A Location's address names the radio:

- **an interface of Linux's kernel**, `whart://wpan0`, an `AF_IEEE802154`
  datagram socket bound to the interface's own PAN and short address
  (`net::ieee802154::Interface`), which a user opens with no capability:
  the gateway is the interface's short address, and the Location's
  `network` must be its PAN; another is refused, naming both. Elsewhere,
  and on a Linux whose kernel carries no 802.15.4 sockets, it is refused in
  words, so `xmip-service` links WirelessHART on Linux alone;
- **`loopback`**, a device on an in-process superframe
  (`loopback::LoopbackRadio`), the gateway at the well-known `0xF981`.

The device is `loopback::OnTheAir` on either: what a gateway sends it in an
up slot it answers in the next down slot, the slots its own count. Proven
through a node on Linux's two virtual radios, `mac802154_hwsim`'s `wpan0`
and `wpan1`, by the estate's service test (`aired.rs`), the device on the
far interface answering every request until it holds the Stream. The
kernel's MAC sends when the air is clear, with no slot, no channel hopping
and no network clock, so the TDMA timing is not proven (open problem 233).
Until 2026-11-27 the crate wrote a MAC header of its own and the same
fields again after it, and had no radio but the loopback.

## Acknowledgement

Acceptance is at-most-once here. A receive takes what a device publishes, and
published data is answered by nobody above the link layer, whose
acknowledgement the slot already carried before the gateway read it: nobody
is left to tell how the receive cycle ended. Each published variable arrives
whole.

## The declaration

What a Location of this technology is configured with is two `const`s in its `declaration` module, `src/declaration.rs`, beside the code, which `Configured` reads (ADR-0064, amendment 2026-11-26): `declaration::SETTINGS`, `network`, `nickname` and `timeout`, and `SCHEMES`, `whart`; `timeout`'s default, `declaration::TIMEOUT`, is declared there too, the code takes it from there, and the crate root names it as before. Built with its `declaration` feature alone (`--no-default-features --features declaration`), the crate is those and nothing more: its code and every crate the code is built on — the hart technology, whose command set it carries, among them — are behind the default feature. So the library the surfaces load carries it without the code, through the gathering crate xmip-core-declaration (open problem 48), and the language server and the desktop suggest and check a WirelessHART Location's settings though no node in their process loaded the technology. `test/Declaration.Test.ps1` in the estate holds that the declaration builds nothing the transport capability does not.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
