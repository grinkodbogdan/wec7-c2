//! Reading APKs (zip archives): the central directory, stored and deflated
//! entries.

use std::collections::BTreeMap;

pub struct Zip {
    data: Vec<u8>,
    entries: BTreeMap<String, (usize, u16, usize, usize)>, // local header offset, method, comp, uncomp
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

impl Zip {
    pub fn parse(data: Vec<u8>) -> Result<Zip, String> {
        let eocd = (0..data.len().saturating_sub(21))
            .rev()
            .find(|&i| u32_at(&data, i) == Some(0x0605_4b50))
            .ok_or("not a zip archive (no end of central directory)")?;
        let count = u16_at(&data, eocd + 10).ok_or("truncated")? as usize;
        let mut off = u32_at(&data, eocd + 16).ok_or("truncated")? as usize;
        let mut entries = BTreeMap::new();
        for _ in 0..count {
            if u32_at(&data, off) != Some(0x0201_4b50) {
                return Err("bad central directory entry".into());
            }
            let method = u16_at(&data, off + 10).ok_or("truncated")?;
            let comp = u32_at(&data, off + 20).ok_or("truncated")? as usize;
            let uncomp = u32_at(&data, off + 24).ok_or("truncated")? as usize;
            let nlen = u16_at(&data, off + 28).ok_or("truncated")? as usize;
            let elen = u16_at(&data, off + 30).ok_or("truncated")? as usize;
            let clen = u16_at(&data, off + 32).ok_or("truncated")? as usize;
            let local = u32_at(&data, off + 42).ok_or("truncated")? as usize;
            let name = String::from_utf8_lossy(data.get(off + 46..off + 46 + nlen).ok_or("truncated")?).into_owned();
            entries.insert(name, (local, method, comp, uncomp));
            off += 46 + nlen + elen + clen;
        }
        Ok(Zip { data, entries })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(|s| s.as_str())
    }

    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        let &(local, method, comp, uncomp) = self.entries.get(name)?;
        if u32_at(&self.data, local)? != 0x0403_4b50 {
            return None;
        }
        let nlen = u16_at(&self.data, local + 26)? as usize;
        let elen = u16_at(&self.data, local + 28)? as usize;
        let start = local + 30 + nlen + elen;
        let raw = self.data.get(start..start + comp)?;
        match method {
            0 => Some(raw.to_vec()),
            8 => miniz_oxide::inflate::decompress_to_vec(raw).ok().filter(|v| v.len() == uncomp),
            _ => None,
        }
    }
}
