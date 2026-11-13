//! Minimal CP/M 2.2 file system on Bosman-8 floppy images (for `b8disk`).
//!
//! Geometry comes from the DPB tables in the ROM (see `doc/bosman8-hardware.md`):
//! OFF = 0 and the image is stored in logical-track order, so block `n`
//! simply starts at byte `n * block_size` of the image.

use crate::disk::Format;

/// CP/M disk parameters needed for directory/allocation handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    pub block_size: usize,
    /// Number of blocks (DSM + 1).
    pub blocks: usize,
    /// Number of directory entries (DRM + 1).
    pub dir_entries: usize,
    /// Extent mask (EXM).
    pub exm: u8,
}

impl Params {
    /// Parameters for a floppy format (from the ROM DPB tables).
    pub fn for_format(f: Format) -> Params {
        match (f.cylinders, f.sides) {
            (40, 1) => Params { block_size: 1024, blocks: 200, dir_entries: 64, exm: 0 },
            (40, 2) => Params { block_size: 2048, blocks: 200, dir_entries: 128, exm: 1 },
            _ => Params { block_size: 2048, blocks: 400, dir_entries: 192, exm: 0 },
        }
    }
    fn wide_pointers(&self) -> bool {
        self.blocks > 256
    }
    fn ptrs_per_entry(&self) -> usize {
        if self.wide_pointers() { 8 } else { 16 }
    }
    fn dir_blocks(&self) -> usize {
        (self.dir_entries * 32).div_ceil(self.block_size)
    }
    /// Bytes addressed by one directory entry.
    fn entry_capacity(&self) -> usize {
        self.ptrs_per_entry() * self.block_size
    }
}

/// A file found in the directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub user: u8,
    pub name: String,
    pub size: usize,
}

pub struct CpmFs<'a> {
    pub p: Params,
    img: &'a mut Vec<u8>,
}

fn split_name(name: &str) -> Result<([u8; 8], [u8; 3]), String> {
    let up = name.to_ascii_uppercase();
    let (base, ext) = match up.rsplit_once('.') {
        Some((b, e)) => (b.to_string(), e.to_string()),
        None => (up.clone(), String::new()),
    };
    if base.is_empty() || base.len() > 8 || ext.len() > 3 {
        return Err(format!("'{name}' is not a valid 8.3 CP/M file name"));
    }
    let bad = |c: char| !c.is_ascii_graphic() || "<>.,;:=?*[]|/\\".contains(c);
    if base.chars().any(bad) || ext.chars().any(bad) {
        return Err(format!("'{name}' contains characters not allowed in CP/M names"));
    }
    let mut n = [b' '; 8];
    let mut e = [b' '; 3];
    n[..base.len()].copy_from_slice(base.as_bytes());
    e[..ext.len()].copy_from_slice(ext.as_bytes());
    Ok((n, e))
}

fn entry_name(e: &[u8]) -> String {
    let n: String = e[1..9].iter().map(|&c| (c & 0x7f) as char).collect();
    let x: String = e[9..12].iter().map(|&c| (c & 0x7f) as char).collect();
    let n = n.trim_end();
    let x = x.trim_end();
    if x.is_empty() { n.to_string() } else { format!("{n}.{x}") }
}

impl<'a> CpmFs<'a> {
    pub fn new(img: &'a mut Vec<u8>, p: Params) -> Result<Self, String> {
        if img.len() < p.blocks * p.block_size {
            return Err(format!(
                "image too small: {} bytes, expected {}",
                img.len(),
                p.blocks * p.block_size
            ));
        }
        Ok(CpmFs { p, img })
    }

    fn entry(&self, i: usize) -> &[u8] {
        &self.img[i * 32..i * 32 + 32]
    }

    fn entry_mut(&mut self, i: usize) -> &mut [u8] {
        &mut self.img[i * 32..i * 32 + 32]
    }

    fn entry_blocks(&self, e: &[u8]) -> Vec<usize> {
        let mut v = Vec::new();
        if self.p.wide_pointers() {
            for k in 0..8 {
                let b = u16::from_le_bytes([e[16 + 2 * k], e[17 + 2 * k]]) as usize;
                if b != 0 { v.push(b); }
            }
        } else {
            for &b in &e[16..32] {
                if b != 0 { v.push(b as usize); }
            }
        }
        v
    }

    fn used_blocks(&self) -> Vec<bool> {
        let mut used = vec![false; self.p.blocks];
        for b in used.iter_mut().take(self.p.dir_blocks()) {
            *b = true;
        }
        for i in 0..self.p.dir_entries {
            let e = self.entry(i);
            if e[0] == 0xE5 || e[0] > 31 { continue; }
            for b in self.entry_blocks(e) {
                if b < used.len() { used[b] = true; }
            }
        }
        used
    }

    fn matches(e: &[u8], user: u8, n: &[u8; 8], x: &[u8; 3]) -> bool {
        e[0] == user
            && e[1..9].iter().zip(n).all(|(a, b)| a & 0x7f == *b)
            && e[9..12].iter().zip(x).all(|(a, b)| a & 0x7f == *b)
    }

    /// List files (all users).
    pub fn list(&self) -> Vec<FileInfo> {
        let mut files: Vec<(u8, String, usize, usize)> = Vec::new(); // user,name,extent,size
        for i in 0..self.p.dir_entries {
            let e = self.entry(i);
            if e[0] > 31 { continue; }
            let name = entry_name(e);
            let ext_no = (e[12] as usize & 0x1f) | ((e[14] as usize & 0x3f) << 5);
            let rc = e[15] as usize;
            // Bytes up to and including this entry.
            let logical_ext = ext_no; // 16 KB units
            let size = logical_ext * 16384 + rc * 128;
            match files.iter_mut().find(|f| f.0 == e[0] && f.1 == name) {
                Some(f) => {
                    if size > f.3 { f.3 = size; }
                    f.2 += 1;
                }
                None => files.push((e[0], name, 1, size)),
            }
        }
        files.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        files.into_iter().map(|(user, name, _, size)| FileInfo { user, name, size }).collect()
    }

    /// Read a file's contents (multiple of 128 bytes).
    pub fn get(&self, user: u8, name: &str) -> Result<Vec<u8>, String> {
        let (n, x) = split_name(name)?;
        let mut parts: Vec<(usize, Vec<usize>, usize)> = Vec::new();
        for i in 0..self.p.dir_entries {
            let e = self.entry(i);
            if Self::matches(e, user, &n, &x) {
                let ext_no = (e[12] as usize & 0x1f) | ((e[14] as usize & 0x3f) << 5);
                parts.push((ext_no, self.entry_blocks(e), e[15] as usize));
            }
        }
        if parts.is_empty() {
            return Err(format!("{user}:{name}: no such file"));
        }
        parts.sort();
        let mut out = Vec::new();
        let cap = self.p.entry_capacity();
        for (ext_no, blocks, rc) in parts {
            let entry_index = ext_no / (self.p.exm as usize + 1);
            let start = entry_index * cap;
            let end = ext_no * 16384 + rc * 128;
            let len = end.saturating_sub(start).min(cap);
            if out.len() < start { out.resize(start, 0x1A); }
            let mut chunk = Vec::new();
            for b in blocks {
                let o = b * self.p.block_size;
                chunk.extend_from_slice(&self.img[o..o + self.p.block_size]);
            }
            chunk.resize(len, 0x1A);
            out.truncate(start);
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    /// Delete a file. Returns false if it did not exist.
    pub fn remove(&mut self, user: u8, name: &str) -> Result<bool, String> {
        let (n, x) = split_name(name)?;
        let mut found = false;
        for i in 0..self.p.dir_entries {
            if Self::matches(self.entry(i), user, &n, &x) {
                self.entry_mut(i)[0] = 0xE5;
                found = true;
            }
        }
        Ok(found)
    }

    /// Write a file (replacing an existing one with the same name).
    pub fn put(&mut self, user: u8, name: &str, data: &[u8]) -> Result<(), String> {
        if user > 15 {
            return Err("user number must be 0..15".into());
        }
        let (n, x) = split_name(name)?;
        self.remove(user, name)?;
        let bs = self.p.block_size;
        let nblocks = data.len().div_ceil(bs);
        let cap = self.p.entry_capacity();
        let nentries = data.len().div_ceil(cap).max(1);
        let used = self.used_blocks();
        let free: Vec<usize> = (0..self.p.blocks).filter(|&b| !used[b]).collect();
        if free.len() < nblocks {
            return Err(format!(
                "{name}: disk full ({} KB needed, {} KB free)",
                nblocks * bs / 1024,
                free.len() * bs / 1024
            ));
        }
        let slots: Vec<usize> = (0..self.p.dir_entries)
            .filter(|&i| self.entry(i)[0] == 0xE5 || self.entry(i)[0] > 31)
            .take(nentries)
            .collect();
        if slots.len() < nentries {
            return Err(format!("{name}: directory full"));
        }
        let mut next_free = free.into_iter();
        for (k, &slot) in slots.iter().enumerate() {
            let start = k * cap;
            let len = data.len().saturating_sub(start).min(cap);
            let records = len.div_ceil(128);
            let mut e = [0u8; 32];
            e[0] = user;
            e[1..9].copy_from_slice(&n);
            e[9..12].copy_from_slice(&x);
            // Last logical extent covered by this entry and records in it.
            let (ext_no, rc) = if records == 0 {
                (k * (self.p.exm as usize + 1), 0)
            } else {
                let base = k * (self.p.exm as usize + 1);
                let full = (records - 1) / 128;
                (base + full, records - full * 128)
            };
            e[12] = (ext_no & 0x1f) as u8;
            e[14] = ((ext_no >> 5) & 0x3f) as u8;
            e[15] = rc as u8;
            let blocks_here = len.div_ceil(bs);
            for j in 0..blocks_here {
                let b = next_free.next().expect("checked free space");
                let o = b * bs;
                let src = &data[start + j * bs..(start + (j + 1) * bs).min(data.len())];
                self.img[o..o + src.len()].copy_from_slice(src);
                // Pad the last partial record with ^Z, rest of block with E5.
                let rec_end = (src.len().div_ceil(128) * 128).min(bs);
                for byte in &mut self.img[o + src.len()..o + rec_end] { *byte = 0x1A; }
                for byte in &mut self.img[o + rec_end..o + bs] { *byte = 0xE5; }
                if self.p.wide_pointers() {
                    e[16 + 2 * j..18 + 2 * j].copy_from_slice(&(b as u16).to_le_bytes());
                } else {
                    e[16 + j] = b as u8;
                }
            }
            self.entry_mut(slot).copy_from_slice(&e);
        }
        Ok(())
    }

    /// Free space in bytes.
    pub fn free_bytes(&self) -> usize {
        self.used_blocks().iter().filter(|u| !**u).count() * self.p.block_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{F200, F400, F800};

    fn roundtrip(f: Format) {
        let mut img = f.blank().data;
        let p = Params::for_format(f);
        let mut fs = CpmFs::new(&mut img, p).unwrap();
        let small: Vec<u8> = (0..300u32).map(|i| i as u8).collect();
        let big: Vec<u8> = (0..40000u32).map(|i| (i * 7) as u8).collect();
        fs.put(0, "small.txt", &small).unwrap();
        fs.put(3, "BIG.COM", &big).unwrap();
        let l = fs.list();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].name, "SMALL.TXT");
        assert_eq!(l[0].size, 384);
        assert_eq!(l[1].size, 40064);
        let s = fs.get(0, "SMALL.TXT").unwrap();
        assert_eq!(&s[..300], &small[..]);
        assert!(s[300..].iter().all(|&b| b == 0x1A));
        let b = fs.get(3, "big.com").unwrap();
        assert_eq!(&b[..40000], &big[..]);
        assert!(fs.remove(3, "BIG.COM").unwrap());
        assert_eq!(fs.list().len(), 1);
    }

    #[test]
    fn roundtrip_all_formats() {
        roundtrip(F200);
        roundtrip(F400);
        roundtrip(F800);
    }

    #[test]
    fn rejects_bad_names() {
        assert!(split_name("TOOLONGNAME.COM").is_err());
        assert!(split_name("A*.COM").is_err());
        assert!(split_name("OK.COM").is_ok());
    }
}
