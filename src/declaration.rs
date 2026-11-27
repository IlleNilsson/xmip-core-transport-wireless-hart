//! This technology's declaration: the transport guide's "The declaration",
//! `module/core/capability/transport/doc/adding-a-transport.md`. The wait a
//! Location takes unless told otherwise is declared here, and the code takes
//! it from here.

use std::time::Duration;

use xcore::settings::{Applies, Kind, Presence, Setting, Settings};

/// How long a gateway waits for a device's answer unless told otherwise.
pub const TIMEOUT: Duration = Duration::from_secs(1);

/// Every setting a Location takes beyond its address, the radio:
/// `loopback`, a device on an in-process superframe, is the one the estate
/// has; a gateway joins when it exposes one.
pub const SETTINGS: &Settings = &Settings {
    technology: env!("CARGO_PKG_NAME"),
    settings: &[
        Setting {
            name: "network",
            kind: Kind::Integer {
                minimum: 0,
                maximum: 0xffff,
            },
            presence: Presence::Required,
            meaning: "The network identifier of the mesh the device is on.",
            applies: Applies::Both,
        },
        Setting {
            name: "nickname",
            kind: Kind::Integer {
                minimum: 0,
                maximum: 0xffff,
            },
            presence: Presence::Required,
            meaning: "The device's nickname: the one a Receive Location reads and a Send \
                      Location writes unless its target names another.",
            applies: Applies::Both,
        },
        transport::configured::timeout(
            Applies::Both,
            Some(TIMEOUT),
            "How long a device that does not answer is waited on.",
        ),
    ],
};

/// This technology's schemes, as [`transport::Configured::SCHEMES`] defines
/// them.
pub const SCHEMES: &[&str] = &["whart"];
