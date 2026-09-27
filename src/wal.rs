pub enum WalEntry {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

pub fn encode_entry(entry: &WalEntry) -> Vec<u8> {
    let mut payload: Vec<u8> = Vec::new();

    // First op byte
    // then key_len, key_value, value_len and value
    match entry {
        WalEntry::Put { key, value } => {
            payload.push(0u8);

            let key_len: u32 = key.len() as u32;
            payload.extend_from_slice(&key_len.to_le_bytes());
            payload.extend_from_slice(key);

            let value_len: u32 = value.len() as u32;
            payload.extend_from_slice(&value_len.to_le_bytes());
            payload.extend_from_slice(value);
        }
        WalEntry::Delete { key } => {
            payload.push(1u8);
            let key_len: u32 = key.len() as u32;
            payload.extend_from_slice(&key_len.to_le_bytes());
            payload.extend_from_slice(key);
        }
    }

    // then calculate entry_len
    let payload_len: u32 = payload.len() as u32;
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&payload_len.to_le_bytes());
    body.extend_from_slice(&payload);

    // hash it and stored as CRC32.
    let hash: u32 = crc32fast::hash(&body);
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&hash.to_le_bytes());
    out.extend_from_slice(&body);

    // return it.
    out
}

pub fn decode_entry(bytes: &[u8]) -> crate::Result<(WalEntry, usize)> {
    // check bytes.len() >= 8,
    if bytes.len() < 8 {
        return Err(crate::Error::Corruption(
            "wal entry is too short".to_string(),
        ));
    }

    // read stored crc from bytes[0..4], entry_len from bytes[4..8]
    let crc_checksum = &bytes[0..4];
    let entry_len = u32::from_le_bytes(
        bytes[4..8]
            .try_into()
            .expect("error on unwrapping entry_len"),
    ) as usize;

    // check bytes.len() >= 8 + entry_len, else Corruption
    if bytes.len() < 8 + entry_len {
        return Err(crate::Error::Corruption(
            "wal entry is too short".to_string(),
        ));
    }

    // hash bytes[4..8 + entry_len], compare to stored crc, mismatch → Corruption
    let new_checksum = crc32fast::hash(&bytes[4..8 + entry_len]).to_le_bytes();

    if crc_checksum != new_checksum {
        return Err(crate::Error::Corruption(
            "crc checksum does not match".to_string(),
        ));
    }

    // parse key and values and op
    let op = bytes[8];
    let mut pos = 9;
    let key_len: usize = u32::from_le_bytes(
        bytes[pos..pos + 4]
            .try_into()
            .expect("error in unwrapping in key_len"),
    ) as usize;

    pos += 4;
    let key = bytes[pos..pos + key_len].to_vec();
    pos += key_len;

    let entry = match op {
        0 => {
            let value_len: usize = u32::from_le_bytes(
                bytes[pos..pos + 4]
                    .try_into()
                    .expect("error in unwrapping in value_len"),
            ) as usize;
            pos += 4;
            let value = bytes[pos..pos + value_len].to_vec();
            pos += value_len;

            WalEntry::Put { key, value }
        }
        1 => WalEntry::Delete { key },
        _ => {
            return Err(crate::Error::Corruption(
                "operation is not correct".to_string(),
            ));
        }
    };

    if pos != 8 + entry_len {
        return Err(crate::Error::Corruption(
            "Position has some error".to_string(),
        ));
    }

    Ok((entry, 8 + entry_len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_layout() {
        let bytes = encode_entry(&WalEntry::Put {
            key: b"hello".to_vec(),
            value: b"world".to_vec(),
        });
        assert_eq!(bytes.len(), 27);
        assert_eq!(&bytes[4..8], &19u32.to_le_bytes());
        assert_eq!(bytes[8], 0);
    }

    #[test]
    fn encode_with_different_layout() {
        let bytes = encode_entry(&WalEntry::Put {
            key: b"me".to_vec(),
            value: b"you".to_vec(),
        });
        assert_eq!(bytes.len(), 22);
        // op = 1, me_len = 4, me => 2, you_len = 4, you = 3 = 14
        assert_eq!(&bytes[15..19], 3u32.to_le_bytes());
        assert_eq!(bytes[8], 0);
    }

    #[test]
    fn truncate_less_buffer() {
        let mut bytes = Vec::new();
        bytes.push(1);
        let result = decode_entry(&bytes);
        assert!(matches!(result, Err(crate::Error::Corruption(_))));
    }

    #[test]
    fn put_round_trips() {
        let entry = WalEntry::Put {
            key: b"hello".to_vec(),
            value: b"world".to_vec(),
        };
        let bytes = encode_entry(&entry);
        let (decoded, consumed) = decode_entry(&bytes).unwrap();

        assert_eq!(consumed, bytes.len());
        match decoded {
            WalEntry::Put { key, value } => {
                assert_eq!(key, b"hello");
                assert_eq!(value, b"world");
            }
            _ => panic!("expected Put"),
        }
    }

    #[test]
    fn delete_round_trips() {
        let entry = WalEntry::Delete {
            key: b"gone".to_vec(),
        };
        let bytes = encode_entry(&entry);
        let (decoded, consumed) = decode_entry(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        match decoded {
            WalEntry::Delete { key } => assert_eq!(key, b"gone"),
            _ => panic!("expected Delete"),
        }
    }

    #[test]
    fn corrupt_crc_is_rejected() {
        let entry = WalEntry::Put {
            key: b"k".to_vec(),
            value: b"v".to_vec(),
        };
        let mut bytes = encode_entry(&entry);
        // flip a bit in the key, leaving the CRC stale
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        let result = decode_entry(&bytes);
        assert!(matches!(result, Err(crate::Error::Corruption(_))));
    }
}
