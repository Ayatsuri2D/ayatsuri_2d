// Writing the CAFF container through the `cmo3` crate: the PNG entries
// raw, `main.xml` as the `main_xml` entry deflated the way the Editor
// writes it, every entry obfuscated with the key.

use crate::ImportError;

/// One file to store: its name, its bytes and whether it is the project
/// XML (which gets the `main_xml` kind and the zip compression code).
pub struct File {
    pub name: String,
    pub bytes: Vec<u8>,
    pub main: bool,
}

pub fn write(files: &[File], key: u32) -> Result<Vec<u8>, ImportError> {
    use cmo3::caff::{CaffArchive, CaffEntry, CaffPreview, CompressOption, TAG_MAIN_XML};
    let entries = files
        .iter()
        .map(|f| {
            let (tag, compression) = if f.main {
                (TAG_MAIN_XML, CompressOption::Fast)
            } else {
                ("", CompressOption::Raw)
            };
            CaffEntry::new(f.name.clone(), tag, f.bytes.clone()).with_compression(compression)
        })
        .collect();
    let archive = CaffArchive {
        format_identifier: "----".into(),
        format_version: [0, 0, 0],
        archive_version: [0, 0, 0],
        obfuscate_key: key as i32,
        preview: CaffPreview::none(),
        entries,
    };
    cmo3::caff::write(&archive).map_err(|e| ImportError::Export(format!("cmo3: {e}")))
}
