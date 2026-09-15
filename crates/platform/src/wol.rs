//! Wake-on-LAN: waking a powered-off PC with a "magic packet".
//!
//! A PC that is switched off runs no software, so nothing we install can wake it — that part is a
//! hard limit (see `docs/FEATURES.md`). What *can* wake it is the network card, if the PC's BIOS/UEFI
//! has "Wake on LAN" (sometimes "Power on by PCI-E") enabled and Windows' driver is set to allow it.
//! Those are one-time settings the school's IT makes on each PC; nothing here can turn them on
//! remotely. Given that, waking is just a matter of putting the right bytes on the wire.
//!
//! The "magic packet" (AMD's 1995 spec, universally supported) is: six `0xFF` bytes, then the
//! target's 6-byte MAC repeated sixteen times — 102 bytes — sent as a UDP broadcast. Any PC on the
//! same LAN can send it, which is the point: the teacher's Console, or an Agent that is already
//! awake in the same room, broadcasts it and the sleeping PC's card sees its own MAC and powers on.
//!
//! Building the packet is pure and unit-tested. Sending it is plain `std::net` (UDP broadcast needs
//! no admin rights).

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

/// A six-byte hardware (MAC) address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacAddress(pub [u8; 6]);

impl MacAddress {
    /// Parses `AA:BB:CC:DD:EE:FF` (or with `-`), case-insensitively.
    ///
    /// # Errors
    /// Returns [`WolError::BadMac`] if it is not six hex bytes.
    pub fn parse(text: &str) -> Result<Self, WolError> {
        let mut bytes = [0u8; 6];
        let parts: Vec<&str> = text.split([':', '-']).collect();
        if parts.len() != 6 {
            return Err(WolError::BadMac);
        }
        for (slot, part) in bytes.iter_mut().zip(parts) {
            *slot = u8::from_str_radix(part.trim(), 16).map_err(|_| WolError::BadMac)?;
        }
        Ok(Self(bytes))
    }

    /// The usual `AA:BB:CC:DD:EE:FF` form.
    #[must_use]
    pub fn to_hex(self) -> String {
        self.0
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    }

    /// Whether this address is one we should never try to wake: all-zero or a broadcast/multicast
    /// address (the low bit of the first byte set). Waking those is meaningless and a loopback risk.
    #[must_use]
    pub fn is_wakeable(self) -> bool {
        self.0 != [0; 6] && self.0[0] & 1 == 0
    }
}

/// Things that can go wrong sending a magic packet.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WolError {
    /// The MAC address text was not six hex bytes.
    #[error("not a valid MAC address")]
    BadMac,
    /// This MAC cannot be a wake target (zero, broadcast or multicast).
    #[error("that is not an address a PC can be woken at")]
    NotWakeable,
    /// The UDP socket failed.
    #[error("could not send the wake packet: {0}")]
    Io(String),
}

/// Builds the 102-byte magic packet for `mac`.
#[must_use]
pub fn magic_packet(mac: MacAddress) -> [u8; 102] {
    let mut packet = [0xFFu8; 102];
    // First six bytes stay 0xFF; then the MAC sixteen times.
    for chunk in packet[6..].chunks_exact_mut(6) {
        chunk.copy_from_slice(&mac.0);
    }
    packet
}

/// The default port. 9 (discard) is the conventional WoL port; the payload, not the port, matters.
pub const DEFAULT_PORT: u16 = 9;

/// Broadcasts a magic packet for `mac` on the local network.
///
/// Sends to the limited broadcast address `255.255.255.255`, so it reaches every PC on the same
/// subnet without needing to know the target's (absent) IP.
///
/// # Errors
/// [`WolError::NotWakeable`] for a nonsense MAC, or [`WolError::Io`] if the socket fails.
pub fn wake(mac: MacAddress) -> Result<(), WolError> {
    if !mac.is_wakeable() {
        return Err(WolError::NotWakeable);
    }
    let socket = UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
        .map_err(|e| WolError::Io(e.to_string()))?;
    socket
        .set_broadcast(true)
        .map_err(|e| WolError::Io(e.to_string()))?;
    let packet = magic_packet(mac);
    let target = SocketAddr::from((Ipv4Addr::BROADCAST, DEFAULT_PORT));
    let sent = socket
        .send_to(&packet, target)
        .map_err(|e| WolError::Io(e.to_string()))?;
    if sent == packet.len() {
        Ok(())
    } else {
        Err(WolError::Io(format!(
            "sent only {sent} of {} bytes",
            packet.len()
        )))
    }
}

/// This PC's own wakeable MAC addresses, so it can tell a Console how to wake it later.
///
/// Filters out loopback, virtual and non-wakeable adapters, so the Console stores addresses that a
/// magic packet can actually reach.
#[must_use]
pub fn local_macs() -> Vec<MacAddress> {
    imp::local_macs()
}

mod imp {
    use super::MacAddress;

    /// `mac_address` enumerates real interfaces cross-platform, without admin rights. We keep only
    /// wakeable addresses, so the Console never stores a loopback or virtual-adapter MAC.
    pub fn local_macs() -> Vec<MacAddress> {
        mac_address::MacAddressIterator::new()
            .map(|iter| {
                iter.map(|m| MacAddress(m.bytes()))
                    .filter(|m| m.is_wakeable())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_magic_packet_is_six_ff_then_the_mac_sixteen_times() {
        let mac = MacAddress([0x1A, 0x2B, 0x3C, 0x4D, 0x5E, 0x6F]);
        let packet = magic_packet(mac);
        assert_eq!(packet.len(), 102);
        assert_eq!(&packet[..6], &[0xFF; 6], "starts with six 0xFF");
        for repeat in 0..16 {
            let start = 6 + repeat * 6;
            assert_eq!(&packet[start..start + 6], &mac.0, "repeat {repeat}");
        }
    }

    #[test]
    fn a_mac_parses_with_either_separator_and_any_case() {
        let expected = MacAddress([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        assert_eq!(MacAddress::parse("AA:BB:CC:DD:EE:FF").unwrap(), expected);
        assert_eq!(MacAddress::parse("aa-bb-cc-dd-ee-ff").unwrap(), expected);
    }

    #[test]
    fn a_mac_round_trips_through_text() {
        let mac = MacAddress([0x01, 0x23, 0x45, 0x67, 0x89, 0xAB]);
        assert_eq!(MacAddress::parse(&mac.to_hex()).unwrap(), mac);
    }

    #[test]
    fn bad_mac_text_is_refused() {
        assert_eq!(MacAddress::parse("nope"), Err(WolError::BadMac));
        assert_eq!(MacAddress::parse("AA:BB:CC"), Err(WolError::BadMac));
        assert_eq!(
            MacAddress::parse("AA:BB:CC:DD:EE:ZZ"),
            Err(WolError::BadMac)
        );
    }

    #[test]
    fn zero_broadcast_and_multicast_addresses_are_not_wakeable() {
        assert!(!MacAddress([0; 6]).is_wakeable());
        assert!(!MacAddress([0xFF; 6]).is_wakeable(), "broadcast");
        assert!(
            !MacAddress([0x01, 0, 0, 0, 0, 0]).is_wakeable(),
            "multicast bit"
        );
        assert!(MacAddress([0x1A, 0x2B, 0x3C, 0x4D, 0x5E, 0x6F]).is_wakeable());
    }

    #[test]
    fn waking_a_nonsense_address_is_refused_before_touching_the_network() {
        assert_eq!(wake(MacAddress([0; 6])), Err(WolError::NotWakeable));
    }
}
