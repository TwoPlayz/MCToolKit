//! Log files (write-ahead logs and MANIFESTs): 32 KiB blocks of checksummed, possibly
//! fragmented records, per LevelDB's `doc/log_format.md`.

use crate::coding::Reader;
#[cfg(test)]
use crate::coding::put_varint;
use crate::crc32c::{crc32c, mask};
use crate::{Error, TYPE_DELETION, TYPE_VALUE, corrupt};

const BLOCK: usize = 32 * 1024;
const HEADER: usize = 7;
const FULL: u8 = 1;
const FIRST: u8 = 2;
const MIDDLE: u8 = 3;
const LAST: u8 = 4;

/// Reassembles every complete record. A truncated tail (an interrupted write) is ignored,
/// as LevelDB does on recovery.
pub fn read_records(data: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    let mut records = Vec::new();
    let mut pending: Option<Vec<u8>> = None;
    let mut pos = 0;
    while pos + HEADER <= data.len() {
        let block_left = BLOCK - pos % BLOCK;
        if block_left < HEADER {
            pos += block_left;
            continue;
        }
        let h = &data[pos..pos + HEADER];
        let stored_crc = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
        let len = u16::from_le_bytes([h[4], h[5]]) as usize;
        let kind = h[6];
        if kind == 0 && len == 0 {
            // Zero padding (preallocated space): skip to the next block.
            pos += block_left;
            continue;
        }
        let start = pos + HEADER;
        let Some(payload) = data.get(start..start + len) else {
            break;
        };
        if mask(crc32c(&[&[kind], payload])) != stored_crc {
            return Err(corrupt(format!(
                "log record checksum mismatch at offset {pos}"
            )));
        }
        match kind {
            FULL => records.push(payload.to_vec()),
            FIRST => pending = Some(payload.to_vec()),
            MIDDLE => pending
                .as_mut()
                .ok_or_else(|| corrupt("MIDDLE without FIRST"))?
                .extend_from_slice(payload),
            LAST => {
                let mut r = pending
                    .take()
                    .ok_or_else(|| corrupt("LAST without FIRST"))?;
                r.extend_from_slice(payload);
                records.push(r);
            }
            other => return Err(corrupt(format!("unknown log record type {other}"))),
        }
        pos = start + len;
    }
    Ok(records)
}

/// Encodes records into log blocks.
pub fn write_records(records: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for record in records {
        let mut rest: &[u8] = record;
        let mut first = true;
        loop {
            let block_left = BLOCK - out.len() % BLOCK;
            if block_left < HEADER {
                out.resize(out.len() + block_left, 0);
                continue;
            }
            let room = block_left - HEADER;
            let take = rest.len().min(room);
            let last = take == rest.len();
            let kind = match (first, last) {
                (true, true) => FULL,
                (true, false) => FIRST,
                (false, true) => LAST,
                (false, false) => MIDDLE,
            };
            let payload = &rest[..take];
            out.extend_from_slice(&mask(crc32c(&[&[kind], payload])).to_le_bytes());
            out.extend_from_slice(&(take as u16).to_le_bytes());
            out.push(kind);
            out.extend_from_slice(payload);
            rest = &rest[take..];
            first = false;
            if last {
                break;
            }
        }
    }
    out
}

/// Decodes a WriteBatch: `seq: u64, count: u32` then `count` put/delete records.
/// One write-batch operation: key, sequence number, and value (`None` = deletion).
pub type BatchOp = (Vec<u8>, u64, Option<Vec<u8>>);

pub fn parse_write_batch(batch: &[u8]) -> Result<Vec<BatchOp>, Error> {
    let mut r = Reader::new(batch);
    let seq = r.u64_le()?;
    let count = r.u32_le()?;
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count as u64 {
        let kind = r.bytes(1)?[0];
        let key = r.length_prefixed()?.to_vec();
        let value = match kind {
            TYPE_VALUE => Some(r.length_prefixed()?.to_vec()),
            TYPE_DELETION => None,
            other => return Err(corrupt(format!("unknown write batch record type {other}"))),
        };
        out.push((key, seq + i, value));
    }
    Ok(out)
}

/// Encodes a WriteBatch (used by tests to build logs).
#[cfg(test)]
pub fn encode_write_batch(seq: u64, ops: &[(Vec<u8>, Option<Vec<u8>>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&(ops.len() as u32).to_le_bytes());
    for (key, value) in ops {
        out.push(if value.is_some() {
            TYPE_VALUE
        } else {
            TYPE_DELETION
        });
        put_varint(&mut out, key.len() as u64);
        out.extend_from_slice(key);
        if let Some(v) = value {
            put_varint(&mut out, v.len() as u64);
            out.extend_from_slice(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip_across_blocks() {
        let records = vec![
            b"small".to_vec(),
            vec![7u8; 100_000],
            Vec::new(),
            vec![1u8; 32 * 1024 - 7],
        ];
        let encoded = write_records(&records);
        assert_eq!(read_records(&encoded).unwrap(), records);
    }

    #[test]
    fn corrupted_record_is_detected() {
        let mut encoded = write_records(&[b"hello".to_vec()]);
        encoded[8] ^= 1;
        assert!(read_records(&encoded).is_err());
    }

    #[test]
    fn write_batches_round_trip() {
        let ops = vec![(b"a".to_vec(), Some(b"1".to_vec())), (b"b".to_vec(), None)];
        let parsed = parse_write_batch(&encode_write_batch(10, &ops)).unwrap();
        assert_eq!(
            parsed,
            vec![
                (b"a".to_vec(), 10, Some(b"1".to_vec())),
                (b"b".to_vec(), 11, None)
            ]
        );
    }
}
