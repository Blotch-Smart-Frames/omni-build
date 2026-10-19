//! Reading the binary formats inside app packages: zip archives, Android's
//! binary XML and resource tables, and iOS asset catalogs. They're
//! little-endian but for the BOM store an asset catalog is kept in.

/// A cursor over bytes whose reads never fail on their own.
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

    pub fn u8(&mut self) -> u8 {
        u8::from_le_bytes(self.array())
    }

    pub fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.array())
    }

    pub fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.array())
    }

    pub fn u16_be(&mut self) -> u16 {
        u16::from_be_bytes(self.array())
    }

    pub fn u32_be(&mut self) -> u32 {
        u32::from_be_bytes(self.array())
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
        let mut reader = Reader::at(&[0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07], 1);

        assert_eq!(reader.u8(), 0x01);
        assert_eq!(reader.u16(), 0x0302);
        assert_eq!(reader.u32(), 0x0706_0504);
        assert!(!reader.overrun());
    }

    #[test]
    fn reads_big_endian_integers_too() {
        let mut reader = Reader::at(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06], 0);

        assert_eq!(reader.u16_be(), 0x0102);
        assert_eq!(reader.u32_be(), 0x0304_0506);
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
