// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Nayeem Bin Ahsan
//! `Sec-MS-GEC` DRM token generation and Edge-protocol time helpers.
//!
//! The Edge Read-Aloud endpoint requires a `Sec-MS-GEC` query parameter whose
//! value is a SHA-256 hash derived from the current Windows file-time and a
//! fixed client token.  The token is valid inside a rolling 5-minute window;
//! when the server's clock and ours disagree by more than that, the handshake
//! returns HTTP 403 and the connection layer retries with an adjusted skew.

use rand::RngCore;
use std::time::{SystemTime, UNIX_EPOCH};

/// Long-stable trusted-client token embedded in the Edge extension.
/// (Source: `edge-tts` / `rany2` - unchanged for years.)
pub(crate) const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";

/// Edge stable-channel version string.  Microsoft rotates this on release
/// cycles; when every synthesis starts returning HTTP 403, re-pin this to the
/// current Edge version and republish the crate.
pub(crate) const SEC_MS_GEC_VERSION: &str = "2-143.0.3650.75";

/// Seconds between 1601-01-01 (Windows epoch) and 1970-01-01 (Unix epoch).
const WIN_EPOCH: u64 = 11_644_473_600;

/// The token is quantised to this window (5 minutes).  Values outside the
/// current window are rejected by the server.
const TICK_WINDOW_SECS: u64 = 300;

/// 100-nanosecond intervals per second (Windows file-time granularity).
const INTERVALS_PER_SEC: u64 = 10_000_000;

const SECS_PER_MINUTE: u64 = 60;
const SECS_PER_HOUR: u64 = 3_600;
const SECS_PER_DAY: u64 = 86_400;
const HOURS_PER_DAY: u64 = 24;
const DAYS_PER_WEEK: usize = 7;
const DAYS_PER_400_YEARS: i64 = 146_097;

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Days from 0000-03-01 to 1970-01-01 in Howard Hinnant's civil calendar
/// algorithm. See https://howardhinnant.github.io/date_algorithms.html
const DAYS_UNIX_EPOCH_OFFSET: i64 = 719_468;

/// Days in a 4-year Gregorian cycle.
const DAYS_PER_4_YEARS: u64 = 1_460;

/// Days in a 100-year Gregorian cycle.
const DAYS_PER_100_YEARS: u64 = 36_524;

/// Days per year (non-leap).
const DAYS_PER_YEAR: u64 = 365;

/// Numerator coefficient for the month-of-year calculation in Hinnant's algorithm.
const MONTH_NUMERATOR_COEFF: u64 = 153;

/// Small numerator in the month-of-year division.
const MONTH_SCALE: u64 = 5;

/// Constant offset in both month-of-year divisions.
const MONTH_DIVISOR: u64 = 2;

/// Broken-down UTC date-time used to build the `X-Timestamp` header.
pub struct CivilDateTime {
    pub weekday: usize,
    pub year: u64,
    pub month: usize,
    pub day: u64,
    pub hour: u64,
    pub minute: u64,
    pub second: u64,
}

/// Generate a random uppercase-hex string of `nbytes * 2` hex digits.
///
/// Used for the `MUID` cookie, `ConnectionId`, and `X-RequestId` values.
pub fn random_hex(nbytes: usize) -> String {
    let mut buf = vec![0u8; nbytes];
    rand::thread_rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02X}")).collect()
}

/// Current Unix timestamp adjusted by `skew` seconds (0 when clocks agree).
fn unix_secs(skew: i64) -> u64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    (secs + skew).max(0) as u64
}

/// Compute the `Sec-MS-GEC` v2 token for the given clock skew.
///
/// The current Edge service expects the v2 token format
/// `<16 hex>-<64 hex>` derived from the 100-ns Windows file time and the
/// trusted client token.  Ported from `edge-tts` `DRM.generate_sec_ms_gec_v2`.
pub fn sec_ms_gec(skew: i64) -> String {
    let unix = unix_secs(skew) as f64;
    sec_ms_gec_at(unix)
}

/// Compute a v2 token for an explicit Unix timestamp (seconds).
pub fn sec_ms_gec_at(unix_timestamp: f64) -> String {
    let mut ticks = unix_timestamp + WIN_EPOCH as f64;
    ticks -= ticks % TICK_WINDOW_SECS as f64;
    ticks *= INTERVALS_PER_SEC as f64;
    let seed = format!("{ticks:.0}{TRUSTED_CLIENT_TOKEN}");
    let digest = sha256_digest(seed.as_bytes());
    let (low, high) = cs64_digest(seed.as_bytes(), &digest);
    let mut cs64 = Vec::with_capacity(8);
    cs64.extend_from_slice(&low.to_le_bytes());
    cs64.extend_from_slice(&high.to_le_bytes());
    format!("{}-{}", hex_upper(&cs64), hex_upper(&digest))
}

fn hex_upper(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

const U32_MASK: u32 = 0xFFFF_FFFF;
const SHA256_INITIAL: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB,
    0x5BE0CD19,
];
const SHA256_STATE: [u32; 8] = [
    0xFD9FBEA2, 0x91243CE7, 0xBE499094, 0xCD2E1E0A, 0x6C006F49, 0x04BD35AB, 0xFEB0A7E4,
    0x03EFFBD2,
];
const SHA256_SCHEDULE: [u32; 14] = [
    0xEB24BF9E, 0x1558691F, 0xA0C09EE2, 0x0CFA32BA, 0xA622EE8C, 0xD1DD9491, 0xC0DD25A0,
    0xF9E70AB6, 0xF0C31573, 0x7058CE4E, 0x7188E606, 0x00E5AB33, 0xD258B060, 0x72766564,
];
const SHA256_ROUND_CONSTANTS: [u32; 64] = [
    0x428A2F98, 0x71374491, 0xB5C0FBCF, 0xE9B5DBA5, 0x3956C25B, 0x59F111F1, 0x923F82A4,
    0xAB1C5ED5, 0xD807AA98, 0x12835B01, 0x243185BE, 0x550C7DC3, 0x72BE5D74, 0x80DEB1FE,
    0x9BDC06A7, 0xC19BF174, 0xE49B69C1, 0xEFBE4786, 0x0FC19DC6, 0x240CA1CC, 0x2DE92C6F,
    0x4A7484AA, 0x5CB0A9DC, 0x76F988DA, 0x983E5152, 0xA831C66D, 0xB00327C8, 0xBF597FC7,
    0xC6E00BF3, 0xD5A79147, 0x06CA6351, 0x14292967, 0x27B70A85, 0x2E1B2138, 0x4D2C6DFC,
    0x53380D13, 0x650A7354, 0x766A0ABB, 0x81C2C92E, 0x92722C85, 0xA2BFE8A1, 0xA81A664B,
    0xC24B8B70, 0xC76C51A3, 0xD192E819, 0xD6990624, 0xF40E3585, 0x106AA070, 0x19A4C116,
    0x1E376C08, 0x2748774C, 0x34B0BCB5, 0x391C0CB3, 0x4ED8AA4A, 0x5B9CCA4F, 0x682E6FF3,
    0x748F82EE, 0x78A5636F, 0x84C87814, 0x8CC70208, 0x90BEFFFA, 0xA4506CEB, 0xBEF9A3F7,
    0xC67178F2,
];
const SHA256_ADJUSTMENT: u32 = 0xC9DC06A7;
const SHA256_SIGMA0_ADJUSTMENT: u32 = 0x059C0B80;
const SHA256_SIGMA1_ADJUSTMENT: u32 = 0x000B92C0;
const CS64_COEFFICIENTS: [(u32, u32); 14] = [
    (0xE82E91B1, 0xB32700D0),
    (0x44DE24C6, 0x88DFF51D),
    (0x3BEB65BC, 0x29514038),
    (0xC89C5933, 0xB328CBDF),
    (0x8D1EC211, 0xD405B4D5),
    (0x314EF5D5, 0x26E55F3C),
    (0xE92491B1, 0x607801C6),
    (0xF0A5B9D5, 0x82B2C1FB),
    (0x06064CF3, 0xFA51A3DA),
    (0xDA79FA60, 0x207CC464),
    (0xBED20560, 0x3C3C74F3),
    (0x89F9C9F8, 0xD2D14B61),
    (0x35FA38C6, 0xBB04A2C1),
    (0xCB49E3C6, 0xC5626167),
];
const CS64_FACTOR: u32 = 0x82893933;
const CS64_ADJUSTMENT0: u32 = 0xF2F90B1A;
const CS64_ADJUSTMENT1: u32 = 0x0B1A0000;

fn rotl16(value: u32) -> u32 {
    value.rotate_left(16)
}

fn sigma0(value: u32) -> u32 {
    value.rotate_right(7) ^ value.rotate_right(18) ^ (value >> 3)
}

fn sigma1(value: u32) -> u32 {
    value.rotate_right(17) ^ value.rotate_right(19) ^ (value >> 10)
}

fn expand_schedule(words: &mut Vec<u32>, first: usize) {
    for index in first..64 {
        let x = words[index - 2];
        let y = words[index - 15];
        words.push(
            sigma1(x)
                .wrapping_add(words[index - 7])
                .wrapping_add(sigma0(y))
                .wrapping_add(words[index - 16]),
        );
    }
}

fn sha256_init(data: &[u8]) -> [u32; 8] {
    let mut words: Vec<u32> = vec![0; 14];
    words.push(u32::from_be_bytes([0, data[0], data[1], data[2]]));
    words.push(u32::from_be_bytes([data[3], data[4], data[5], data[6]]));
    for index in 16..30 {
        let mut value = SHA256_SCHEDULE[index - 16];
        if index - 2 >= 14 {
            let part = sigma1(words[index - 2]);
            value = value.wrapping_add(if index == 16 {
                part ^ SHA256_SIGMA1_ADJUSTMENT
            } else {
                part
            });
        }
        if index - 7 >= 14 {
            value = value.wrapping_add(words[index - 7]);
        }
        if index - 15 >= 14 {
            let part = sigma0(words[index - 15]);
            value = value.wrapping_add(if index == 29 {
                part ^ SHA256_SIGMA0_ADJUSTMENT
            } else {
                part
            });
        }
        words.push(value);
    }
    let tail = sigma1(words[28])
        .wrapping_add(words[23])
        .wrapping_add(sigma0(words[15]))
        .wrapping_add(words[14])
        .wrapping_add(SHA256_ADJUSTMENT)
        .wrapping_sub(SHA256_ROUND_CONSTANTS[14]);
    words.push(tail);
    expand_schedule(&mut words, 31);
    let working = sha256_rounds(&SHA256_STATE, &words, 14);
    let mut out = [0u32; 8];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = SHA256_INITIAL[index].wrapping_add(working[index]);
    }
    out
}

fn sha256_rounds(state: &[u32; 8], words: &[u32], first_round: usize) -> [u32; 8] {
    let mut registers = *state;
    for index in first_round..64 {
        let [a, b, c, d, e, f, g, h] = registers;
        let constant = if index == 14 && first_round == 14 {
            SHA256_ADJUSTMENT
        } else {
            SHA256_ROUND_CONSTANTS[index]
        };
        let t1 = h
            .wrapping_add(e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25))
            .wrapping_add(g ^ (e & (f ^ g)))
            .wrapping_add(constant)
            .wrapping_add(words[index]);
        let t2 = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22))
            .wrapping_add((a & b) | (c & (a | b)));
        registers = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
    }
    registers
}

fn sha256_digest(data: &[u8]) -> [u8; 32] {
    assert!(data.len() >= 7);
    let mask = U32_MASK;
    let length = 57 + data.len();
    let mut padded = data[7..].to_vec();
    padded.push(0x80);
    let zeros = (55i64 - length as i64).rem_euclid(64) as usize;
    padded.extend(std::iter::repeat(0u8).take(zeros));
    padded.extend_from_slice(&((length as u64) * 8).to_be_bytes());
    let mut state = sha256_init(&data[..7]);
    for chunk in padded.chunks(64) {
        let mut words = Vec::with_capacity(64);
        for part in chunk.chunks(4) {
            words.push(u32::from_be_bytes([part[0], part[1], part[2], part[3]]));
        }
        expand_schedule(&mut words, 16);
        let working = sha256_rounds(&state, &words, 0);
        for index in 0..8 {
            state[index] = state[index].wrapping_add(working[index]) & mask;
        }
    }
    let mut out = [0u8; 32];
    for (index, word) in state.iter().enumerate() {
        out[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn words_le4_even(data: &[u8]) -> Vec<u32> {
    let mut padded = data.to_vec();
    while padded.len() % 4 != 0 {
        padded.push(0);
    }
    let mut words: Vec<u32> = padded
        .chunks(4)
        .map(|part| u32::from_le_bytes([part[0], part[1], part[2], part[3]]))
        .collect();
    if words.len() % 2 != 0 {
        words.push(0);
    }
    words
}

fn cs64_digest(data: &[u8], digest: &[u8; 32]) -> (u32, u32) {
    let mask = U32_MASK;
    let key_a = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]) | 1;
    let key_b = u32::from_le_bytes([digest[4], digest[5], digest[6], digest[7]]) | 1;
    let factors = [
        key_a.wrapping_mul(CS64_FACTOR) & mask,
        key_b.wrapping_mul(CS64_FACTOR) & mask,
    ];
    let mut terms: Vec<u32> = CS64_COEFFICIENTS
        .iter()
        .enumerate()
        .map(|(index, (coefficient, _))| coefficient.wrapping_mul(factors[index % 2]) & mask)
        .collect();
    let mut offsets: Vec<u32> = CS64_COEFFICIENTS.iter().map(|(_, offset)| *offset).collect();
    let mut seeded = Vec::with_capacity(data.len() + 1);
    seeded.push(0u8);
    seeded.extend_from_slice(data);
    for (index, word) in words_le4_even(&seeded).iter().enumerate() {
        let key = if index % 2 == 0 { key_a } else { key_b };
        let mut term = word.wrapping_mul(key);
        let mut offset = rotl16(*word)
            .wrapping_mul(if index % 2 == 0 { 0xEF0569FB } else { 0xC31713DB });
        if index == 0 {
            term = term.wrapping_add(CS64_ADJUSTMENT0.wrapping_mul(factors[0])) & mask;
            offset = offset.wrapping_add(CS64_ADJUSTMENT1) & mask;
        }
        terms.push(term & mask);
        offsets.push(offset & mask);
    }
    let mut swap_acc = 0u32;
    let mut swap_last = 0u32;
    let mut state = 0u32;
    let mut acc = 0u32;
    for index in (0..terms.len()).step_by(2) {
        let x = terms[index];
        let y = terms[index + 1];
        let t = rotl16(offsets[index].wrapping_add(x) & mask);
        let q = t.wrapping_mul(0x76F50F34) & mask;
        swap_acc = swap_acc.wrapping_add(
            rotl16(q).wrapping_mul(0xC3EFEA97).wrapping_add(q) & mask,
        );
        let t = rotl16(offsets[index + 1].wrapping_add(y) & mask);
        let q = t.wrapping_mul(0x1881CE3C) & mask;
        swap_last = rotl16(q).wrapping_mul(0x35BD1EC9).wrapping_add(q) & mask;
        swap_acc = swap_acc.wrapping_add(swap_last) & mask;

        state = state.wrapping_mul(key_a).wrapping_add(x) & mask;
        state = rotl16(state).wrapping_mul(0xCF98B111) & mask;
        state = rotl16(state).wrapping_mul(0x87085B9F) & mask;
        state = rotl16(state).wrapping_mul(0x12CEB96D) & mask;
        state = rotl16(state).wrapping_mul(0x257E1D83) & mask;
        acc = acc.wrapping_add(state);
        let mut t = state.wrapping_mul(key_b).wrapping_add(y) & mask;
        t = rotl16(t).wrapping_mul(0xA27416F5) & mask;
        t = rotl16(t).wrapping_mul(0xD38396FF) & mask;
        state = rotl16(rotl16(t).wrapping_mul(0x7C932B89) & mask)
            .wrapping_mul(0xBFA49F69)
            & mask;
        acc = acc.wrapping_add(state) & mask;
    }
    (swap_last ^ state, swap_acc ^ acc)
}

/// Convert a Unix timestamp (UTC) into a broken-down date-time.
///
/// Uses Howard Hinnant's `days_from_civil` inverse algorithm - no external
/// date crate required.
pub fn civil_utc(secs: u64) -> CivilDateTime {
    let second = secs % SECS_PER_MINUTE;
    let minute = (secs / SECS_PER_MINUTE) % SECS_PER_MINUTE;
    let hour = (secs / SECS_PER_HOUR) % HOURS_PER_DAY;
    let days = (secs / SECS_PER_DAY) as i64;

    // 1970-01-01 was a Thursday (index 4).
    let weekday = ((days + 4).rem_euclid(DAYS_PER_WEEK as i64)) as usize;

    let z = days + DAYS_UNIX_EPOCH_OFFSET;
    let era = if z >= 0 {
        z / DAYS_PER_400_YEARS
    } else {
        (z - (DAYS_PER_400_YEARS - 1)) / DAYS_PER_400_YEARS
    };
    let doe = (z - era * DAYS_PER_400_YEARS) as u64;
    let yoe = (doe - doe / DAYS_PER_4_YEARS + doe / DAYS_PER_100_YEARS
        - doe / DAYS_PER_400_YEARS as u64)
        / DAYS_PER_YEAR;
    let y = yoe as i64 + era * 400;
    let doy = doe - (DAYS_PER_YEAR * yoe + yoe / 4 - yoe / 100);
    let mp = (MONTH_SCALE * doy + MONTH_DIVISOR) / MONTH_NUMERATOR_COEFF;
    let day = doy - (MONTH_NUMERATOR_COEFF * mp + MONTH_DIVISOR) / MONTH_SCALE + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as usize;
    let year = (y + if month <= 2 { 1 } else { 0 }) as u64;

    CivilDateTime {
        weekday,
        year,
        month,
        day,
        hour,
        minute,
        second,
    }
}

/// Format a Unix timestamp as a JavaScript-style date string (Edge
/// `X-Timestamp` header format).  Pure - no clock access.
pub fn date_string(secs: u64) -> String {
    let dt = civil_utc(secs);
    format!(
        "{} {} {:02} {} {:02}:{:02}:{:02} GMT+0000 (Coordinated Universal Time)",
        WEEKDAYS.get(dt.weekday).copied().unwrap_or("Sun"),
        MONTHS
            .get(dt.month.wrapping_sub(1))
            .copied()
            .unwrap_or("Jan"),
        dt.day,
        dt.year,
        dt.hour,
        dt.minute,
        dt.second,
    )
}

/// Current time formatted for the `X-Timestamp` header.
pub(crate) fn current_date_string() -> String {
    date_string(unix_secs(0))
}


#[cfg(test)]
mod token_v2_tests {
    use super::*;

    #[test]
    fn matches_edge_tts_reference_tokens() {
        assert_eq!(
            sec_ms_gec_at(1_700_000_000.0),
            "2CD23A2CA0B2595A-F3C846C370E05F9CC83ACC5F995C5F5D87A4D0F14CE8936A0F902E64099E38A2"
        );
        assert_eq!(
            sec_ms_gec_at(1_794_018_579.0),
            "4D31DCFA29C9AA65-794B6417CFCF59B156B0D567FA3DE790C76AABE8EFFC044E38991D5E546C744B"
        );
    }
}

