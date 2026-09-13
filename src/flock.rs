//! Passive detector for Flock Safety ALPR camera infrastructure.
//!
//! Flock's cameras run a 2.4GHz radio whose transmitter MAC falls in a known
//! set of OUIs. Watching 802.11 frames in promiscuous mode and matching the
//! transmitter address (addr2) against that set flags the hardware nearby,
//! without ever transmitting.
//!
//! The OUI list and the method come from the flock-you project
//! (github.com/colonelpanichacks/flock-you) and @NitekryDPaul's research.
//! This implements its core tier -- transmitter-side OUI match -- not the
//! probe-request IE fingerprinting.
//!
//! Passive only: promiscuous receive plus channel hopping. Nothing is sent.
//!
//! Region note: Flock hardware is deployed mainly in the US and Canada. The
//! OUI list will not match anything elsewhere.

use core::cell::RefCell;

use critical_section::Mutex;

/// Flock camera transmitter OUIs, lowercase, as raw bytes.
/// Synced with flock-you's target list (31 prefixes + DeFlockJoplin's).
pub const FLOCK_OUIS: [[u8; 3]; 32] = [
    [0x70, 0xc9, 0x4e], // 70:c9:4e
    [0x3c, 0x91, 0x80], // 3c:91:80
    [0xd8, 0xf3, 0xbc], // d8:f3:bc
    [0x80, 0x30, 0x49], // 80:30:49
    [0xb8, 0x35, 0x32], // b8:35:32
    [0x14, 0x5a, 0xfc], // 14:5a:fc
    [0x74, 0x4c, 0xa1], // 74:4c:a1
    [0x08, 0x3a, 0x88], // 08:3a:88
    [0x9c, 0x2f, 0x9d], // 9c:2f:9d
    [0xc0, 0x35, 0x32], // c0:35:32
    [0x94, 0x08, 0x53], // 94:08:53
    [0xe4, 0xaa, 0xea], // e4:aa:ea
    [0xf4, 0x6a, 0xdd], // f4:6a:dd
    [0xe0, 0x0a, 0xf6], // e0:0a:f6
    [0x24, 0xb2, 0xb9], // 24:b2:b9
    [0x00, 0xf4, 0x8d], // 00:f4:8d
    [0xd0, 0x39, 0x57], // d0:39:57
    [0xe8, 0xd0, 0xfc], // e8:d0:fc
    [0xe0, 0x4f, 0x43], // e0:4f:43
    [0xb8, 0x1e, 0xa4], // b8:1e:a4
    [0x70, 0x08, 0x94], // 70:08:94
    [0x58, 0x8e, 0x81], // 58:8e:81
    [0xec, 0x1b, 0xbd], // ec:1b:bd
    [0x3c, 0x71, 0xbf], // 3c:71:bf
    [0x58, 0x00, 0xe3], // 58:00:e3
    [0x90, 0x35, 0xea], // 90:35:ea
    [0x5c, 0x93, 0xa2], // 5c:93:a2
    [0x64, 0x6e, 0x69], // 64:6e:69
    [0x48, 0x27, 0xea], // 48:27:ea
    [0xa4, 0xcf, 0x12], // a4:cf:12
    [0x14, 0xb5, 0xcd], // 14:b5:cd
    [0x82, 0x6b, 0xf2], // 82:6b:f2
];

/// The channels Flock cameras are observed hopping, highest first: the order
/// flock-you found detects fastest.
pub const HOP_CHANNELS: [u8; 3] = [11, 6, 1];

/// A recorded detection.
#[derive(Clone, Copy)]
pub struct Hit {
    pub mac: [u8; 6],
    /// Index into [`FLOCK_OUIS`], for showing which prefix matched.
    pub oui_index: u8,
    pub channel: u8,
    pub rssi: i8,
    pub count: u32,
    /// True once drawn, so the app only redraws changed rows.
    pub dirty: bool,
}

pub const MAX_HITS: usize = 12;

/// Shared between the promiscuous callback (writer) and the app (reader).
pub struct FlockState {
    pub hits: [Option<Hit>; MAX_HITS],
    /// Total non-control frames examined — proves the sniffer is alive even
    /// when nothing matches.
    pub frames: u32,
    /// Bumped on every new or updated hit, so the app knows to redraw.
    pub revision: u32,
}

impl FlockState {
    const fn new() -> Self {
        Self {
            hits: [None; MAX_HITS],
            frames: 0,
            revision: 0,
        }
    }
}

/// Global because the promiscuous callback is a bare `fn` with no state
/// parameter; it can only reach the detection table through a static.
pub static FLOCK: Mutex<RefCell<FlockState>> = Mutex::new(RefCell::new(FlockState::new()));

/// Return the matching OUI index, if the three bytes are a Flock prefix.
fn match_oui(oui: &[u8]) -> Option<u8> {
    FLOCK_OUIS
        .iter()
        .position(|p| p == oui)
        .map(|i| i as u8)
}

/// Promiscuous receive callback. Runs in Wi-Fi task context, so it does the
/// minimum: parse the transmitter address, match, and record. No drawing, no
/// allocation.
pub fn on_frame(pkt: esp_radio::wifi::sniffer::PromiscuousPkt<'_>) {
    let d = pkt.data;
    // Need through addr2 (bytes 10..16) to read the transmitter.
    if d.len() < 16 {
        return;
    }
    // Frame type lives in bits 2-3 of the first control byte. Control frames
    // (type 1) do not carry addr2 at this offset, so skip them.
    let ftype = (d[0] >> 2) & 0x3;
    if ftype == 1 {
        return;
    }

    let addr2 = &d[10..16];
    let matched = match_oui(&addr2[..3]);

    let channel = pkt.rx_cntl.channel as u8;
    let rssi = pkt.rx_cntl.rssi as i8;

    critical_section::with(|cs| {
        let mut state = FLOCK.borrow_ref_mut(cs);
        state.frames = state.frames.wrapping_add(1);

        let Some(oui_index) = matched else { return };

        // Update an existing entry, or claim a free slot.
        for slot in state.hits.iter_mut() {
            if let Some(h) = slot {
                if h.mac == addr2 {
                    h.channel = channel;
                    h.rssi = rssi;
                    h.count = h.count.saturating_add(1);
                    h.dirty = true;
                    state.revision = state.revision.wrapping_add(1);
                    return;
                }
            }
        }
        for slot in state.hits.iter_mut() {
            if slot.is_none() {
                let mut mac = [0u8; 6];
                mac.copy_from_slice(addr2);
                *slot = Some(Hit {
                    mac,
                    oui_index,
                    channel,
                    rssi,
                    count: 1,
                    dirty: true,
                });
                state.revision = state.revision.wrapping_add(1);
                return;
            }
        }
        // Table full: a new camera is dropped rather than evicting a
        // confirmed one. MAX_HITS is generous for any real location.
    });
}

/// Clear the table, for a fresh run.
pub fn reset() {
    critical_section::with(|cs| {
        *FLOCK.borrow_ref_mut(cs) = FlockState::new();
    });
}
