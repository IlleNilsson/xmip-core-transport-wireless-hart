# xmip-core-transport-wireless-hart

WirelessHART transport: the HART command set in TDMA-framed DLPDUs over IEEE 802.15.4 — a Stream written and read in chunks through the device-specific commands hart defines, the network and transport layers around them; a loopback radio and its slots stand in for the gateway. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
