//! Offline MAC OUI vendor lookup.
//!
//! The appliance is local-first: it never queries an external OUI service. A
//! small static table of IEEE OUI prefixes for well-known smart-home, network
//! and consumer vendors gives discovered devices a friendly label in the UI.
//!
//! The table is best effort and only used for display; a missing or unknown
//! prefix simply leaves the vendor empty.

/// Normalize a MAC address to lower-case colon notation.
///
/// Accepts colon, hyphen, dot or space separated groups and bare hex strings.
/// Returns `None` when the value does not contain exactly 12 hexadecimal digits.
///
/// # Examples
/// ```
/// use hs_discovery::vendor::normalize_mac;
///
/// assert_eq!(
///     normalize_mac("AA-BB-CC-DD-EE-FF"),
///     Some("aa:bb:cc:dd:ee:ff".to_owned())
/// );
/// assert_eq!(normalize_mac("not-a-mac"), None);
/// ```
#[must_use]
pub fn normalize_mac(mac: &str) -> Option<String> {
    let mut hex = String::with_capacity(12);
    for character in mac.chars() {
        if character.is_ascii_hexdigit() {
            hex.push(character.to_ascii_lowercase());
        } else if !matches!(character, ':' | '-' | '.' | ' ') {
            return None;
        }
    }
    if hex.len() != 12 {
        return None;
    }
    let groups: Vec<&str> = (0..6).map(|index| &hex[index * 2..index * 2 + 2]).collect();
    Some(groups.join(":"))
}

/// Resolve the vendor of a MAC address through the built-in OUI table.
///
/// The MAC is normalized first, so any common separator style is accepted.
/// Unknown prefixes and malformed values return `None`.
///
/// # Examples
/// ```
/// use hs_discovery::vendor::vendor_for_mac;
///
/// assert_eq!(vendor_for_mac("B8:27:EB:12:34:56"), Some("Raspberry Pi"));
/// assert_eq!(vendor_for_mac("02:00:00:00:00:01"), None);
/// ```
#[must_use]
pub fn vendor_for_mac(mac: &str) -> Option<&'static str> {
    let normalized = normalize_mac(mac)?;
    let prefix = normalized.get(..8)?;
    OUI_TABLE
        .iter()
        .find(|(oui, _vendor)| *oui == prefix)
        .map(|(_oui, vendor)| *vendor)
}

/// Built-in OUI prefix table: `(aa:bb:cc, vendor label)`.
///
/// Prefixes are lower-case colon notation and unique. The list intentionally
/// covers the ecosystems the appliance integrates with (Home Assistant, Sonos,
/// Apple Home, Google Home, Amazon Alexa, Shelly, Tasmota/Sonoff, Tuya, ...).
const OUI_TABLE: &[(&str, &str)] = &[
    // Apple
    ("00:03:93", "Apple"),
    ("00:1b:63", "Apple"),
    ("3c:07:54", "Apple"),
    ("8c:85:90", "Apple"),
    ("a4:83:e7", "Apple"),
    ("f0:18:98", "Apple"),
    // Samsung
    ("00:12:fb", "Samsung"),
    ("00:1e:10", "Samsung"),
    ("34:23:ba", "Samsung"),
    ("8c:77:12", "Samsung"),
    // Espressif (ESP8266/ESP32 modules used by most DIY IoT devices)
    ("24:0a:c4", "Espressif"),
    ("24:62:ab", "Espressif"),
    ("3c:61:05", "Espressif"),
    ("84:f3:eb", "Espressif"),
    ("ac:67:b2", "Espressif"),
    ("b4:e6:2d", "Espressif"),
    ("cc:50:e3", "Espressif"),
    ("dc:4f:22", "Espressif"),
    // Shelly / Allterco Robotics
    ("34:94:54", "Shelly"),
    ("98:cd:ac", "Shelly"),
    // Philips Hue / Signify
    ("00:17:88", "Philips Hue"),
    ("ec:b5:fa", "Philips Hue"),
    // Sonoff / ITEAD
    ("3c:71:bf", "Sonoff"),
    // Tuya
    ("18:69:d8", "Tuya"),
    ("50:02:91", "Tuya"),
    // Google / Nest
    ("3c:5a:b4", "Google"),
    ("54:60:09", "Google"),
    ("f4:f5:d8", "Google"),
    ("18:b4:30", "Nest"),
    ("64:16:66", "Nest"),
    // Amazon / Ring / Echo
    ("00:fc:8b", "Amazon"),
    ("0c:47:c9", "Amazon"),
    ("44:65:0d", "Amazon"),
    ("68:54:fd", "Amazon"),
    ("54:e0:19", "Ring"),
    // Sonos
    ("00:0e:58", "Sonos"),
    ("5c:aa:fd", "Sonos"),
    ("78:28:ca", "Sonos"),
    // Xiaomi / Mijia
    ("00:9e:c8", "Xiaomi"),
    ("28:6c:07", "Xiaomi"),
    ("64:b4:73", "Xiaomi"),
    ("78:11:dc", "Xiaomi"),
    // Aqara / Lumi
    ("04:cf:8c", "Aqara"),
    ("54:ef:44", "Aqara"),
    ("00:15:8d", "Aqara"),
    // TP-Link / Kasa / Tapo
    ("00:1d:0f", "TP-Link"),
    ("14:cc:20", "TP-Link"),
    ("50:64:2b", "TP-Link"),
    ("a4:2b:b0", "TP-Link"),
    // Broadlink
    ("34:ea:34", "Broadlink"),
    ("78:a5:04", "Broadlink"),
    // Roku
    ("00:0d:4b", "Roku"),
    ("ac:3a:7a", "Roku"),
    ("b0:a7:37", "Roku"),
    // LG
    ("00:1e:75", "LG"),
    ("10:f9:6f", "LG"),
    ("58:a2:b5", "LG"),
    // Sony
    ("00:13:a9", "Sony"),
    ("30:f9:ed", "Sony"),
    ("78:84:3c", "Sony"),
    // Huawei
    ("00:e0:fc", "Huawei"),
    ("28:6e:d4", "Huawei"),
    ("48:46:fb", "Huawei"),
    // Raspberry Pi
    ("b8:27:eb", "Raspberry Pi"),
    ("dc:a6:32", "Raspberry Pi"),
    ("e4:5f:01", "Raspberry Pi"),
    // Intel
    ("00:1b:21", "Intel"),
    ("3c:97:0e", "Intel"),
    ("7c:7a:91", "Intel"),
    // Realtek
    ("00:e0:4c", "Realtek"),
    // Ubiquiti / UniFi
    ("04:18:d6", "Ubiquiti"),
    ("24:a4:3c", "Ubiquiti"),
    ("44:d9:e7", "Ubiquiti"),
    ("78:8a:20", "Ubiquiti"),
    ("f0:9f:c2", "Ubiquiti"),
    // iRobot
    ("50:14:79", "iRobot"),
    // Belkin / Wemo
    ("94:10:3e", "Belkin"),
    // Wyze
    ("2c:aa:8e", "Wyze"),
    // LIFX
    ("68:c6:3a", "LIFX"),
    // Honeywell
    ("00:d0:2d", "Honeywell"),
];

#[cfg(test)]
mod tests {
    use super::{normalize_mac, vendor_for_mac};

    #[test]
    fn normalizes_common_mac_formats() {
        for value in [
            "AA:BB:CC:DD:EE:FF",
            "aa-bb-cc-dd-ee-ff",
            "aabb.ccdd.eeff",
            "aa bb cc dd ee ff",
            "aabbccddeeff",
        ] {
            assert_eq!(
                normalize_mac(value),
                Some("aa:bb:cc:dd:ee:ff".to_owned()),
                "{value} should normalize"
            );
        }
    }

    #[test]
    fn rejects_malformed_macs() {
        for value in ["", "00:11:22", "zz:yy:xx:ww:vv:uu", "00:11:22:33:44:55:66"] {
            assert_eq!(normalize_mac(value), None, "{value} should be malformed");
        }
    }

    #[test]
    fn resolves_known_vendors() {
        assert_eq!(vendor_for_mac("B8:27:EB:12:34:56"), Some("Raspberry Pi"));
        assert_eq!(vendor_for_mac("ac-67-b2-aa-bb-cc"), Some("Espressif"));
        assert_eq!(vendor_for_mac("00:17:88:00:00:01"), Some("Philips Hue"));
        assert_eq!(vendor_for_mac("34:94:54:12:34:56"), Some("Shelly"));
        assert_eq!(vendor_for_mac("3c:71:bf:00:00:01"), Some("Sonoff"));
        assert_eq!(vendor_for_mac("54:ef:44:00:00:01"), Some("Aqara"));
        assert_eq!(vendor_for_mac("b0:a7:37:00:00:01"), Some("Roku"));
        assert_eq!(vendor_for_mac("00:0e:58:00:00:01"), Some("Sonos"));
    }

    #[test]
    fn unknown_and_malformed_vendors_are_none() {
        assert_eq!(vendor_for_mac("02:00:00:00:00:01"), None);
        assert_eq!(vendor_for_mac("ff:ff:ff:ff:ff:ff"), None);
        assert_eq!(vendor_for_mac("not-a-mac"), None);
        assert_eq!(vendor_for_mac(""), None);
    }

    #[test]
    fn oui_prefixes_are_unique_and_well_formed() {
        let mut seen = std::collections::BTreeSet::new();
        for (prefix, vendor) in super::OUI_TABLE {
            assert_eq!(prefix.len(), 8, "bad OUI prefix {prefix}");
            assert!(prefix.chars().all(|c| c.is_ascii_hexdigit() || c == ':'));
            assert!(!vendor.is_empty());
            assert!(seen.insert(prefix), "duplicate OUI prefix {prefix}");
        }
        assert!(
            seen.len() >= 25,
            "the OUI table must cover at least 25 prefixes"
        );
    }
}
