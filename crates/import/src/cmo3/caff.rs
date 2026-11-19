// The CAFF container a Cubism Editor project (.cmo3, .can3) is stored in,
// read by the `cmo3` crate. This keeps the small view the importer uses:
// an entry's bytes by name, and `main.xml` unpacked.

use crate::ImportError;

fn err(msg: impl Into<String>) -> ImportError {
    ImportError::Export(format!("cmo3: {}", msg.into()))
}

pub struct Caff {
    archive: cmo3::caff::CaffArchive,
}

impl Caff {
    pub fn open(bytes: &[u8]) -> Result<Self, ImportError> {
        let archive = cmo3::caff::read(bytes).map_err(|e| err(e.to_string()))?;
        Ok(Caff { archive })
    }

    /// The entry names, in table order.
    #[cfg(test)]
    pub fn names(&self) -> Vec<&str> {
        self.archive.entries.iter().map(|e| e.path.as_str()).collect()
    }

    /// An entry's bytes, de-obfuscated and unpacked.
    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.archive.by_path(name).map(|e| e.content.clone())
    }

    /// `main.xml`, unpacked from its zip.
    pub fn main_xml(&self) -> Result<Vec<u8>, ImportError> {
        self.archive
            .first_by_tag(cmo3::caff::TAG_MAIN_XML)
            .map(|e| e.content.clone())
            .ok_or_else(|| err("no main.xml"))
    }
}
