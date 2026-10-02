//! Fetch's bad ports (Fetch §2.9, port blocking): the ports of services
//! that a request crafted by a page could talk to as if it were their
//! own client, which no fetch connects to.

/// The bad ports, in order.
pub const BAD_PORTS: [u16; 83] = [
    0, 1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 69, 77, 79, 87, 95, 101,
    102, 103, 104, 109, 110, 111, 113, 115, 117, 119, 123, 135, 137, 139, 143, 161, 179, 389, 427,
    465, 512, 513, 514, 515, 526, 530, 531, 532, 540, 548, 554, 556, 563, 587, 601, 636, 989, 990,
    993, 995, 1719, 1720, 1723, 2049, 3659, 4045, 4190, 5060, 5061, 6000, 6566, 6665, 6666, 6667,
    6668, 6669, 6679, 6697, 10080,
];

/// Whether `port` is a bad port.
pub fn is_bad(port: u16) -> bool {
    BAD_PORTS.binary_search(&port).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_sorted_and_tells_good_from_bad() {
        assert!(BAD_PORTS.windows(2).all(|pair| pair[0] < pair[1]));
        for port in BAD_PORTS {
            assert!(is_bad(port), "{port}");
        }
        for port in [80, 443, 8080, 8443, 3000, 2, 24, 10079, 10081, u16::MAX] {
            assert!(!is_bad(port), "{port}");
        }
    }
}
