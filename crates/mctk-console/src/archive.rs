//! The save archive.
//!
//! Layout (all integers big-endian):
//! - header: `index_offset: u32`, `file_count: u32`, `original_version: u16`, `version: u16`
//! - file data, back to back
//! - index at `index_offset`: per file a 144-byte entry of a UTF-16BE name padded to 128
//!   bytes, `size: u32`, `offset: u32`, `timestamp: u64`

use crate::{Error, bad};

const HEADER: usize = 12;
const ENTRY: usize = 144;
const NAME_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    pub name: String,
    pub data: Vec<u8>,
    pub timestamp: u64,
    /// Offset as stored; entries with no data may share an offset with their neighbour.
    offset: u32,
}

impl ArchiveEntry {
    pub fn new(name: impl Into<String>, data: Vec<u8>, timestamp: u64) -> Self {
        ArchiveEntry {
            name: name.into(),
            data,
            timestamp,
            offset: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveArchive {
    pub original_version: u16,
    pub version: u16,
    pub entries: Vec<ArchiveEntry>,
    /// Whether the archive is little-endian (Windows 64, PS4, Vita, Xbox One).
    pub little: bool,
    /// The data area as read. Console saves can contain overlapping entries and gaps
    /// (the game reuses space and declares region sizes with slack), so an unmodified
    /// archive is written back from this verbatim.
    original_data: Option<Vec<u8>>,
}

fn u32_at(b: &[u8], o: usize, little: bool) -> u32 {
    let bytes = b[o..o + 4].try_into().unwrap();
    if little { u32::from_le_bytes(bytes) } else { u32::from_be_bytes(bytes) }
}

fn u32_bytes(v: u32, little: bool) -> [u8; 4] {
    if little { v.to_le_bytes() } else { v.to_be_bytes() }
}

fn u16_at(b: &[u8], o: usize, little: bool) -> u16 {
    let bytes = [b[o], b[o + 1]];
    if little { u16::from_le_bytes(bytes) } else { u16::from_be_bytes(bytes) }
}

impl SaveArchive {
    /// A new archive of the given file entries, laid out back to back when encoded.
    pub fn new(version: u16, entries: Vec<ArchiveEntry>, little: bool) -> Self {
        SaveArchive { original_version: version, version, entries, little, original_data: None }
    }

    /// Parses an archive in either byte order: the one whose index fits the file is used.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        match Self::parse_with(bytes, false) {
            Err(be) => Self::parse_with(bytes, true).map_err(|_| be),
            ok => ok,
        }
    }

    /// Parses an archive of the given byte order (little-endian on Windows 64, PS4, Vita and
    /// Xbox One).
    pub fn parse_with(bytes: &[u8], little: bool) -> Result<Self, Error> {
        if bytes.len() < HEADER {
            return Err(bad("save archive shorter than its header"));
        }
        let index = u32_at(bytes, 0, little) as usize;
        let count = u32_at(bytes, 4, little) as usize;
        let original_version = u16_at(bytes, 8, little);
        let version = u16_at(bytes, 10, little);
        let table = count
            .checked_mul(ENTRY)
            .and_then(|t| index.checked_add(t))
            .filter(|&end| end <= bytes.len())
            .ok_or_else(|| bad("save archive index outside the file"))?;
        if table != bytes.len() {
            return Err(bad(format!(
                "{} bytes after the save archive index",
                bytes.len() - table
            )));
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let e = &bytes[index + i * ENTRY..index + (i + 1) * ENTRY];
            let units: Vec<u16> = e[..NAME_BYTES]
                .chunks(2)
                .map(|c| u16_at(c, 0, little))
                .take_while(|&u| u != 0)
                .collect();
            let name = String::from_utf16(&units).map_err(|_| bad("file name is not UTF-16"))?;
            let size = u32_at(e, 128, little) as usize;
            let offset = u32_at(e, 132, little);
            let ts = e[136..144].try_into().unwrap();
            let timestamp = if little { u64::from_le_bytes(ts) } else { u64::from_be_bytes(ts) };
            let start = offset as usize;
            let data = bytes
                .get(start..start + size)
                .filter(|_| start >= HEADER && start + size <= index)
                .ok_or_else(|| bad(format!("{name} lies outside the data area")))?;
            entries.push(ArchiveEntry {
                name,
                data: data.to_vec(),
                timestamp,
                offset,
            });
        }
        Ok(SaveArchive {
            original_version,
            version,
            entries,
            little,
            original_data: Some(bytes[HEADER..index].to_vec()),
        })
    }

    /// Whether every entry still matches the bytes at its original offset.
    fn unmodified(&self) -> Option<&[u8]> {
        let data = self.original_data.as_deref()?;
        self.entries
            .iter()
            .all(|e| {
                let start = (e.offset as usize).wrapping_sub(HEADER);
                e.offset as usize >= HEADER
                    && data.get(start..start + e.data.len()) == Some(&e.data[..])
            })
            .then_some(data)
    }

    /// Serializes the archive. An unmodified archive reproduces its original bytes;
    /// otherwise files are laid out back to back in index order.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut out = vec![0u8; HEADER];
        let mut placed = Vec::with_capacity(self.entries.len());
        if let Some(data) = self.unmodified() {
            out.extend_from_slice(data);
            placed.extend(self.entries.iter().map(|e| e.offset));
        } else {
            for e in &self.entries {
                placed
                    .push(u32::try_from(out.len()).map_err(|_| bad("archive larger than 4 GiB"))?);
                out.extend_from_slice(&e.data);
            }
        }
        let index = u32::try_from(out.len()).map_err(|_| bad("archive larger than 4 GiB"))?;
        for (e, offset) in self.entries.iter().zip(placed) {
            let mut name = [0u8; NAME_BYTES];
            let units: Vec<u16> = e.name.encode_utf16().collect();
            if units.len() * 2 > NAME_BYTES {
                return Err(bad(format!("file name too long: {}", e.name)));
            }
            for (i, u) in units.iter().enumerate() {
                name[i * 2..i * 2 + 2].copy_from_slice(&if self.little { u.to_le_bytes() } else { u.to_be_bytes() });
            }
            out.extend_from_slice(&name);
            out.extend_from_slice(&u32_bytes(e.data.len() as u32, self.little));
            out.extend_from_slice(&u32_bytes(offset, self.little));
            out.extend_from_slice(&if self.little { e.timestamp.to_le_bytes() } else { e.timestamp.to_be_bytes() });
        }
        out[0..4].copy_from_slice(&u32_bytes(index, self.little));
        out[4..8].copy_from_slice(&u32_bytes(self.entries.len() as u32, self.little));
        for (range, v) in [(8..10, self.original_version), (10..12, self.version)] {
            out[range].copy_from_slice(&if self.little { v.to_le_bytes() } else { v.to_be_bytes() });
        }
        Ok(out)
    }

    pub fn get(&self, name: &str) -> Option<&ArchiveEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Region files with their dimension id (0 Overworld, -1 Nether, 1 End) and position.
    pub fn regions(&self) -> Vec<(i32, i32, i32, &ArchiveEntry)> {
        self.entries
            .iter()
            .filter_map(|e| {
                let (dim, rest) = if let Some(r) = e.name.strip_prefix("DIM-1") {
                    (-1, r)
                } else if let Some(r) = e.name.strip_prefix("DIM1/") {
                    (1, r)
                } else {
                    (0, e.name.as_str())
                };
                let mut parts = rest.strip_prefix("r.")?.strip_suffix(".mcr")?.split('.');
                let x = parts.next()?.parse().ok()?;
                let z = parts.next()?.parse().ok()?;
                parts.next().is_none().then_some((dim, x, z, e))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_relays_out_edits() {
        let archive = SaveArchive {
            original_version: 6,
            version: 6,
            entries: vec![
                ArchiveEntry::new("level.dat", vec![1, 2, 3], 42),
                ArchiveEntry::new("DIM-1r.0.-1.mcr", vec![], 0),
                ArchiveEntry::new("DIM1/r.-1.0.mcr", vec![9; 10], 7),
            ],
            little: false,
            original_data: None,
        };
        let bytes = archive.encode().unwrap();
        let mut parsed = SaveArchive::parse(&bytes).unwrap();
        assert_eq!(parsed.encode().unwrap(), bytes);
        assert_eq!(parsed.get("level.dat").unwrap().data, vec![1, 2, 3]);
        let regions: Vec<_> = parsed.regions().iter().map(|r| (r.0, r.1, r.2)).collect();
        assert_eq!(regions, vec![(-1, 0, -1), (1, -1, 0)]);

        parsed.entries[0].data = vec![5; 100];
        let edited = SaveArchive::parse(&parsed.encode().unwrap()).unwrap();
        assert_eq!(edited.get("level.dat").unwrap().data, vec![5; 100]);
        assert_eq!(edited.get("DIM1/r.-1.0.mcr").unwrap().data, vec![9; 10]);
    }

    #[test]
    fn overlapping_entries_survive_verbatim() {
        // Two entries sharing bytes, as the Xbox 360 game writes them.
        let mut bytes = vec![0u8; HEADER];
        bytes.extend_from_slice(b"abcdefgh");
        let index = bytes.len() as u32;
        for (name, off, size) in [("a", 12u32, 6u32), ("b", 16, 4)] {
            let mut n = [0u8; NAME_BYTES];
            n[1] = name.as_bytes()[0];
            bytes.extend_from_slice(&n);
            bytes.extend_from_slice(&size.to_be_bytes());
            bytes.extend_from_slice(&off.to_be_bytes());
            bytes.extend_from_slice(&0u64.to_be_bytes());
        }
        bytes[0..4].copy_from_slice(&index.to_be_bytes());
        bytes[4..8].copy_from_slice(&2u32.to_be_bytes());
        let a = SaveArchive::parse(&bytes).unwrap();
        assert_eq!(a.get("b").unwrap().data, b"efgh");
        assert_eq!(a.encode().unwrap(), bytes);
    }
}
