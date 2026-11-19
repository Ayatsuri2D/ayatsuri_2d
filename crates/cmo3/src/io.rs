//! Big-endian, XOR-keyed binary reader and writer for the CAFF container.
//!
//! All multi-byte integers are big-endian. An obfuscation key XORs values by width:
//! byte = key & 0xFF, short = key & 0xFFFF, int = key, long = (key << 32) | key where the key is
//! SIGN-EXTENDED first (a negative key floods the high dword with ones; the official editor does
//! exactly this, and the official reader decodes garbage offsets otherwise). A key of 0 means "not
//! obfuscated".

use crate::{Result, bail, ensure};

/// Reader over a byte slice with an explicit cursor.
pub struct BinaryReader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> BinaryReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        BinaryReader { data, position: 0 }
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn seek(&mut self, offset: usize) {
        self.position = offset;
    }

    /// Advances past `count` bytes. A CAFF "skip(n)" is a literal reserved gap, not alignment.
    pub fn skip(&mut self, count: usize) {
        self.position += count;
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self.position.saturating_add(count);
        ensure!(
            end <= self.data.len(),
            "read past end: need {} bytes at offset {} of {}",
            count,
            self.position,
            self.data.len()
        );
        let out = &self.data[self.position..end];
        self.position = end;
        Ok(out)
    }

    pub fn read_u8(&mut self, key: i32) -> Result<u8> {
        let b = self.take(1)?[0];
        Ok(b ^ (key as u8))
    }

    pub fn read_i16(&mut self, key: i32) -> Result<i16> {
        let b = self.take(2)?;
        let combined = u16::from_be_bytes([b[0], b[1]]);
        Ok((combined ^ (key as u16)) as i16)
    }

    pub fn read_i32(&mut self, key: i32) -> Result<i32> {
        let b = self.take(4)?;
        let combined = i32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        Ok(combined ^ key)
    }

    pub fn read_i64(&mut self, key: i32) -> Result<i64> {
        let b = self.take(8)?;
        let combined = i64::from_be_bytes(b.try_into().unwrap());
        Ok(combined ^ long_mask(key))
    }

    pub fn read_bool(&mut self, key: i32) -> Result<bool> {
        Ok(self.read_u8(key)? != 0)
    }

    /// 7-bit, MSB-first variable-length unsigned integer (1 to 4 bytes). Length prefix of strings.
    pub fn read_var_int(&mut self, key: i32) -> Result<u32> {
        let b0 = self.read_u8(key)? as u32;
        if b0 & 0x80 == 0 {
            return Ok(b0);
        }
        let b1 = self.read_u8(key)? as u32;
        if b1 & 0x80 == 0 {
            return Ok(((b0 & 0x7F) << 7) | (b1 & 0x7F));
        }
        let b2 = self.read_u8(key)? as u32;
        if b2 & 0x80 == 0 {
            return Ok(((b0 & 0x7F) << 14) | ((b1 & 0x7F) << 7) | b2);
        }
        let b3 = self.read_u8(key)? as u32;
        if b3 & 0x80 == 0 {
            return Ok(((b0 & 0x7F) << 21) | ((b1 & 0x7F) << 14) | ((b2 & 0x7F) << 7) | b3);
        }
        bail!("varint exceeds 4 bytes at offset {}", self.position - 4)
    }

    /// A var-int byte length followed by that many XOR'd UTF-8 bytes.
    pub fn read_string(&mut self, key: i32) -> Result<String> {
        let len = self.read_var_int(key)? as usize;
        let raw = self.read_bytes(len, key)?;
        Ok(String::from_utf8(raw)?)
    }

    /// `count` raw bytes, each XOR'd with key & 0xFF.
    pub fn read_bytes(&mut self, count: usize, key: i32) -> Result<Vec<u8>> {
        let src = self.take(count)?;
        Ok(xor_bytes(src, key))
    }
}

/// Growable big-endian writer, inverse of [`BinaryReader`].
pub struct BinaryWriter {
    buffer: Vec<u8>,
}

impl Default for BinaryWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl BinaryWriter {
    pub fn new() -> Self {
        BinaryWriter { buffer: Vec::with_capacity(64 * 1024) }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        BinaryWriter { buffer: Vec::with_capacity(capacity) }
    }

    pub fn position(&self) -> usize {
        self.buffer.len()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }

    /// Writes `count` zero bytes (a CAFF reserved gap).
    pub fn skip(&mut self, count: usize) {
        self.buffer.resize(self.buffer.len() + count, 0);
    }

    /// Overwrites an already-written big-endian i32 (caller XORs beforehand if needed).
    pub fn patch_i32(&mut self, offset: usize, value: i32) {
        self.buffer[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    pub fn write_u8(&mut self, value: u8, key: i32) {
        self.buffer.push(value ^ (key as u8));
    }

    pub fn write_i16(&mut self, value: i16, key: i32) {
        let encoded = (value as u16) ^ (key as u16);
        self.buffer.extend_from_slice(&encoded.to_be_bytes());
    }

    pub fn write_i32(&mut self, value: i32, key: i32) {
        self.buffer.extend_from_slice(&(value ^ key).to_be_bytes());
    }

    pub fn write_i64(&mut self, value: i64, key: i32) {
        self.buffer.extend_from_slice(&(value ^ long_mask(key)).to_be_bytes());
    }

    pub fn write_bool(&mut self, value: bool, key: i32) {
        self.write_u8(u8::from(value), key);
    }

    pub fn write_var_int(&mut self, value: u32, key: i32) -> Result<()> {
        match value {
            v if v < 0x80 => self.write_u8(v as u8, key),
            v if v < 0x4000 => {
                self.write_u8((((v >> 7) & 0x7F) | 0x80) as u8, key);
                self.write_u8((v & 0x7F) as u8, key);
            }
            v if v < 0x20_0000 => {
                self.write_u8((((v >> 14) & 0x7F) | 0x80) as u8, key);
                self.write_u8((((v >> 7) & 0x7F) | 0x80) as u8, key);
                self.write_u8((v & 0x7F) as u8, key);
            }
            v if v < 0x1000_0000 => {
                self.write_u8((((v >> 21) & 0x7F) | 0x80) as u8, key);
                self.write_u8((((v >> 14) & 0x7F) | 0x80) as u8, key);
                self.write_u8((((v >> 7) & 0x7F) | 0x80) as u8, key);
                self.write_u8((v & 0x7F) as u8, key);
            }
            v => bail!("varint too large: {v}"),
        }
        Ok(())
    }

    pub fn write_string(&mut self, value: &str, key: i32) -> Result<()> {
        let utf8 = value.as_bytes();
        self.write_var_int(utf8.len() as u32, key)?;
        self.write_bytes(utf8, key);
        Ok(())
    }

    pub fn write_bytes(&mut self, bytes: &[u8], key: i32) {
        if key == 0 {
            self.buffer.extend_from_slice(bytes);
        } else {
            let k = key as u8;
            self.buffer.extend(bytes.iter().map(|b| b ^ k));
        }
    }
}

/// Number of bytes `value` takes as a CAFF var-int.
pub fn var_int_size(value: u32) -> Result<usize> {
    Ok(match value {
        v if v < 0x80 => 1,
        v if v < 0x4000 => 2,
        v if v < 0x20_0000 => 3,
        v if v < 0x1000_0000 => 4,
        v => bail!("varint too large: {v}"),
    })
}

fn long_mask(key: i32) -> i64 {
    let k = key as i64; // sign-extended on purpose, see module docs
    (k << 32) | k
}

fn xor_bytes(src: &[u8], key: i32) -> Vec<u8> {
    if key == 0 {
        src.to_vec()
    } else {
        let k = key as u8;
        src.iter().map(|b| b ^ k).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_round_trip() {
        for v in [0u32, 1, 0x7F, 0x80, 0x3FFF, 0x4000, 0x1F_FFFF, 0x20_0000, 0x0FFF_FFFF] {
            let mut w = BinaryWriter::new();
            w.write_var_int(v, 0x5A5A5A5A).unwrap();
            let bytes = w.into_bytes();
            assert_eq!(bytes.len(), var_int_size(v).unwrap());
            let mut r = BinaryReader::new(&bytes);
            assert_eq!(r.read_var_int(0x5A5A5A5A).unwrap(), v);
        }
    }

    #[test]
    fn negative_key_long_mask_floods_high_dword() {
        // The editor's 64-bit mask ORs the sign-extended key.
        let mut w = BinaryWriter::new();
        w.write_i64(0x1234, -2);
        let bytes = w.into_bytes();
        assert_eq!(&bytes[0..4], &[0xFF, 0xFF, 0xFF, 0xFF]);
        let mut r = BinaryReader::new(&bytes);
        assert_eq!(r.read_i64(-2).unwrap(), 0x1234);
    }

    #[test]
    fn string_round_trip() {
        let mut w = BinaryWriter::new();
        w.write_string("imageFileBuf_12.png", 77).unwrap();
        let bytes = w.into_bytes();
        let mut r = BinaryReader::new(&bytes);
        assert_eq!(r.read_string(77).unwrap(), "imageFileBuf_12.png");
    }
}
