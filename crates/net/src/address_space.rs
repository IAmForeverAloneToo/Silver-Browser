//! IP address spaces (Local Network Access, decision D19): every address
//! a connection would be made to is loopback, local or public, and a
//! page may not reach into a space less public than its own.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Where an address is, from the most private to the most public.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AddressSpace {
    /// This machine.
    Loopback,
    /// The local network: private, link-local and shared ranges.
    Local,
    /// Everything else.
    Public,
}

impl AddressSpace {
    /// The space `address` is in, by Local Network Access's table. An
    /// IPv4-mapped IPv6 address is classified by its IPv4 address.
    pub fn of(address: IpAddr) -> AddressSpace {
        match address {
            IpAddr::V4(v4) => of_v4(v4),
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => of_v4(v4),
                None => of_v6(v6),
            },
        }
    }

    /// Whether a page in this space may make a request to an address in
    /// `target`: not when the target is less public, so a public page
    /// cannot reach the local network or this machine, and a local page
    /// cannot reach this machine.
    pub fn may_request(self, target: AddressSpace) -> bool {
        target >= self
    }
}

impl fmt::Display for AddressSpace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AddressSpace::Loopback => "loopback",
            AddressSpace::Local => "local",
            AddressSpace::Public => "public",
        })
    }
}

/// Whether `address` is in `network/prefix`.
fn in_v4(address: Ipv4Addr, network: [u8; 4], prefix: u32) -> bool {
    let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
    u32::from(address) & mask == u32::from(Ipv4Addr::from(network)) & mask
}

fn in_v6(address: Ipv6Addr, network: [u16; 8], prefix: u32) -> bool {
    let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
    u128::from(address) & mask == u128::from(Ipv6Addr::from(network)) & mask
}

fn of_v4(address: Ipv4Addr) -> AddressSpace {
    // 0.0.0.0 itself reaches this machine; the rest of 0.0.0.0/8 is
    // local, so the exact address is looked at first.
    if in_v4(address, [127, 0, 0, 0], 8)
        || address == Ipv4Addr::UNSPECIFIED
        || in_v4(address, [198, 18, 0, 0], 15)
    {
        return AddressSpace::Loopback;
    }
    let local = [
        ([10, 0, 0, 0], 8),
        ([172, 16, 0, 0], 12),
        ([192, 168, 0, 0], 16),
        ([100, 64, 0, 0], 10),
        ([169, 254, 0, 0], 16),
        ([0, 0, 0, 0], 8),
    ];
    if local
        .iter()
        .any(|&(network, prefix)| in_v4(address, network, prefix))
    {
        AddressSpace::Local
    } else {
        AddressSpace::Public
    }
}

fn of_v6(address: Ipv6Addr) -> AddressSpace {
    if address == Ipv6Addr::LOCALHOST || address == Ipv6Addr::UNSPECIFIED {
        return AddressSpace::Loopback;
    }
    let local = [
        ([0xfc00, 0, 0, 0, 0, 0, 0, 0], 7),
        ([0xfe80, 0, 0, 0, 0, 0, 0, 0], 10),
        ([0xfec0, 0, 0, 0, 0, 0, 0, 0], 10),
        ([0x2001, 0x0db8, 0, 0, 0, 0, 0, 0], 32),
        ([0x3fff, 0, 0, 0, 0, 0, 0, 0], 20),
    ];
    if local
        .iter()
        .any(|&(network, prefix)| in_v6(address, network, prefix))
    {
        AddressSpace::Local
    } else {
        AddressSpace::Public
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AddressSpace::*;

    fn space(address: &str) -> AddressSpace {
        AddressSpace::of(address.parse().expect(address))
    }

    #[test]
    fn the_table() {
        for (address, expected) in [
            ("127.0.0.1", Loopback),
            ("127.255.255.254", Loopback),
            ("0.0.0.0", Loopback),
            ("198.18.0.1", Loopback),
            ("198.19.255.255", Loopback),
            ("::1", Loopback),
            ("::", Loopback),
            ("10.1.2.3", Local),
            ("172.16.0.1", Local),
            ("172.31.255.255", Local),
            ("192.168.1.1", Local),
            ("100.64.0.1", Local),
            ("100.127.255.255", Local),
            ("169.254.169.254", Local),
            ("0.1.2.3", Local),
            ("fc00::1", Local),
            ("fdff::1", Local),
            ("fe80::1", Local),
            ("febf::1", Local),
            ("fec0::1", Local),
            ("2001:db8::1", Local),
            ("3fff:0fff::1", Local),
            ("8.8.8.8", Public),
            ("172.15.255.255", Public),
            ("172.32.0.0", Public),
            ("100.128.0.0", Public),
            ("198.20.0.0", Public),
            ("198.17.255.255", Public),
            ("2606:4700::1111", Public),
            ("3fff:1000::1", Public),
            ("2001:db9::1", Public),
        ] {
            assert_eq!(space(address), expected, "{address}");
        }
    }

    #[test]
    fn mapped_addresses_are_their_ipv4_address() {
        assert_eq!(space("::ffff:127.0.0.1"), Loopback);
        assert_eq!(space("::ffff:192.168.0.1"), Local);
        assert_eq!(space("::ffff:8.8.8.8"), Public);
    }

    #[test]
    fn requests_only_go_as_public_or_more() {
        assert!(Public.may_request(Public));
        assert!(!Public.may_request(Local));
        assert!(!Public.may_request(Loopback));
        assert!(Local.may_request(Public));
        assert!(Local.may_request(Local));
        assert!(!Local.may_request(Loopback));
        assert!(Loopback.may_request(Loopback));
        assert!(Loopback.may_request(Local));
        assert!(Loopback.may_request(Public));
    }
}
