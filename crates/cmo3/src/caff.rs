//! The CAFF container.
//!
//! Layout: header (26 bytes, cleartext) -> preview block (28 bytes, cleartext) -> file table
//! (XOR-obfuscated with the header key) -> concatenated blobs -> guard bytes `b` `c`.
//!
//! A FAST/SMALL entry is stored as a PARTIAL single-entry zip stream named `contents`: local file
//! header, raw DEFLATE payload, data descriptor, and nothing else (no central directory, no EOCD).

use std::io::Read;

use crate::io::{BinaryReader, BinaryWriter, var_int_size};
use crate::{Result, bail, ensure};

/// Preview image encoding byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    NotInit,
    Png,
    NoPreview,
}

impl ImageType {
    pub fn type_no(self) -> u8 {
        match self {
            ImageType::NotInit => 0,
            ImageType::Png => 1,
            ImageType::NoPreview => 127,
        }
    }

    pub fn from_type_no(n: u8) -> Result<Self> {
        Ok(match n {
            0 => ImageType::NotInit,
            1 => ImageType::Png,
            127 => ImageType::NoPreview,
            other => bail!("unknown ImageType {other}"),
        })
    }
}

/// Preview pixel layout byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorType {
    NotInit,
    Argb,
    Rgb,
    NoPreview,
}

impl ColorType {
    pub fn type_no(self) -> u8 {
        match self {
            ColorType::NotInit => 0,
            ColorType::Argb => 1,
            ColorType::Rgb => 2,
            ColorType::NoPreview => 127,
        }
    }

    pub fn from_type_no(n: u8) -> Result<Self> {
        Ok(match n {
            0 => ColorType::NotInit,
            1 => ColorType::Argb,
            2 => ColorType::Rgb,
            127 => ColorType::NoPreview,
            other => bail!("unknown ColorType {other}"),
        })
    }
}

/// Per-entry storage mode. RAW = stored as-is (PNGs); FAST/SMALL = single-entry zip stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressOption {
    Raw,
    Fast,
    Small,
}

impl CompressOption {
    /// The on-disk byte.
    pub fn serialize_no(self) -> u8 {
        match self {
            CompressOption::Raw => 16,
            CompressOption::Fast => 33,
            CompressOption::Small => 37,
        }
    }

    /// The java.util.zip level used when (re)compressing.
    pub fn zip_level(self) -> u32 {
        match self {
            CompressOption::Raw => 0,
            CompressOption::Fast => 1,
            CompressOption::Small => 5,
        }
    }

    pub fn is_compressed(self) -> bool {
        self != CompressOption::Raw
    }

    pub fn from_serialize_no(n: u8) -> Result<Self> {
        Ok(match n {
            16 => CompressOption::Raw,
            33 => CompressOption::Fast,
            37 => CompressOption::Small,
            other => bail!("unknown CompressOption {other}"),
        })
    }
}

/// One logical file inside the archive. `content` is fully decoded (de-obfuscated, decompressed).
#[derive(Debug, Clone)]
pub struct CaffEntry {
    pub path: String,
    pub tag: String,
    pub content: Vec<u8>,
    pub compression: CompressOption,
    pub obfuscated: bool,
}

impl CaffEntry {
    pub fn new(path: impl Into<String>, tag: impl Into<String>, content: Vec<u8>) -> Self {
        CaffEntry {
            path: path.into(),
            tag: tag.into(),
            content,
            compression: CompressOption::Raw,
            obfuscated: true,
        }
    }

    pub fn with_compression(mut self, compression: CompressOption) -> Self {
        self.compression = compression;
        self
    }
}

/// The CAFF preview thumbnail. Real cmo3 files carry NO_PREVIEW.
#[derive(Debug, Clone)]
pub struct CaffPreview {
    pub image_type: ImageType,
    pub color_type: ColorType,
    pub width: i16,
    pub height: i16,
    /// PNG bytes, or None when `image_type` is not PNG.
    pub png: Option<Vec<u8>>,
}

impl CaffPreview {
    pub fn none() -> Self {
        CaffPreview {
            image_type: ImageType::NoPreview,
            color_type: ColorType::NoPreview,
            width: 0,
            height: 0,
            png: None,
        }
    }

    pub fn present(&self) -> bool {
        self.image_type == ImageType::Png && self.png.is_some()
    }
}

/// An in-memory CAFF archive. Entry order is significant: consumers map paths positionally.
#[derive(Debug, Clone)]
pub struct CaffArchive {
    /// 4-char format identifier; "----" for cmo3.
    pub format_identifier: String,
    pub format_version: [u8; 3],
    pub archive_version: [u8; 3],
    /// XOR key; 0 means not obfuscated.
    pub obfuscate_key: i32,
    pub preview: CaffPreview,
    pub entries: Vec<CaffEntry>,
}

impl Default for CaffArchive {
    fn default() -> Self {
        CaffArchive {
            format_identifier: DEFAULT_FORMAT_IDENTIFIER.to_string(),
            format_version: [0, 0, 0],
            archive_version: [0, 0, 0],
            obfuscate_key: 0,
            preview: CaffPreview::none(),
            entries: Vec::new(),
        }
    }
}

pub const MAGIC: &[u8; 4] = b"CAFF";
pub const DEFAULT_FORMAT_IDENTIFIER: &str = "----";
/// Archive tag of the serialized model document (main.xml).
pub const TAG_MAIN_XML: &str = "main_xml";
/// Archive tag of the preview image entry.
pub const TAG_PREVIEW: &str = "preview";

impl CaffArchive {
    pub fn first_by_tag(&self, tag: &str) -> Option<&CaffEntry> {
        self.entries.iter().find(|e| e.tag == tag)
    }

    pub fn all_by_tag<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a CaffEntry> + 'a {
        self.entries.iter().filter(move |e| e.tag == tag)
    }

    pub fn by_path(&self, path: &str) -> Option<&CaffEntry> {
        self.entries.iter().find(|e| e.path == path)
    }

    pub fn by_path_mut(&mut self, path: &str) -> Option<&mut CaffEntry> {
        self.entries.iter_mut().find(|e| e.path == path)
    }
}

// ---------------------------------------------------------------------------------------------
// Single-entry zip framing
// ---------------------------------------------------------------------------------------------

const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4B50; // PK\x03\x04
const DATA_DESCRIPTOR_SIGNATURE: u32 = 0x0807_4B50; // PK\x07\x08
const LOCAL_HEADER_SIZE: usize = 30;
const DATA_DESCRIPTOR_SIZE: usize = 16;
const VERSION_DEFLATED: u16 = 20;
const METHOD_DEFLATED: u16 = 8;
const FLAG_DATA_DESCRIPTOR: u16 = 0x0008;
const FLAG_UTF8_NAME: u16 = 0x0800;
const ENTRY_NAME: &[u8] = b"contents";

/// The zip framing CAFF wraps FAST/SMALL blobs in.
pub mod zip {
    use super::*;

    fn le_u16(b: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([b[at], b[at + 1]])
    }

    fn le_u32(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    /// Inflates a single-entry CAFF zip stream back to the raw payload. The local header's sizes
    /// are ignored (the editor zeroes them, flag bit 3); the trailing data descriptor's
    /// uncompressed size caps the output when it is usable.
    pub fn unzip_single(zip_stream: &[u8]) -> Result<Vec<u8>> {
        ensure!(zip_stream.len() >= LOCAL_HEADER_SIZE, "Empty CAFF zip stream");
        ensure!(
            le_u32(zip_stream, 0) == LOCAL_HEADER_SIGNATURE,
            "CAFF blob is not a zip local file header"
        );
        let name_len = le_u16(zip_stream, 26) as usize;
        let extra_len = le_u16(zip_stream, 28) as usize;
        let payload_start = LOCAL_HEADER_SIZE + name_len + extra_len;
        ensure!(payload_start <= zip_stream.len(), "Truncated CAFF zip local header");

        let cap = declared_size(zip_stream);
        let mut out = Vec::new();
        flate2::read::DeflateDecoder::new(&zip_stream[payload_start..])
            .take(cap)
            .read_to_end(&mut out)
            .map_err(|e| crate::Error::with_source("inflate failed", e))?;
        Ok(out)
    }

    /// The uncompressed size the trailing data descriptor declares, else u64::MAX. A lying
    /// descriptor truncates only its own entry; anything unrecognizable falls back to unbounded.
    fn declared_size(blob: &[u8]) -> u64 {
        if blob.len() < LOCAL_HEADER_SIZE + DATA_DESCRIPTOR_SIZE {
            return u64::MAX;
        }
        let start = blob.len() - DATA_DESCRIPTOR_SIZE;
        if le_u32(blob, start) != DATA_DESCRIPTOR_SIGNATURE {
            return u64::MAX;
        }
        let declared = le_u32(blob, start + 12);
        if declared >= 1 { declared as u64 } else { u64::MAX }
    }

    /// Wraps `contents` in a single-entry ("contents") zip stream at the given DEFLATE level.
    /// `dos_date_time` is the MS-DOS stamp (date high 16 bits, time low 16), local wall clock.
    pub fn zip_single(contents: &[u8], level: u32, dos_date_time: u32) -> Result<Vec<u8>> {
        let deflated = deflate_raw(contents, level)?;
        let checksum = crc32fast::hash(contents);
        let mut out = Vec::with_capacity(
            LOCAL_HEADER_SIZE + ENTRY_NAME.len() + deflated.len() + DATA_DESCRIPTOR_SIZE,
        );

        // Local file header (APPNOTE.TXT 4.3.7).
        out.extend_from_slice(&LOCAL_HEADER_SIGNATURE.to_le_bytes());
        out.extend_from_slice(&VERSION_DEFLATED.to_le_bytes());
        out.extend_from_slice(&(FLAG_DATA_DESCRIPTOR | FLAG_UTF8_NAME).to_le_bytes());
        out.extend_from_slice(&METHOD_DEFLATED.to_le_bytes());
        out.extend_from_slice(&((dos_date_time & 0xFFFF) as u16).to_le_bytes());
        out.extend_from_slice(&((dos_date_time >> 16) as u16).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // crc: in the descriptor
        out.extend_from_slice(&0u32.to_le_bytes()); // compressed size: in the descriptor
        out.extend_from_slice(&0u32.to_le_bytes()); // uncompressed size: in the descriptor
        out.extend_from_slice(&(ENTRY_NAME.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(ENTRY_NAME);

        out.extend_from_slice(&deflated);

        // Data descriptor (APPNOTE.TXT 4.3.9). No central directory, no EOCD: that is what a CAFF
        // blob measurably is, and emitting them would grow the file by 76 bytes on every save.
        out.extend_from_slice(&DATA_DESCRIPTOR_SIGNATURE.to_le_bytes());
        out.extend_from_slice(&checksum.to_le_bytes());
        out.extend_from_slice(&(deflated.len() as u32).to_le_bytes());
        out.extend_from_slice(&(contents.len() as u32).to_le_bytes());
        Ok(out)
    }

    fn deflate_raw(data: &[u8], level: u32) -> Result<Vec<u8>> {
        use std::io::Write;
        let mut enc =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(level));
        enc.write_all(data)?;
        Ok(enc.finish()?)
    }

    /// Packs a civil local date/time into the MS-DOS stamp (date high 16, time low 16).
    pub fn dos_date_time_of(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> u32 {
        let y = (year - 1980).clamp(0, 127) as u32;
        let date = (y << 9) | (month << 5) | day;
        let time = (hour << 11) | (minute << 5) | (second / 2);
        (date << 16) | time
    }

    /// The local wall clock now as an MS-DOS stamp (the editor writes local time).
    pub fn current_dos_date_time() -> u32 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as libc::time_t)
            .unwrap_or(0);
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: localtime_r writes into the tm we own; both pointers are valid for the call.
        let ok = unsafe { !libc::localtime_r(&now, &mut tm).is_null() };
        if !ok {
            return dos_date_time_of(1980, 1, 1, 0, 0, 0);
        }
        dos_date_time_of(
            tm.tm_year + 1900,
            (tm.tm_mon + 1) as u32,
            tm.tm_mday as u32,
            tm.tm_hour as u32,
            tm.tm_min as u32,
            tm.tm_sec as u32,
        )
    }
}

// ---------------------------------------------------------------------------------------------
// Container codec
// ---------------------------------------------------------------------------------------------

const HEADER_AND_PREVIEW_BYTES: usize = 54;
const GUARD_BYTES: [u8; 2] = [0x62, 0x63]; // 'b' 'c'

struct EntryMeta {
    path: String,
    tag: String,
    offset: usize,
    stored_size: usize,
    obfuscated: bool,
    compression: CompressOption,
}

struct HeaderAndTable {
    archive_version: [u8; 3],
    format_identifier: String,
    format_version: [u8; 3],
    obfuscate_key: i32,
    preview_image_type: ImageType,
    preview_color_type: ColorType,
    preview_width: i16,
    preview_height: i16,
    preview_start_pos: usize,
    preview_file_size: i32,
    metas: Vec<EntryMeta>,
}

/// True if the bytes start with the cleartext `CAFF` magic.
pub fn is_caff(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[..4] == MAGIC
}

fn read_header_and_table(r: &mut BinaryReader<'_>) -> Result<HeaderAndTable> {
    // HEADER (not obfuscated)
    let magic = r.read_bytes(4, 0)?;
    ensure!(magic == MAGIC, "not a CAFF file (magic={:?})", String::from_utf8_lossy(&magic));
    let archive_version = [r.read_u8(0)?, r.read_u8(0)?, r.read_u8(0)?];
    let format_identifier = String::from_utf8_lossy(&r.read_bytes(4, 0)?).into_owned();
    let format_version = [r.read_u8(0)?, r.read_u8(0)?, r.read_u8(0)?];
    let obfuscate_key = r.read_i32(0)?;
    r.skip(8);

    // PREVIEW (not obfuscated)
    let preview_image_type = ImageType::from_type_no(r.read_u8(0)?)?;
    let preview_color_type = ColorType::from_type_no(r.read_u8(0)?)?;
    r.skip(2);
    let preview_width = r.read_i16(0)?;
    let preview_height = r.read_i16(0)?;
    let preview_start_pos = (r.read_i64(0)? & 0xFFFF_FFFF) as usize;
    let preview_file_size = r.read_i32(0)?;
    r.skip(8);

    // FILE TABLE (obfuscated)
    let count = r.read_i32(obfuscate_key)?;
    ensure!(count >= 0, "negative entry count {count}");
    // Each table row is at least 24 bytes, so a count the file cannot hold is corrupt (and would
    // otherwise size the Vec before any row is read).
    let max_count = (r.len().saturating_sub(HEADER_AND_PREVIEW_BYTES + 4)) / 24;
    ensure!(count as usize <= max_count, "entry count {count} exceeds what the file can hold");
    let mut metas = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let path = r.read_string(obfuscate_key)?;
        let tag = r.read_string(obfuscate_key)?;
        // The startPos high word is a writer artifact; the real offset is the low 32 bits.
        let start_pos = (r.read_i64(obfuscate_key)? & 0xFFFF_FFFF) as usize;
        let file_size = r.read_i32(obfuscate_key)?;
        ensure!(file_size >= 0, "negative entry size for '{path}'");
        let obfuscated = r.read_bool(obfuscate_key)?;
        let compression = CompressOption::from_serialize_no(r.read_u8(obfuscate_key)?)?;
        r.skip(8);
        metas.push(EntryMeta {
            path,
            tag,
            offset: start_pos,
            stored_size: file_size as usize,
            obfuscated,
            compression,
        });
    }

    Ok(HeaderAndTable {
        archive_version,
        format_identifier,
        format_version,
        obfuscate_key,
        preview_image_type,
        preview_color_type,
        preview_width,
        preview_height,
        preview_start_pos,
        preview_file_size,
        metas,
    })
}

fn decode_entry(r: &mut BinaryReader<'_>, meta: &EntryMeta, key: i32) -> Result<CaffEntry> {
    r.seek(meta.offset);
    let stored = r.read_bytes(meta.stored_size, if meta.obfuscated { key } else { 0 })?;
    let content =
        if meta.compression.is_compressed() { zip::unzip_single(&stored)? } else { stored };
    Ok(CaffEntry {
        path: meta.path.clone(),
        tag: meta.tag.clone(),
        content,
        compression: meta.compression,
        obfuscated: meta.obfuscated,
    })
}

/// Parses a CAFF archive, decoding every entry's payload.
pub fn read(bytes: &[u8]) -> Result<CaffArchive> {
    let mut r = BinaryReader::new(bytes);
    let parsed = read_header_and_table(&mut r)?;

    let mut entries = Vec::with_capacity(parsed.metas.len());
    for meta in &parsed.metas {
        entries.push(decode_entry(&mut r, meta, parsed.obfuscate_key)?);
    }

    let png = if parsed.preview_image_type == ImageType::Png && parsed.preview_file_size > 0 {
        r.seek(parsed.preview_start_pos);
        Some(r.read_bytes(parsed.preview_file_size as usize, 0)?)
    } else {
        None
    };

    Ok(CaffArchive {
        format_identifier: parsed.format_identifier,
        format_version: parsed.format_version,
        archive_version: parsed.archive_version,
        obfuscate_key: parsed.obfuscate_key,
        preview: CaffPreview {
            image_type: parsed.preview_image_type,
            color_type: parsed.preview_color_type,
            width: parsed.preview_width,
            height: parsed.preview_height,
            png,
        },
        entries,
    })
}

/// Decodes only the first entry carrying `tag` (a cheap probe that skips the layer PNGs).
pub fn read_first_entry_by_tag(bytes: &[u8], tag: &str) -> Result<Option<CaffEntry>> {
    let mut r = BinaryReader::new(bytes);
    let parsed = read_header_and_table(&mut r)?;
    match parsed.metas.iter().find(|m| m.tag == tag) {
        Some(meta) => Ok(Some(decode_entry(&mut r, meta, parsed.obfuscate_key)?)),
        None => Ok(None),
    }
}

/// Lists the file table without decoding any blob: (path, tag, offset, stored size, obfuscated,
/// compression).
pub type TableRow = (String, String, usize, usize, bool, CompressOption);

pub fn list_table(bytes: &[u8]) -> Result<Vec<TableRow>> {
    let mut r = BinaryReader::new(bytes);
    let parsed = read_header_and_table(&mut r)?;
    Ok(parsed
        .metas
        .into_iter()
        .map(|m| (m.path, m.tag, m.offset, m.stored_size, m.obfuscated, m.compression))
        .collect())
}

/// Serializes an archive to CAFF bytes with a fresh wall-clock stamp on compressed entries.
pub fn write(archive: &CaffArchive) -> Result<Vec<u8>> {
    write_with_stamp(archive, zip::current_dos_date_time())
}

/// Serializes an archive to CAFF bytes, stamping compressed entries with `dos_date_time`.
/// Offsets are computed in one pass and written as clean 64-bit values.
pub fn write_with_stamp(archive: &CaffArchive, dos_date_time: u32) -> Result<Vec<u8>> {
    let key = archive.obfuscate_key;

    let mut stored_blobs: Vec<Vec<u8>> = Vec::with_capacity(archive.entries.len());
    for entry in &archive.entries {
        stored_blobs.push(if entry.compression.is_compressed() {
            zip::zip_single(&entry.content, entry.compression.zip_level(), dos_date_time)?
        } else {
            entry.content.clone()
        });
    }

    let mut table_bytes = 4usize;
    for entry in &archive.entries {
        let p = entry.path.len();
        let t = entry.tag.len();
        table_bytes += var_int_size(p as u32)? + p + var_int_size(t as u32)? + t;
        table_bytes += 8 + 4 + 1 + 1 + 8;
    }

    let blob_start = HEADER_AND_PREVIEW_BYTES + table_bytes;
    let mut offsets = Vec::with_capacity(archive.entries.len());
    let mut cursor = blob_start;
    for blob in &stored_blobs {
        offsets.push(cursor);
        cursor += blob.len();
    }

    let mut w = BinaryWriter::with_capacity(cursor + 2);

    // HEADER
    w.write_bytes(MAGIC, 0);
    for b in archive.archive_version {
        w.write_u8(b, 0);
    }
    let ident = archive.format_identifier.as_bytes();
    ensure!(ident.len() == 4, "format identifier must be 4 bytes: '{}'", archive.format_identifier);
    w.write_bytes(ident, 0);
    for b in archive.format_version {
        w.write_u8(b, 0);
    }
    w.write_i32(key, 0);
    w.skip(8);

    // PREVIEW
    let preview_index = archive.entries.iter().position(|e| e.tag == TAG_PREVIEW);
    match preview_index {
        Some(i) if archive.preview.present() => {
            w.write_u8(ImageType::Png.type_no(), 0);
            w.write_u8(archive.preview.color_type.type_no(), 0);
            w.skip(2);
            w.write_i16(archive.preview.width, 0);
            w.write_i16(archive.preview.height, 0);
            w.write_i64(offsets[i] as i64, 0);
            w.write_i32(stored_blobs[i].len() as i32, 0);
            w.skip(8);
        }
        _ => {
            w.write_u8(ImageType::NoPreview.type_no(), 0);
            w.write_u8(ColorType::NoPreview.type_no(), 0);
            w.skip(2);
            w.write_i16(0, 0);
            w.write_i16(0, 0);
            w.write_i64(0, 0);
            w.write_i32(0, 0);
            w.skip(8);
        }
    }

    // FILE TABLE
    w.write_i32(archive.entries.len() as i32, key);
    for (i, entry) in archive.entries.iter().enumerate() {
        w.write_string(&entry.path, key)?;
        w.write_string(&entry.tag, key)?;
        w.write_i64(offsets[i] as i64, key);
        w.write_i32(stored_blobs[i].len() as i32, key);
        w.write_bool(entry.obfuscated, key);
        w.write_u8(entry.compression.serialize_no(), key);
        w.skip(8);
    }
    debug_assert_eq!(w.position(), blob_start);

    // BLOBS
    for (i, entry) in archive.entries.iter().enumerate() {
        w.write_bytes(&stored_blobs[i], if entry.obfuscated { key } else { 0 });
    }

    // GUARD BYTES (not obfuscated)
    w.write_u8(GUARD_BYTES[0], 0);
    w.write_u8(GUARD_BYTES[1], 0);

    Ok(w.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_single_round_trip() {
        let payload = b"<root><main/></root>".repeat(50);
        let z =
            zip::zip_single(&payload, 5, zip::dos_date_time_of(2026, 11, 18, 12, 30, 10)).unwrap();
        assert_eq!(&z[0..4], &LOCAL_HEADER_SIGNATURE.to_le_bytes());
        assert_eq!(&z[z.len() - 16..z.len() - 12], &DATA_DESCRIPTOR_SIGNATURE.to_le_bytes());
        assert_eq!(zip::unzip_single(&z).unwrap(), payload);
    }

    #[test]
    fn archive_round_trip_with_negative_key() {
        let archive = CaffArchive {
            obfuscate_key: -123456,
            entries: vec![
                CaffEntry::new("imageFileBuf.png", "", vec![1, 2, 3, 4, 5]),
                CaffEntry::new(
                    "main.xml",
                    TAG_MAIN_XML,
                    b"<root fileFormatVersion=\"504000000\"/>".to_vec(),
                )
                .with_compression(CompressOption::Small),
            ],
            ..CaffArchive::default()
        };
        let bytes = write_with_stamp(&archive, 0).unwrap();
        assert!(is_caff(&bytes));
        assert_eq!(&bytes[bytes.len() - 2..], &GUARD_BYTES);
        let back = read(&bytes).unwrap();
        assert_eq!(back.obfuscate_key, -123456);
        assert_eq!(back.entries.len(), 2);
        assert_eq!(back.entries[0].content, vec![1, 2, 3, 4, 5]);
        assert_eq!(back.entries[1].content, archive.entries[1].content);
        assert_eq!(back.entries[1].compression, CompressOption::Small);
        let probe = read_first_entry_by_tag(&bytes, TAG_MAIN_XML).unwrap().unwrap();
        assert_eq!(probe.path, "main.xml");
    }
}
