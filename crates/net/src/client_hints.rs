//! User-Agent client hints (2026-11-20): what the reference browser
//! says of itself beyond its user agent string, `navigator.userAgentData`
//! and the `Sec-CH-UA` fields, with this browser named among its brands
//! as Brave names itself: a GREASE brand, "Silver" and "Chromium", in the
//! order and the GREASE form Chromium derives from its major version
//! (`components/embedder_support/user_agent_utils.cc`; probed against
//! Chrome 158: "Chromium", "Not.A/Brand" 24, the browser's brand; 157:
//! "Not-A.Brand" 99, the browser's brand, "Chromium").

use crate::fetch::{CHROME_FULL, CHROME_MAJOR};

/// This browser's brand.
pub const BRAND: &str = "Silver";

/// Chromium's GREASE characters and versions.
const GREASEY_CHARS: [&str; 11] = [" ", "(", ":", "-", ".", "/", ")", ";", "=", "?", "_"];
const GREASED_VERSIONS: [&str; 3] = ["8", "99", "24"];

/// The orders Chromium puts three brands in, by its seed.
const ORDERS: [[usize; 3]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [1, 2, 0],
    [2, 0, 1],
    [2, 1, 0],
];

/// The major version as a number, the seed of the GREASE brand.
fn seed() -> usize {
    CHROME_MAJOR.parse().unwrap_or(0)
}

/// The brands and their versions, in Chromium's order for the major
/// version: majors only, or full versions (`fullVersionList`).
pub fn brands(full: bool) -> Vec<(String, String)> {
    brands_for(seed(), full)
}

fn brands_for(seed: usize, full: bool) -> Vec<(String, String)> {
    let chars = GREASEY_CHARS.len();
    let grease = format!(
        "Not{}A{}Brand",
        GREASEY_CHARS[seed % chars],
        GREASEY_CHARS[(seed + 1) % chars]
    );
    let grease_version = GREASED_VERSIONS[seed % GREASED_VERSIONS.len()];
    let (grease_version, version) = if full {
        (format!("{grease_version}.0.0.0"), CHROME_FULL.to_owned())
    } else {
        (grease_version.to_owned(), seed.to_string())
    };
    let order = ORDERS[seed % ORDERS.len()];
    let mut list = vec![(String::new(), String::new()); 3];
    list[order[0]] = (grease, grease_version);
    list[order[1]] = ("Chromium".to_owned(), version.clone());
    list[order[2]] = (BRAND.to_owned(), version);
    list
}

/// `Sec-CH-UA`: the brands with their majors, as a structured list.
pub fn sec_ch_ua() -> String {
    brands(false)
        .iter()
        .map(|(brand, version)| format!("\"{brand}\";v=\"{version}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `Sec-CH-UA-Mobile`: a desktop browser's.
pub const SEC_CH_UA_MOBILE: &str = "?0";

/// The platform the hints name, the reference's on this system.
pub fn platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Linux"
    }
}

/// `Sec-CH-UA-Platform`.
pub fn sec_ch_ua_platform() -> String {
    format!("\"{}\"", platform())
}

/// The architecture and bitness the hints name.
pub fn architecture() -> (&'static str, &'static str) {
    let architecture = if cfg!(any(target_arch = "aarch64", target_arch = "arm")) {
        "arm"
    } else {
        "x86"
    };
    let bitness = if cfg!(target_pointer_width = "64") {
        "64"
    } else {
        "32"
    };
    (architecture, bitness)
}

/// Windows' Universal API Contract version by the first build that has
/// it (Microsoft's table, as Chromium reports `platformVersion` on
/// Windows 10 and later: "10.0.0" on 19045).
const CONTRACTS: [(u32, u32); 13] = [
    (10240, 1),
    (10586, 2),
    (14393, 3),
    (15063, 4),
    (16299, 5),
    (17134, 6),
    (17763, 7),
    (18362, 8),
    (19041, 10),
    (20348, 12),
    (22000, 14),
    (22621, 15),
    (26100, 19),
];

/// `platformVersion` on Windows for build `build`: the contract version
/// of the newest row at or below it, `<major>.0.0`; a build past the
/// table takes the table's last; one before Windows 10 "0.0.0".
pub fn windows_platform_version(build: u32) -> String {
    let major = CONTRACTS
        .iter()
        .take_while(|(first, _)| *first <= build)
        .last()
        .map_or(0, |(_, contract)| *contract);
    format!("{major}.0.0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_brands_are_chromiums_for_the_major_version() {
        // Chrome 158 (probed 2026-11-24): "Chromium", "Not.A/Brand" 24, its
        // brand; full versions 158.0.8127.37 and the GREASE's 24.0.0.0.
        assert_eq!(
            brands_for(158, false),
            [
                ("Chromium".to_owned(), "158".to_owned()),
                ("Not.A/Brand".to_owned(), "24".to_owned()),
                (BRAND.to_owned(), "158".to_owned()),
            ]
        );
        assert_eq!(
            brands(true),
            [
                ("Chromium".to_owned(), "158.0.8127.37".to_owned()),
                ("Not.A/Brand".to_owned(), "24.0.0.0".to_owned()),
                (BRAND.to_owned(), "158.0.8127.37".to_owned()),
            ]
        );
        // Chrome 157 (probed): "Not-A.Brand" 99, its brand, "Chromium".
        assert_eq!(
            brands_for(157, false),
            [
                ("Not-A.Brand".to_owned(), "99".to_owned()),
                (BRAND.to_owned(), "157".to_owned()),
                ("Chromium".to_owned(), "157".to_owned()),
            ]
        );
        // 156's GREASE (probed in Chromium 156): "Not:A-Brand" 8, first.
        assert_eq!(
            brands_for(156, false)[0],
            ("Not:A-Brand".to_owned(), "8".to_owned())
        );
        assert_eq!(brands_for(156, false)[1].0, "Chromium");
        let full = brands(true);
        assert!(
            full.iter()
                .any(|(brand, version)| brand == BRAND && version == CHROME_FULL)
        );
        assert!(full.iter().any(|(_, version)| version.ends_with(".0.0.0")));
        assert_eq!(
            sec_ch_ua(),
            format!(
                "\"Chromium\";v=\"{CHROME_MAJOR}\", \"Not.A/Brand\";v=\"24\", \"{BRAND}\";v=\"{CHROME_MAJOR}\""
            )
        );
    }

    #[test]
    fn a_windows_build_names_its_contract_version() {
        assert_eq!(windows_platform_version(19045), "10.0.0");
        assert_eq!(windows_platform_version(22631), "15.0.0");
        assert_eq!(windows_platform_version(26100), "19.0.0");
        // Past the table: its last; before Windows 10: none.
        assert_eq!(windows_platform_version(99999), "19.0.0");
        assert_eq!(windows_platform_version(9600), "0.0.0");
    }
}
