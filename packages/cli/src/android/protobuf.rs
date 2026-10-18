//! Protocol Buffers at the wire level, for the messages inside an Android
//! App Bundle: aapt2's `Resources.proto` (the manifest and the resource
//! table).
//!
//! OmniBuild changes a few fields and must keep every other as aapt2 wrote
//! it, including fields added to the format after OmniBuild was written.
//! Code generated from the `.proto` files would drop fields it doesn't
//! know, so messages stay as their fields, in order, and only the ones
//! being changed are decoded further.

use anyhow::{Result, bail};

/// A field's value as its wire type stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Wire {
    Varint(u64),
    Fixed64([u8; 8]),
    /// A string, bytes, a nested message or packed repeated values.
    Bytes(Vec<u8>),
    Fixed32([u8; 4]),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub number: u32,
    pub value: Wire,
}

impl Field {
    pub fn bytes(number: u32, value: impl Into<Vec<u8>>) -> Self {
        Self {
            number,
            value: Wire::Bytes(value.into()),
        }
    }

    pub fn varint(number: u32, value: u64) -> Self {
        Self {
            number,
            value: Wire::Varint(value),
        }
    }
}

/// A message's fields, in the order they're written.
pub fn decode(message: &[u8]) -> Result<Vec<Field>> {
    let mut fields = Vec::new();
    let mut position = 0;
    while position < message.len() {
        let Some(field) = decode_field(message, &mut position) else {
            bail!("it's corrupt or cut short");
        };
        fields.push(field);
    }
    Ok(fields)
}

fn decode_field(message: &[u8], position: &mut usize) -> Option<Field> {
    let key = read_varint(message, position)?;
    let number = u32::try_from(key >> 3).ok()?;
    let value = match key & 7 {
        0 => Wire::Varint(read_varint(message, position)?),
        1 => Wire::Fixed64(
            take(message, position, 8)?
                .try_into()
                .expect("take gives the length asked for"),
        ),
        2 => {
            // Too long for memory is too long for the message.
            let len = usize::try_from(read_varint(message, position)?).unwrap_or(usize::MAX);
            Wire::Bytes(take(message, position, len)?.to_vec())
        }
        5 => Wire::Fixed32(
            take(message, position, 4)?
                .try_into()
                .expect("take gives the length asked for"),
        ),
        // Groups (3 and 4) are long deprecated, and aapt2 doesn't write them.
        _ => return None,
    };
    Some(Field { number, value })
}

fn read_varint(message: &[u8], position: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *message.get(*position)?;
        *position += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(value);
        }
    }
    None
}

fn take<'a>(message: &'a [u8], position: &mut usize, len: usize) -> Option<&'a [u8]> {
    let bytes = message.get(*position..position.checked_add(len)?)?;
    *position += len;
    Some(bytes)
}

pub fn encode(fields: &[Field]) -> Vec<u8> {
    let mut message = Vec::new();
    for field in fields {
        let (wire_type, payload) = match &field.value {
            Wire::Varint(value) => (0, encode_varint(*value)),
            Wire::Fixed64(bytes) => (1, bytes.to_vec()),
            Wire::Bytes(bytes) => (
                2,
                [encode_varint(bytes.len() as u64), bytes.clone()].concat(),
            ),
            Wire::Fixed32(bytes) => (5, bytes.to_vec()),
        };
        message.extend(encode_varint(u64::from(field.number) << 3 | wire_type));
        message.extend(payload);
    }
    message
}

fn encode_varint(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    while value >= 0x80 {
        bytes.push(u8::try_from(value & 0x7f).expect("seven bits fit a byte") | 0x80);
        value >>= 7;
    }
    bytes.push(u8::try_from(value).expect("under 0x80 fits a byte"));
    bytes
}

/// The first field with this number, as bytes: a string or a message.
pub fn bytes(fields: &[Field], number: u32) -> Option<&[u8]> {
    fields.iter().find_map(|field| match &field.value {
        Wire::Bytes(bytes) if field.number == number => Some(bytes.as_slice()),
        _ => None,
    })
}

/// The first field with this number as a varint. A missing one is 0 in
/// proto3, except in a `oneof`, where it's the field that's set.
pub fn uint(fields: &[Field], number: u32) -> Option<u64> {
    fields.iter().find_map(|field| match field.value {
        Wire::Varint(value) if field.number == number => Some(value),
        _ => None,
    })
}

/// The first field with this number as a string, or nothing.
pub fn string(fields: &[Field], number: u32) -> Option<String> {
    bytes(fields, number).map(|bytes| String::from_utf8_lossy(bytes).into_owned())
}

/// The nested message in the first field with this number. A missing or
/// unreadable one reads as empty, as a missing one is in proto3.
pub fn message(fields: &[Field], number: u32) -> Vec<Field> {
    bytes(fields, number)
        .and_then(|bytes| decode(bytes).ok())
        .unwrap_or_default()
}

/// Replaces every field with this number by `field`, or adds it.
pub fn set(fields: &mut Vec<Field>, field: Field) {
    fields.retain(|other| other.number != field.number);
    fields.push(field);
}

#[cfg(test)]
mod tests {
    use super::{Field, Wire, bytes, decode, encode, message, set, string, uint};

    #[test]
    fn decodes_and_encodes_every_wire_type() {
        let fields = vec![
            Field::varint(1, 300),
            Field {
                number: 2,
                value: Wire::Fixed64([1; 8]),
            },
            Field::bytes(3, "text"),
            Field {
                number: 4,
                value: Wire::Fixed32([2; 4]),
            },
            Field::varint(1000, u64::MAX),
        ];

        let encoded = encode(&fields);

        assert_eq!(&encoded[..3], [0x08, 0xac, 0x02]);
        assert_eq!(decode(&encoded).unwrap(), fields);
    }

    #[test]
    fn reads_fields_by_number() {
        let nested = encode(&[Field::varint(1, 7)]);
        let mut fields = vec![
            Field::bytes(1, "first"),
            Field::bytes(1, "second"),
            Field::bytes(2, nested),
            Field::varint(3, 5),
            Field::bytes(4, [0xff]),
        ];

        assert_eq!(string(&fields, 1).as_deref(), Some("first"));
        assert_eq!(uint(&message(&fields, 2), 1), Some(7));
        assert_eq!(uint(&fields, 3), Some(5));
        assert_eq!(uint(&fields, 1), None);
        assert_eq!(bytes(&fields, 3), None);
        // A field that isn't a message reads as an empty one.
        assert_eq!(message(&fields, 4), []);
        assert_eq!(message(&fields, 9), []);

        set(&mut fields, Field::bytes(1, "only"));
        assert_eq!(string(&fields, 1).as_deref(), Some("only"));
        assert_eq!(fields.len(), 4);
    }

    #[test]
    fn rejects_a_corrupt_message() {
        for corrupt in [
            &[0x08, 0x80][..],   // A varint cut short.
            &[0x12],             // A length cut short.
            &[0x12, 0x05, 0x61], // Bytes cut short.
            &[
                0x12, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01,
            ], // Longer than memory.
            &[0x09, 0x01],       // Fixed64 cut short.
            &[0x0d, 0x01],       // Fixed32 cut short.
            &[0x0b],             // A group.
            &[0xff; 11],         // A varint over 64 bits.
            &[0x80, 0x80, 0x80, 0x80, 0x80, 0x01], // A field number over 32 bits.
        ] {
            assert_eq!(
                decode(corrupt).unwrap_err().to_string(),
                "it's corrupt or cut short",
                "{corrupt:x?}"
            );
        }
    }
}
