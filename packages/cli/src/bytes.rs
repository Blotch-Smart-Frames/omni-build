//! Reading the little-endian binary formats inside app packages: zip
//! archives, and later Android's binary XML and resource tables.

/// A cursor over little-endian bytes whose reads never fail on their own.
/// A read past the end returns zeros (or no bytes) and marks the reader as
/// overrun, which the caller checks once when it has read a whole
/// structure. Shells come from files users hand OmniBuild, so a truncated
/// one must give an error rather than a panic; checking every read
/// separately would bury each format's layout under error handling.
pub struct Reader<'a> {
    data: &'a [u8],
    position: usize,
    overrun: bool,
}

impl<'a> Reader<'a> {
    /// A reader starting at `position`, which may be past the end: the
    /// first read then overruns.
    pub fn at(data: &'a [u8], position: usize) -> Self {
        Self {
            data,
            position,
            overrun: false,
        }
    }

    pub fn bytes(&mut self, len: usize) -> &'a [u8] {
        let range = self.position..self.position.saturating_add(len);
        if let Some(bytes) = self.data.get(range) {
            self.position += len;
            bytes
        } else {
            self.overrun = true;
            self.position = self.data.len();
            &[]
        }
    }

    pub fn skip(&mut self, len: usize) {
        self.bytes(len);
    }

    fn array<const N: usize>(&mut self) -> [u8; N] {
        self.bytes(N).try_into().unwrap_or([0; N])
    }

    pub fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.array())
    }

    pub fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.array())
    }

    /// Whether any read so far went past the end.
    pub fn overrun(&self) -> bool {
        self.overrun
    }
}

#[cfg(test)]
mod tests {
    use super::Reader;

    #[test]
    fn reads_little_endian_integers_in_order() {
        let mut reader = Reader::at(&[0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06], 1);

        assert_eq!(reader.u16(), 0x0201);
        assert_eq!(reader.u32(), 0x0605_0403);
        assert!(!reader.overrun());
    }

    #[test]
    fn a_read_past_the_end_gives_zeros_and_marks_the_reader() {
        let mut reader = Reader::at(&[1, 2, 3], 0);
        reader.skip(2);

        assert_eq!(reader.u32(), 0);
        assert!(reader.overrun());
        // Everything after an overrun is past the end too.
        assert_eq!(reader.bytes(1), &[] as &[u8]);
    }

    #[test]
    fn a_reader_can_start_past_the_end() {
        let mut reader = Reader::at(&[1, 2], 5);

        assert_eq!(reader.u16(), 0);
        assert!(reader.overrun());
    }
}
