//! Minimal passive BLE scanner, driven straight over HCI.
//!
//! esp-radio hands out raw HCI read/write, which is all a passive scan needs —
//! no host stack. The sequence is Reset, LE Set Scan Parameters, LE Set Scan
//! Enable, then parse LE Advertising Report events as they arrive.
//!
//! This only listens. It never connects to, pairs with, or transmits to any
//! device; a passive scan does not even send scan requests.

use esp_hal::delay::Delay;
use esp_radio::ble::controller::BleConnector;

/// HCI packet indicators.
const HCI_CMD: u8 = 0x01;
const HCI_EVT: u8 = 0x04;

/// Opcodes (OGF/OCF packed little-endian on the wire).
const OP_RESET: u16 = 0x0C03;
const OP_SET_EVENT_MASK: u16 = 0x0C01;
const OP_LE_SET_EVENT_MASK: u16 = 0x2001;
const OP_LE_SET_SCAN_PARAMS: u16 = 0x200B;
const OP_LE_SET_SCAN_ENABLE: u16 = 0x200C;

const EVT_LE_META: u8 = 0x3E;
const EVT_CMD_COMPLETE: u8 = 0x0E;
const SUBEVT_ADV_REPORT: u8 = 0x02;

/// Company identifier assigned to Apple, as it appears in manufacturer data.
pub const APPLE_COMPANY_ID: u16 = 0x004C;

/// Apple manufacturer-data types we can name.
const APPLE_TYPE_FIND_MY: u8 = 0x12;
/// Find My payload length that marks a *separated* accessory — a tag whose
/// owner is not nearby. Phones, tablets and laptops taking part in the Find
/// My network emit the same 0x12 type with a short payload instead, so the
/// length is what separates "an AirTag" from "somebody's iPhone".
const APPLE_FIND_MY_TAG_LEN: u8 = 0x19;
const APPLE_TYPE_NEARBY: u8 = 0x10;
const APPLE_TYPE_IBEACON: u8 = 0x02;
const APPLE_TYPE_AIRDROP: u8 = 0x05;

// 16-bit service UUIDs that identify a tracker or notable device.
// Signatures cross-checked against the BLE-Hound project's classifier.
const UUID_TILE_A: u16 = 0xFEED;
const UUID_TILE_B: u16 = 0xFEEC;
const UUID_SMARTTAG: u16 = 0xFD5A;
const UUID_META_GLASSES: u16 = 0xFD5F;
/// Google Fast Pair, also carried by Find My Device network tags. Not from
/// BLE-Hound; included because it is the fourth major tracking network.
const UUID_FAST_PAIR: u16 = 0xFE2C;

/// What a device appears to be, inferred from its advertising data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A separated Find My accessory: an AirTag or similar tag.
    FindMyTag,
    /// A device taking part in the Find My network — typically an iPhone,
    /// iPad or Mac, not a tag.
    FindMyDevice,
    /// Tile tracker.
    Tile,
    /// Samsung Galaxy SmartTag.
    SmartTag,
    /// Google Fast Pair / Find My Device network tag.
    FastPair,
    /// Meta Ray-Ban smart glasses.
    MetaGlasses,
    /// Flipper Zero.
    Flipper,
    /// Apple "Nearby" advert, emitted by phones, watches and buds.
    AppleNearby,
    IBeacon,
    AirDrop,
    Apple,
    /// Manufacturer data from some other vendor.
    Vendor(u16),
    /// Nothing identifying.
    Plain,
}

impl Kind {
    pub fn label(&self) -> &'static str {
        match self {
            Kind::FindMyTag => "AIRTAG / Find My tag",
            Kind::FindMyDevice => "Find My device",
            Kind::Tile => "TILE tracker",
            Kind::SmartTag => "GALAXY SmartTag",
            Kind::FastPair => "FastPair/FindMyDev",
            Kind::MetaGlasses => "Meta glasses",
            Kind::Flipper => "Flipper Zero",
            Kind::AppleNearby => "Apple nearby",
            Kind::IBeacon => "iBeacon",
            Kind::AirDrop => "AirDrop",
            Kind::Apple => "Apple",
            Kind::Vendor(_) => "vendor",
            Kind::Plain => "-",
        }
    }

    /// True for item trackers — the devices worth flagging if one is
    /// following you around.
    pub fn is_tracker(&self) -> bool {
        // Deliberately excludes FindMyDevice: a phone or laptop broadcasting
        // Find My is not an item tracker, and counting them would bury a real
        // tag in a house full of Apple hardware.
        matches!(
            self,
            Kind::FindMyTag | Kind::Tile | Kind::SmartTag | Kind::FastPair
        )
    }
}

/// One parsed advertising report.
#[derive(Debug, Clone, Copy)]
pub struct Advert {
    /// Address as received, least-significant byte first.
    pub addr: [u8; 6],
    /// True when the address is randomly generated. Trackers rotate their
    /// address periodically, so a repeat sighting may look like a new device.
    pub random_addr: bool,
    pub rssi: i8,
    pub kind: Kind,
    pub name: [u8; 20],
    pub name_len: u8,
}

impl Advert {
    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("")
    }
}

fn cmd(opcode: u16, params: &[u8], out: &mut [u8]) -> usize {
    out[0] = HCI_CMD;
    out[1] = (opcode & 0xff) as u8;
    out[2] = (opcode >> 8) as u8;
    out[3] = params.len() as u8;
    out[4..4 + params.len()].copy_from_slice(params);
    4 + params.len()
}

/// Put the controller into a passive scan.
///
/// Each command is given time to complete and its Command Complete event is
/// logged: a controller that rejects Set Scan Enable reports a non-zero
/// status here, which is otherwise silent and looks like "no beacons".
pub fn start_scan(ble: &mut BleConnector<'_>, delay: &mut Delay) {
    let mut buf = [0u8; 32];

    let n = cmd(OP_RESET, &[], &mut buf);
    let _ = ble.write(&buf[..n]);
    // Reset takes real time; commands sent too early are dropped.
    delay.delay_millis(200);
    report(ble, "reset");

    // Enable the LE Meta Event, which carries every advertising report.
    //
    // This is the step whose absence looks exactly like an empty room: the
    // controller scans happily and Set Scan Enable returns success, but the
    // default event mask after Reset only covers events 0..=44, and LE Meta
    // is bit 61. Reports are generated and then dropped before the host ever
    // sees them. The mask is 8 octets, least-significant first, so bit 61
    // lives in octet 7 as 0x20.
    let mask = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x3f];
    let n = cmd(OP_SET_EVENT_MASK, &mask, &mut buf);
    let _ = ble.write(&buf[..n]);
    delay.delay_millis(50);
    report(ble, "event mask");

    // And within LE, enable the advertising-report sub-event (bit 1).
    let le_mask = [0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let n = cmd(OP_LE_SET_EVENT_MASK, &le_mask, &mut buf);
    let _ = ble.write(&buf[..n]);
    delay.delay_millis(50);
    report(ble, "le event mask");

    // Passive scan, interval 0x0060 and window 0x0030 in 0.625ms units,
    // public own address, accept all advertisements.
    //
    // Multi-byte HCI parameters are little-endian, so 0x0060 goes out as
    // 0x60 0x00 -- sending it big-endian asks for a 60x longer interval.
    let params = [0x00, 0x60, 0x00, 0x30, 0x00, 0x00, 0x00];
    let n = cmd(OP_LE_SET_SCAN_PARAMS, &params, &mut buf);
    let _ = ble.write(&buf[..n]);
    delay.delay_millis(50);
    report(ble, "scan params");

    // Enable, without duplicate filtering: repeat sightings are how signal
    // strength gets refreshed.
    let n = cmd(OP_LE_SET_SCAN_ENABLE, &[0x01, 0x00], &mut buf);
    let _ = ble.write(&buf[..n]);
    delay.delay_millis(50);
    report(ble, "scan enable");
}

/// Drain pending HCI packets, logging any Command Complete status.
fn report(ble: &mut BleConnector<'_>, what: &str) {
    let mut buf = [0u8; 257];
    for _ in 0..8 {
        let n = ble.next(&mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        // [type][evt][plen][num_cmd][opcode_lo][opcode_hi][status]
        if n >= 7 && buf[0] == HCI_EVT && buf[1] == EVT_CMD_COMPLETE {
            let opcode = u16::from_le_bytes([buf[4], buf[5]]);
            let status = buf[6];
            esp_println::println!(
                "ble: {what} -> opcode {opcode:#06x} status {status:#04x}{}",
                if status == 0 { " (ok)" } else { " FAILED" }
            );
        } else {
            esp_println::println!("ble: {what} -> event {:02x?}", &buf[..n.min(8)]);
        }
    }
}

pub fn stop_scan(ble: &mut BleConnector<'_>) {
    let mut buf = [0u8; 32];
    let n = cmd(OP_LE_SET_SCAN_ENABLE, &[0x00, 0x00], &mut buf);
    let _ = ble.write(&buf[..n]);
    let mut sink = [0u8; 257];
    for _ in 0..8 {
        if ble.next(&mut sink).unwrap_or(0) == 0 {
            break;
        }
    }
}

/// Read one HCI packet and return an advert if it was an advertising report.
pub fn poll(ble: &mut BleConnector<'_>) -> Option<Advert> {
    poll_counted(ble, &mut 0)
}

/// As [`poll`], but counts every HCI packet seen — including ones that are
/// not advertising reports. A zero count means the scan never started; a
/// non-zero count with no adverts means the parsing is at fault.
pub fn poll_counted(ble: &mut BleConnector<'_>, packets: &mut u32) -> Option<Advert> {
    let mut buf = [0u8; 257];
    let n = ble.next(&mut buf).ok()?;
    if n == 0 {
        return None;
    }
    *packets = packets.saturating_add(1);
    if n < 4 || buf[0] != HCI_EVT || buf[1] != EVT_LE_META {
        return None;
    }
    // buf: [type][evt][plen][subevent][num_reports][event_type][addr_type][addr*6][len][data][rssi]
    if buf[3] != SUBEVT_ADV_REPORT || n < 12 {
        return None;
    }

    // Only the first report is parsed; this controller delivers one at a time.
    let addr_type = buf[6];
    let mut addr = [0u8; 6];
    addr.copy_from_slice(&buf[7..13]);
    let data_len = *buf.get(13)? as usize;
    let data_end = 14 + data_len;
    if data_end >= n {
        return None;
    }
    let data = &buf[14..data_end];
    let rssi = buf[data_end] as i8;

    let parsed = parse_ad(data);
    Some(Advert {
        addr,
        random_addr: addr_type == 0x01 || addr_type == 0x03,
        rssi,
        kind: classify(&addr, &parsed),
        name: parsed.name,
        name_len: parsed.name_len,
    })
}

/// Fields lifted out of an advertising payload.
struct Parsed {
    /// Company ID, then the first two payload bytes. For Apple those are the
    /// sub-type and its length.
    mfg: Option<(u16, Option<u8>, Option<u8>)>,
    /// 16-bit service UUIDs, from UUID lists and from service data.
    services: [u16; 8],
    service_count: u8,
    name: [u8; 20],
    name_len: u8,
}

impl Parsed {
    fn has_service(&self, uuid: u16) -> bool {
        self.services[..self.service_count as usize].contains(&uuid)
    }
}

/// Walk the length/type/value structures of an advertising payload.
fn parse_ad(data: &[u8]) -> Parsed {
    let mut p = Parsed {
        mfg: None,
        services: [0; 8],
        service_count: 0,
        name: [0; 20],
        name_len: 0,
    };

    let mut i = 0usize;
    while i < data.len() {
        let len = data[i] as usize;
        if len == 0 || i + len >= data.len() + 1 {
            break;
        }
        let end = (i + 1 + len).min(data.len());
        if i + 2 > end {
            break;
        }
        let ad_type = data[i + 1];
        let value = &data[i + 2..end];

        match ad_type {
            // Shortened or complete local name.
            0x08 | 0x09 => {
                let take = value.len().min(p.name.len());
                p.name[..take].copy_from_slice(&value[..take]);
                p.name_len = take as u8;
            }
            // 16-bit service UUID lists (incomplete, complete, solicitation).
            0x02 | 0x03 | 0x14 => {
                for pair in value.chunks_exact(2) {
                    push_service(&mut p, u16::from_le_bytes([pair[0], pair[1]]));
                }
            }
            // Service data, 16-bit UUID: the UUID leads the payload.
            0x16 if value.len() >= 2 => {
                push_service(&mut p, u16::from_le_bytes([value[0], value[1]]));
            }
            // Manufacturer specific data: first two bytes are the company ID.
            0xFF if value.len() >= 2 => {
                p.mfg = Some((
                    u16::from_le_bytes([value[0], value[1]]),
                    value.get(2).copied(),
                    value.get(3).copied(),
                ));
            }
            _ => {}
        }
        i += len + 1;
    }

    p
}

fn push_service(p: &mut Parsed, uuid: u16) {
    if (p.service_count as usize) < p.services.len() && !p.has_service(uuid) {
        p.services[p.service_count as usize] = uuid;
        p.service_count += 1;
    }
}

/// Identify a device from its advert. Service UUIDs are checked before the
/// manufacturer fallback, since trackers are identified by UUID.
fn classify(addr: &[u8; 6], p: &Parsed) -> Kind {
    if p.has_service(UUID_TILE_A) || p.has_service(UUID_TILE_B) {
        return Kind::Tile;
    }
    if p.has_service(UUID_SMARTTAG) {
        return Kind::SmartTag;
    }
    if p.has_service(UUID_FAST_PAIR) {
        return Kind::FastPair;
    }
    if p.has_service(UUID_META_GLASSES) {
        return Kind::MetaGlasses;
    }
    if is_flipper(addr, p) {
        return Kind::Flipper;
    }

    match p.mfg {
        Some((APPLE_COMPANY_ID, sub, len)) => match sub {
            Some(APPLE_TYPE_FIND_MY) => {
                if len == Some(APPLE_FIND_MY_TAG_LEN) {
                    Kind::FindMyTag
                } else {
                    Kind::FindMyDevice
                }
            }
            Some(APPLE_TYPE_NEARBY) => Kind::AppleNearby,
            Some(APPLE_TYPE_IBEACON) => Kind::IBeacon,
            Some(APPLE_TYPE_AIRDROP) => Kind::AirDrop,
            _ => Kind::Apple,
        },
        Some((company, _, _)) => Kind::Vendor(company),
        None => Kind::Plain,
    }
}

/// Flipper Zeros are recognised by OUI, by their service UUIDs, or by name.
fn is_flipper(addr: &[u8; 6], p: &Parsed) -> bool {
    // addr arrives least-significant byte first, so the OUI is the tail.
    let oui = [addr[5], addr[4], addr[3]];
    if matches!(oui, [0x80, 0xE1, 0x26] | [0x80, 0xE1, 0x27] | [0x0C, 0xFA, 0x22]) {
        return true;
    }
    if (0x3080..=0x3083).any(|u| p.has_service(u)) {
        return true;
    }
    let name = core::str::from_utf8(&p.name[..p.name_len as usize]).unwrap_or("");
    name.to_ascii_lowercase().contains("flipper")
}
