use crate::{BodyId, EditRecord, EditRequest};
use std::collections::BTreeMap;
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"EDJ2";
const RECORD_BYTES: usize = 42;

#[derive(Debug)]
pub struct JournalError(pub String);

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for JournalError {}
impl From<std::io::Error> for JournalError {
    fn from(value: std::io::Error) -> Self {
        Self(value.to_string())
    }
}

#[derive(Clone, Debug, Default)]
pub struct EditJournal {
    records: Vec<EditRecord>,
    columns: BTreeMap<(BodyId, u8, u64, u64), i64>,
}

impl EditJournal {
    pub fn records(&self) -> &[EditRecord] {
        &self.records
    }
    pub fn columns(&self) -> &BTreeMap<(BodyId, u8, u64, u64), i64> {
        &self.columns
    }
    pub fn next_sequence(&self) -> u64 {
        self.records
            .last()
            .map_or(1, |record| record.sequence.saturating_add(1))
    }

    /// Apply one already-sequenced server record to an in-memory client
    /// journal. This is the same strict replay path used when loading EDJ2.
    pub fn apply_record(&mut self, record: EditRecord) -> Result<(), JournalError> {
        record.edit.validate().map_err(JournalError)?;
        let expected = self.next_sequence();
        if record.sequence != expected {
            return Err(JournalError(format!(
                "journal sequence gap: expected {expected}, found {}",
                record.sequence
            )));
        }
        self.columns.insert(
            (
                record.edit.body,
                record.edit.face,
                record.edit.ci,
                record.edit.cj,
            ),
            record.edit.value,
        );
        self.records.push(record);
        Ok(())
    }
}

/// EDJ2 is a four-byte header followed only by fixed-size records. The file
/// length is the record count; accepting an edit performs one append+flush
/// and never rewrites existing bytes.
pub struct JournalStore {
    path: PathBuf,
    journal: EditJournal,
}

impl JournalStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, JournalError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)?;
            file.write_all(MAGIC)?;
            file.sync_data()?;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?.read_to_end(&mut bytes)?;
        if bytes.len() < MAGIC.len() || &bytes[..4] != MAGIC {
            return Err(JournalError(format!(
                "{} is not an EDJ2 journal",
                path.display()
            )));
        }
        // A crash mid-append can leave a torn final record. Whole records
        // are intact (append-only, fsynced), so recover by truncating the
        // incomplete tail — loudly — instead of refusing to start (R-4).
        // The file itself is truncated so the next append lands on a
        // record boundary rather than after garbage.
        let usable = MAGIC.len() + (bytes.len() - MAGIC.len()) / RECORD_BYTES * RECORD_BYTES;
        if usable != bytes.len() {
            eprintln!(
                "WARNING: {} ends with a torn journal record ({} trailing bytes); truncating to the last whole record (crash recovery)",
                path.display(),
                bytes.len() - usable
            );
            let file = std::fs::OpenOptions::new().write(true).open(&path)?;
            file.set_len(usable as u64)?;
            file.sync_data()?;
            bytes.truncate(usable);
        }
        let mut journal = EditJournal::default();
        for raw in bytes[4..].chunks_exact(RECORD_BYTES) {
            journal.apply_record(decode_record(raw)?)?;
        }
        Ok(Self { path, journal })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn journal(&self) -> &EditJournal {
        &self.journal
    }

    pub fn append(
        &mut self,
        accepted_at_mono_ms: u64,
        edit: EditRequest,
    ) -> Result<EditRecord, JournalError> {
        edit.validate().map_err(JournalError)?;
        let record = EditRecord {
            sequence: self.journal.next_sequence(),
            accepted_at_mono_ms,
            edit,
        };
        let encoded = encode_record(&record);
        let mut file = std::fs::OpenOptions::new().append(true).open(&self.path)?;
        file.write_all(&encoded)?;
        file.sync_data()?;
        self.journal.apply_record(record.clone())?;
        Ok(record)
    }

}

fn encode_record(record: &EditRecord) -> [u8; RECORD_BYTES] {
    let mut out = [0u8; RECORD_BYTES];
    out[0..8].copy_from_slice(&record.sequence.to_le_bytes());
    out[8..16].copy_from_slice(&record.accepted_at_mono_ms.to_le_bytes());
    out[16] = record.edit.body.wire_id();
    out[17] = record.edit.face;
    out[18..26].copy_from_slice(&record.edit.ci.to_le_bytes());
    out[26..34].copy_from_slice(&record.edit.cj.to_le_bytes());
    out[34..42].copy_from_slice(&record.edit.value.to_le_bytes());
    out
}

fn decode_record(raw: &[u8]) -> Result<EditRecord, JournalError> {
    let u64_at = |start: usize| u64::from_le_bytes(raw[start..start + 8].try_into().unwrap());
    let body = BodyId::from_wire_id(raw[16])
        .ok_or_else(|| JournalError(format!("invalid body byte {}", raw[16])))?;
    Ok(EditRecord {
        sequence: u64_at(0),
        accepted_at_mono_ms: u64_at(8),
        edit: EditRequest {
            body,
            face: raw[17],
            ci: u64_at(18),
            cj: u64_at(26),
            value: i64::from_le_bytes(raw[34..42].try_into().unwrap()),
        },
    })
}

pub fn load_legacy_edits(
    path: &Path,
    body: BodyId,
) -> Result<BTreeMap<(u8, u64, u64), i64>, JournalError> {
    let raw = std::fs::read(path)?;
    if raw.len() < 8 || &raw[0..4] != b"EDT1" {
        return Err(JournalError(format!(
            "{} is not an EDT1 edit map",
            path.display()
        )));
    }
    let count = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
    if raw.len() != 8 + count * 25 {
        return Err(JournalError(format!(
            "{} has an invalid EDT1 length",
            path.display()
        )));
    }
    let mut result = BTreeMap::new();
    for record in raw[8..].chunks_exact(25) {
        let edit = EditRequest {
            body,
            face: record[0],
            ci: u64::from_le_bytes(record[1..9].try_into().unwrap()),
            cj: u64::from_le_bytes(record[9..17].try_into().unwrap()),
            value: i64::from_le_bytes(record[17..25].try_into().unwrap()),
        };
        edit.validate().map_err(JournalError)?;
        if edit.value != 0 {
            result.insert((edit.face, edit.ci, edit.cj), edit.value);
        }
    }
    Ok(result)
}

pub fn write_legacy_edits(
    path: &Path,
    body: BodyId,
    columns: &BTreeMap<(BodyId, u8, u64, u64), i64>,
) -> Result<(), JournalError> {
    let selected = columns
        .iter()
        .filter(|((record_body, _, _, _), value)| *record_body == body && **value != 0)
        .collect::<Vec<_>>();
    let mut bytes = Vec::with_capacity(8 + selected.len() * 25);
    bytes.extend_from_slice(b"EDT1");
    bytes.extend_from_slice(&(selected.len() as u32).to_le_bytes());
    for ((_, face, ci, cj), value) in selected {
        bytes.push(*face);
        bytes.extend_from_slice(&ci.to_le_bytes());
        bytes.extend_from_slice(&cj.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    // Atomic replace: a crash mid-write must never leave a truncated
    // snapshot where a complete one used to be (std::fs::rename replaces
    // on Windows via MOVEFILE_REPLACE_EXISTING).
    let tmp = path.with_extension("edt1.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "triangulum-{name}-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn journal_persistence_round_trip_preserves_order_and_lww_state() {
        let path = unique_path("journal");
        {
            let mut store = JournalStore::open(&path).unwrap();
            store
                .append(
                    10,
                    EditRequest {
                        body: BodyId::Neisor,
                        face: 1,
                        ci: 2,
                        cj: 3,
                        value: 1,
                    },
                )
                .unwrap();
            store
                .append(
                    20,
                    EditRequest {
                        body: BodyId::Neisor,
                        face: 1,
                        ci: 2,
                        cj: 3,
                        value: -2,
                    },
                )
                .unwrap();
            store
                .append(
                    30,
                    EditRequest {
                        body: BodyId::Moon,
                        face: 4,
                        ci: 5,
                        cj: 6,
                        value: 8,
                    },
                )
                .unwrap();
        }
        let reopened = JournalStore::open(&path).unwrap();
        assert_eq!(reopened.journal().records().len(), 3);
        assert_eq!(reopened.journal().records()[2].sequence, 3);
        assert_eq!(reopened.journal().columns()[&(BodyId::Neisor, 1, 2, 3)], -2);
        assert_eq!(reopened.journal().columns()[&(BodyId::Moon, 4, 5, 6)], 8);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn torn_final_record_is_truncated_and_recovered() {
        let path = unique_path("torn");
        {
            let mut store = JournalStore::open(&path).unwrap();
            for value in 1..=2 {
                store
                    .append(
                        value as u64 * 10,
                        EditRequest {
                            body: BodyId::Neisor,
                            face: 0,
                            ci: 1,
                            cj: value as u64,
                            value,
                        },
                    )
                    .unwrap();
            }
        }
        // Simulate a crash mid-append: garbage partial record at the tail.
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(&[0xAB; 17]).unwrap();
        }
        let reopened = JournalStore::open(&path).unwrap();
        assert_eq!(reopened.journal().records().len(), 2);
        assert_eq!(reopened.journal().next_sequence(), 3);
        // The file itself was truncated back to a record boundary.
        let len = std::fs::metadata(&path).unwrap().len() as usize;
        assert_eq!(len, 4 + 2 * RECORD_BYTES);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn legacy_snapshot_round_trip() {
        let path = unique_path("legacy");
        let columns = BTreeMap::from([
            ((BodyId::Neisor, 0, 7, 8), 2),
            ((BodyId::Moon, 1, 9, 10), -3),
        ]);
        write_legacy_edits(&path, BodyId::Neisor, &columns).unwrap();
        let loaded = load_legacy_edits(&path, BodyId::Neisor).unwrap();
        assert_eq!(loaded, BTreeMap::from([((0, 7, 8), 2)]));
        let _ = std::fs::remove_file(path);
    }
}
