use std::io::Write;

#[derive(Debug)]
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

pub struct Wal {
    file: std::fs::File,
}

impl Wal {
    pub fn create(path: &std::path::Path) -> crate::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .append(true)
            .create_new(true)
            .open(path)?;
        Ok(Self { file })
    }
    pub fn open_append(path: &std::path::Path) -> crate::Result<Self> {
        let file = std::fs::OpenOptions::new().append(true).open(path)?;
        Ok(Self { file })
    }
    pub fn append(&mut self, entry: &WalEntry) -> crate::Result<()> {
        // encodes + writes + fsyncs
        let bytes = encode_entry(entry);
        self.file.write_all(&bytes)?;
        self.file.sync_all()?;
        Ok(())
    }
    pub fn replay(path: &std::path::Path) -> crate::Result<Vec<WalEntry>> {
        // read the whole file into memory
        let data = std::fs::read(path)?;
        let mut entries = Vec::new();
        let mut pos = 0;

        while pos < data.len() {
            match decode_entry(&data[pos..]) {
                Ok((entry, consumed)) => {
                    entries.push(entry);
                    pos += consumed;
                }
                Err(e) => {
                    //check reamining bytes from files
                    if data.len() - pos < 8 {
                        break;
                    }

                    let entry_len = u32::from_le_bytes(
                        data[pos + 4..pos + 8]
                            .try_into()
                            .expect("Error extracting in entry_len"),
                    ) as usize;

                    if pos + 8 + entry_len < data.len() {
                        return Err(e);
                    }

                    break;
                }
            }
        }

        Ok(entries)
    }
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
    fn truncate_incomplete_payload() {
        let entry = WalEntry::Put {
            key: b"test".to_vec(),
            value: b"test".to_vec(),
        };

        let bytes = encode_entry(&entry);
        let result = decode_entry(&bytes[..10]);
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

    #[test]
    fn replay_short_torn_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let mut wal: Wal = Wal::create(&path).unwrap();
        wal.append(&WalEntry::Put {
            key: b"a".to_vec(),
            value: b"1".to_vec(),
        })
        .unwrap();
        let new_bytes: [u8; 3] = [1, 2, 3];
        let mut good_bytes: Vec<u8> = std::fs::read(&path).expect("Error in reading file");
        good_bytes.extend_from_slice(&new_bytes);
        std::fs::write(&path, good_bytes).expect("Error on writing file");

        let entries = Wal::replay(&path).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn replay_first_entry_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let mut wal: Wal = Wal::create(&path).unwrap();
        wal.append(&WalEntry::Put {
            key: b"a".to_vec(),
            value: b"1".to_vec(),
        })
        .unwrap();

        wal.append(&WalEntry::Put {
            key: b"a".to_vec(),
            value: b"1".to_vec(),
        })
        .unwrap();

        let mut bytes: Vec<u8> = std::fs::read(&path).expect("Error in reading file");
        bytes[10] ^= 0xFF;
        std::fs::write(&path, bytes).expect("Error on writing file");

        let entries = Wal::replay(&path);
        assert!(matches!(entries, Err(crate::Error::Corruption(_))));
    }

    #[test]
    fn append_then_replay_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let mut wal: Wal = Wal::create(&path).unwrap();
        wal.append(&WalEntry::Put {
            key: b"a".to_vec(),
            value: b"1".to_vec(),
        })
        .unwrap();
        wal.append(&WalEntry::Delete { key: b"b".to_vec() })
            .unwrap();
        drop(wal);

        let entries = Wal::replay(&path).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn replay_discards_torn_final_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let mut wal = Wal::create(&path).unwrap();
        wal.append(&WalEntry::Put {
            key: b"a".to_vec(),
            value: b"1".to_vec(),
        })
        .unwrap();
        drop(wal);

        // simulate a crash mid-write: append a second, well-formed entry,
        // then truncate the file partway through it
        let good_bytes = encode_entry(&WalEntry::Put {
            key: b"b".to_vec(),
            value: b"2".to_vec(),
        });
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(&good_bytes[..good_bytes.len() - 2]).unwrap(); // drop last 2 bytes

        let entries = Wal::replay(&path).unwrap();
        assert_eq!(entries.len(), 1); // only the first, complete entry survives
    }

    #[test]
    fn open_append_adds_to_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wal");

        Wal::create(&path)
            .unwrap()
            .append(&WalEntry::Put {
                key: b"a".to_vec(),
                value: b"1".to_vec(),
            })
            .unwrap();

        let mut wal = Wal::open_append(&path).unwrap();
        wal.append(&WalEntry::Put {
            key: b"b".to_vec(),
            value: b"2".to_vec(),
        })
        .unwrap();
        drop(wal);

        assert_eq!(Wal::replay(&path).unwrap().len(), 2);
    }
}
