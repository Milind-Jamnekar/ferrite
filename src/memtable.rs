#[derive(Debug)]
pub enum Value {
    Data(Vec<u8>),
    Tombstone,
}

pub struct MemTable {
    map: std::collections::BTreeMap<Vec<u8>, Value>,
    size_bytes: usize,
}

impl MemTable {
    pub fn new() -> Self {
        Self {
            map: std::collections::BTreeMap::new(),
            size_bytes: 0,
        }
    }

    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.size_bytes += key.len() + value.len();
        self.map.insert(key, Value::Data(value));
    }

    pub fn delete(&mut self, key: Vec<u8>) {
        self.map.insert(key, Value::Tombstone);
    }

    pub fn get(&self, key: &[u8]) -> Option<&Value> {
        self.map.get(key)
    }
    pub fn size_bytes(&self) -> usize {
        self.size_bytes
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Vec<u8>, &Value)> {
        for (key, value) in self.map.iter() {
            println!("{key:?} {value:?}")
        }
        self.map.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_get_returns_data() {
        let mut mt = MemTable::new();
        mt.put(b"k".to_vec(), b"v".to_vec());
        match mt.get(b"k") {
            Some(Value::Data(v)) => assert_eq!(v, b"v"),
            other => panic!("expected Some(Data), got {other:?}"),
        }
    }

    #[test]
    fn delete_stores_tombstone() {
        let mut mt = MemTable::new();
        mt.put(b"k".to_vec(), b"v".to_vec());
        mt.delete(b"k".to_vec());
        assert!(matches!(mt.get(b"k"), Some(Value::Tombstone)));
    }

    #[test]
    fn missing_key_returns_none() {
        let mt = MemTable::new();
        assert!(mt.get(b"nope").is_none());
    }

    #[test]
    fn size_bytes_grows_with_puts() {
        let mut mt = MemTable::new();
        assert_eq!(mt.size_bytes(), 0);
        mt.put(b"key".to_vec(), b"value".to_vec());
        assert!(mt.size_bytes() > 0);
        mt.put(b"ab".to_vec(), b"cd".to_vec());
        assert_eq!(mt.size_bytes(), 3 + 5 + 2 + 2);
    }

    #[test]
    fn iter_is_sorted_by_key() {
        let mut mt = MemTable::new();
        mt.put(b"c".to_vec(), b"3".to_vec());
        mt.put(b"a".to_vec(), b"1".to_vec());
        mt.put(b"b".to_vec(), b"2".to_vec());
        let keys: Vec<_> = mt.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(keys, vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    }
}
