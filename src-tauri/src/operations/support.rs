//! Sources of identifiers and time, and the revision a plan is checked against.
//!
//! An operation that creates a layer has to invent an identifier and stamp a
//! time. Both are injected rather than read from the environment inside the
//! operation, so that the same transaction run against the same document with the
//! same sources gives the same result — which is what makes a replay testable and
//! a recorded macro explainable.
use crate::error::AppError;
use crate::layers::LayerDocument;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

/// Where new layer identifiers come from.
pub trait IdSource: Send {
    /// A new identifier of the form `prefix` + characters from `[a-z0-9]`, no
    /// longer than the document limit.
    fn next(&mut self, prefix: &str) -> String;
}

/// Unpredictable identifiers, as the interface makes: twelve characters, so a
/// duplicate or a restored project cannot collide with one in practice.
///
/// Identifiers are not secrets. `RandomState` is seeded per process by the
/// operating system, which is plenty to keep two layers apart.
pub struct RandomIds {
    state: RandomState,
    counter: u64,
}

impl Default for RandomIds {
    fn default() -> Self {
        Self {
            state: RandomState::new(),
            counter: 0,
        }
    }
}

impl IdSource for RandomIds {
    fn next(&mut self, prefix: &str) -> String {
        self.counter += 1;
        let mut hasher = self.state.build_hasher();
        hasher.write_u64(self.counter);
        hasher.write_u128(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos()),
        );
        let mut bits = hasher.finish();
        let mut id = String::from(prefix);
        for _ in 0..12 {
            id.push(ALPHABET[(bits % ALPHABET.len() as u64) as usize] as char);
            bits = bits.rotate_right(7) ^ bits.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
        id
    }
}

/// Predictable identifiers, `prefix` then a counter. For tests and for replays
/// that must be reproduced exactly.
#[derive(Default)]
pub struct SequenceIds {
    next: u64,
}

impl IdSource for SequenceIds {
    fn next(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}seq{:04}", self.next)
    }
}

/// Where the time stamped on a layer comes from.
pub trait Clock: Send + Sync {
    /// An ISO 8601 UTC timestamp with millisecond precision.
    fn now(&self) -> String;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> String {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64);
        iso8601(millis)
    }
}

/// A clock that always says the same thing.
pub struct FixedClock(pub String);

impl Clock for FixedClock {
    fn now(&self) -> String {
        self.0.clone()
    }
}

/// Formats Unix milliseconds as `2026-10-06T00:12:34.567Z`.
///
/// Civil-from-days, after Howard Hinnant's algorithm: exact for every date the
/// proleptic Gregorian calendar has, with no table of month lengths to get wrong.
pub fn iso8601(unix_millis: i64) -> String {
    let seconds = unix_millis.div_euclid(1000);
    let millis = unix_millis.rem_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let in_day = seconds.rem_euclid(86_400);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        in_day / 3600,
        (in_day % 3600) / 60,
        in_day % 60
    )
}

/// Writes `value` with every object's keys in sorted order, so two values that
/// mean the same thing produce the same bytes whatever order a map iterated in.
fn canonical(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push(b'{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(Value::String(key.clone()).to_string().as_bytes());
                out.push(b':');
                canonical(&map[key], out);
            }
            out.push(b'}');
        }
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                canonical(item, out);
            }
            out.push(b']');
        }
        other => out.extend_from_slice(other.to_string().as_bytes()),
    }
}

/// The revision of a document: a SHA-256 of its canonical form.
///
/// A plan that names the revision it was made against is refused if the document
/// has since changed, so an automation prepared for one document cannot quietly
/// run against another. The tree carries no pixels, so this is cheap.
pub fn document_revision(document: &LayerDocument) -> Result<String, AppError> {
    let value = serde_json::to_value(document).map_err(|error| {
        AppError::ProcessingFailure(format!("could not hash the document: {error}"))
    })?;
    let mut bytes = Vec::with_capacity(4096);
    canonical(&value, &mut bytes);
    let digest = Sha256::digest(&bytes);
    Ok(format!("{digest:x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_matches_known_instants() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso8601(1_791_245_554_123), "2026-10-06T00:12:34.123Z");
        // Before the epoch, and across a century that is not a leap year.
        assert_eq!(iso8601(-1), "1969-12-31T23:59:59.999Z");
        assert_eq!(iso8601(4_107_542_400_000), "2100-03-01T00:00:00.000Z");
    }

    #[test]
    fn random_identifiers_are_well_formed_and_distinct() {
        let mut ids = RandomIds::default();
        let a = ids.next("l");
        let b = ids.next("l");
        assert_ne!(a, b);
        assert!(a.starts_with('l') && a.len() == 13);
        assert!(a.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    }

    #[test]
    fn sequence_identifiers_are_reproducible() {
        let mut first = SequenceIds::default();
        let mut second = SequenceIds::default();
        assert_eq!(first.next("g"), second.next("g"));
        assert_eq!(first.next("g"), "gseq0002");
    }

    #[test]
    fn the_revision_does_not_depend_on_map_order() {
        let mut a = LayerDocument::new(8, 8);
        let mut b = LayerDocument::new(8, 8);
        for key in ["x", "y", "z", "w"] {
            a.layers.push(crate::operations::structure::new_layer(
                &format!("id{key}"),
                "n",
                crate::layers::LayerContent::Group {
                    children: vec![],
                    isolated: true,
                },
                "t",
            ));
        }
        for key in ["x", "y", "z", "w"] {
            b.layers.push(crate::operations::structure::new_layer(
                &format!("id{key}"),
                "n",
                crate::layers::LayerContent::Group {
                    children: vec![],
                    isolated: true,
                },
                "t",
            ));
        }
        assert_eq!(
            document_revision(&a).unwrap(),
            document_revision(&b).unwrap()
        );
        b.layers[0].name = "different".into();
        assert_ne!(
            document_revision(&a).unwrap(),
            document_revision(&b).unwrap()
        );
    }
}
