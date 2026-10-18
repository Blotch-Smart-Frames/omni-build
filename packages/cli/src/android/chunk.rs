//! The chunks Android's binary resource formats are made of
//! (`ResChunk_header` in AOSP's `ResourceTypes.h`): binary XML and
//! resource tables alike. Each chunk starts with its type, its header's
//! size and its own size, so a reader can step over the ones it doesn't
//! need.

use anyhow::{Result, bail};

use crate::bytes::Reader;

pub struct Chunk {
    pub kind: u16,
    pub start: usize,
    pub header_size: usize,
    pub size: usize,
}

impl Chunk {
    /// Where the chunk's contents start, after its header.
    pub fn body(&self) -> usize {
        self.start + self.header_size
    }

    pub fn end(&self) -> usize {
        self.start + self.size
    }
}

/// The chunks one after another from `start` to `end` in `data`.
pub fn chunks(data: &[u8], start: usize, end: usize) -> Result<Vec<Chunk>> {
    let mut chunks = Vec::new();
    let mut position = start;
    while position < end {
        let mut header = Reader::at(data, position);
        let (kind, header_size, size) = (
            header.u16(),
            usize::from(header.u16()),
            usize::try_from(header.u32()).expect("OmniBuild runs on 32 and 64-bit systems"),
        );
        // A chunk smaller than its own header would never end.
        if size < 8 || header_size > size || position.saturating_add(size) > end.min(data.len()) {
            bail!("it's corrupt or cut short");
        }
        chunks.push(Chunk {
            kind,
            start: position,
            header_size,
            size,
        });
        position += size;
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::chunks;

    fn chunk(kind: u16, header_size: u16, size: u32) -> Vec<u8> {
        let mut bytes = [kind.to_le_bytes(), header_size.to_le_bytes()].concat();
        bytes.extend(size.to_le_bytes());
        bytes.resize(usize::try_from(size).unwrap().max(8), 0);
        bytes
    }

    #[test]
    fn steps_through_chunks() {
        let data = [chunk(1, 8, 12), chunk(2, 10, 16)].concat();

        let chunks = chunks(&data, 0, data.len()).unwrap();

        let read: Vec<_> = chunks
            .iter()
            .map(|chunk| (chunk.kind, chunk.start, chunk.body(), chunk.end()))
            .collect();
        assert_eq!(read, [(1, 0, 8, 12), (2, 12, 22, 28)]);
    }

    #[test]
    fn rejects_chunks_that_dont_fit() {
        for data in [
            chunk(1, 8, 4),
            chunk(1, 12, 8),
            [chunk(1, 8, 8), chunk(2, 8, 100)].concat(),
        ] {
            let end = data.len();
            assert_eq!(
                chunks(&data[..end.min(16)], 0, end)
                    .err()
                    .unwrap()
                    .to_string(),
                "it's corrupt or cut short"
            );
        }
    }
}
