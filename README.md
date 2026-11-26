# xmip-core-transport-wireless-hart

WirelessHART transport: the HART command set in TDMA-framed DLPDUs over IEEE 802.15.4 — a Stream written and read in chunks through the device-specific commands hart defines, the network and transport layers around them; a loopback radio and its slots stand in for the gateway. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

A send target is read by `net::Target` in [xmip-core-library-net](https://github.com/IlleNilsson/xmip-core-library-net), the one reading of a URI every technology calls: scheme, authority, path and decoded query. Until 2026-09-28 it was read through the transport capability's `socket::target`, which split it on its first slash and left the query in the path.

A `0x` number in a target is read by `codec::hex::prefixed_number` in [xmip-core-library-codec](https://github.com/IlleNilsson/xmip-core-library-codec), which refuses a sign; until 2026-09-28 it was read with `from_str_radix`, which took `0x+7e8`.

## Acknowledgement

Acceptance is at-most-once here. A receive takes what a device publishes, and
published data is answered by nobody above the link layer, whose
acknowledgement the slot already carried before the gateway read it: nobody
is left to tell how the receive cycle ended. Each published variable arrives
whole.

## The declaration

What a Location of this technology is configured with is two `const`s in its `declaration` module, `src/declaration.rs`, beside the code, which `Configured` reads (ADR-0064, amendment 2026-11-26): `declaration::SETTINGS`, `network`, `nickname` and `timeout`, and `SCHEMES`, `whart`; `timeout`'s default, `declaration::TIMEOUT`, is declared there too, the code takes it from there, and the crate root names it as before. Built with its `declaration` feature alone (`--no-default-features --features declaration`), the crate is those and nothing more: its code and every crate the code is built on — the hart technology, whose command set it carries, among them — are behind the default feature. So the library the surfaces load can carry it without the code, for the language server and the desktop to suggest and check a WirelessHART Location's settings though no node in their process loaded the technology, once the gathering crate of open problem 48 links it. `test/Declaration.Test.ps1` in the estate holds that the declaration builds nothing the transport capability does not.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
