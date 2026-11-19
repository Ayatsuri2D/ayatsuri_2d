//! The high-level `.cmo3` API: container plus model graph, with the pixel-entry helpers.

use std::collections::HashSet;
use std::path::Path;

use crate::caff::{self, CaffArchive, CaffEntry, CompressOption, TAG_MAIN_XML};
use crate::model::{self, ModelGraph, ObjRef, Value};
use crate::version::Cmo3Version;
use crate::xml;
use crate::{Result, bail, ensure};

/// A loaded CMO3: the CAFF container (all entries, including layer/atlas/icon PNGs) and the model
/// graph (the `CModelSource` root plus the shared pool). Mutate either, then [`Cmo3Model::write`].
#[derive(Debug, Clone)]
pub struct Cmo3Model {
    pub archive: CaffArchive,
    pub graph: ModelGraph,
}

impl Cmo3Model {
    /// Parses a `.cmo3` from raw bytes.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        Self::read_archive(caff::read(bytes)?)
    }

    /// Parses a `.cmo3` whose container is already decoded.
    pub fn read_archive(archive: CaffArchive) -> Result<Self> {
        let main_xml = match archive.first_by_tag(TAG_MAIN_XML) {
            Some(e) => &e.content,
            None => bail!("not a CMO3: no main_xml entry"),
        };
        let doc = xml::parse(main_xml)?;
        let graph = model::read_model(&doc)?;
        Ok(Cmo3Model { archive, graph })
    }

    pub fn read_file(path: impl AsRef<Path>) -> Result<Self> {
        Self::read(&std::fs::read(path)?)
    }

    /// True if the bytes start with the CAFF magic.
    pub fn matches(bytes: &[u8]) -> bool {
        caff::is_caff(bytes)
    }

    /// Cheap version probe: decodes only `main_xml` and scans for `fileFormatVersion="..."`.
    /// Never fails: a truncated or corrupt container reports None.
    pub fn get_version(bytes: &[u8]) -> Option<Cmo3Version> {
        if !caff::is_caff(bytes) {
            return None;
        }
        let entry = caff::read_first_entry_by_tag(bytes, TAG_MAIN_XML).ok()??;
        let raw = extract_file_format_version(&entry.content)?;
        Cmo3Version::from_file_format_version(&raw)
    }

    /// The decoded `main_xml` bytes as stored in the archive (not the re-emitted graph).
    pub fn stored_main_xml(&self) -> Option<&[u8]> {
        self.archive.first_by_tag(TAG_MAIN_XML).map(|e| e.content.as_slice())
    }

    /// Re-emits the graph as main.xml bytes, completing the prologue for classes added after read.
    pub fn render_main_xml(&self) -> Result<Vec<u8>> {
        let mut doc = model::write_model(&self.graph);
        crate::author::complete_prologue(&mut doc)?;
        Ok(xml::write(&doc))
    }

    /// Serializes back to `.cmo3` bytes. The decompressed main.xml of an unedited model
    /// round-trips byte for byte; the whole file does not, since compressed entries carry the
    /// write's own timestamp and a different DEFLATE implementation.
    pub fn write(&self) -> Result<Vec<u8>> {
        let main_xml = self.render_main_xml()?;
        let mut archive = self.archive.clone();
        let mut found = false;
        for entry in archive.entries.iter_mut() {
            if entry.tag == TAG_MAIN_XML {
                entry.content = main_xml.clone();
                found = true;
            }
        }
        ensure!(found, "archive lost its main_xml entry");
        caff::write(&archive)
    }

    pub fn write_file(&self, path: impl AsRef<Path>) -> Result<()> {
        std::fs::write(path, self.write()?)?;
        Ok(())
    }

    /// The model root (normally a `CModelSource` object).
    pub fn root(&self) -> Option<ObjRef> {
        self.graph.root_object()
    }

    /// The `<root fileFormatVersion>` attribute.
    pub fn file_format_version(&self) -> Option<&str> {
        self.graph.root_attr("fileFormatVersion")
    }

    /// `CModelSource.targetVersionNo`, or None when the document carries none.
    pub fn target_version_no(&self) -> Option<i32> {
        let root = self.root()?;
        match root.borrow().field("targetVersionNo") {
            Some(Value::Int(v)) => Some(*v),
            _ => None,
        }
    }

    /// Sets `targetVersionNo`, inserting the field where the editor writes it (before
    /// `latestVersionOfLastModelerNo`) when the source carried none.
    pub fn set_target_version_no(&mut self, version_no: i32) -> Result<()> {
        let root = match self.root() {
            Some(r) => r,
            None => bail!("model root is not an object"),
        };
        ensure!(root.borrow().tag == "CModelSource", "model root is not a CModelSource");
        root.borrow_mut().set_field_before(
            "targetVersionNo",
            Value::Int(version_no),
            "latestVersionOfLastModelerNo",
        );
        Ok(())
    }

    /// Every reachable object with `tag`, each listed once, in traversal order.
    pub fn objects_with_tag(&self, tag: &str) -> Vec<ObjRef> {
        let mut seen: HashSet<*const ()> = HashSet::new();
        let mut out = Vec::new();
        let mut stack: Vec<Value> = Vec::new();
        if let Some(r) = &self.graph.root {
            stack.push(r.clone());
        }
        for entry in &self.graph.shared {
            stack.push(entry.value.clone());
        }
        while let Some(v) = stack.pop() {
            if let Some(ptr) = v.identity()
                && !seen.insert(ptr)
            {
                continue;
            }
            match &v {
                Value::Object(o) => {
                    let obj = o.borrow();
                    if obj.tag == tag {
                        out.push(o.clone());
                    }
                    for f in obj.fields.iter().rev() {
                        stack.push(f.value.clone());
                    }
                }
                Value::Shared(s) => stack.push(s.borrow().clone()),
                Value::List { items, .. } | Value::Set(items) => {
                    stack.extend(items.iter().rev().cloned())
                }
                Value::Map { entries, .. } => {
                    for (k, val) in entries.iter().rev() {
                        stack.push(val.clone());
                        stack.push(k.clone());
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// All `CImageResource`s reachable in the model, each linking an embedded PNG.
    pub fn image_resources(&self) -> Vec<ObjRef> {
        self.objects_with_tag("CImageResource")
    }

    /// The archive path a `CImageResource`'s `imageFileBuf` names.
    pub fn image_file_buf_path(resource: &ObjRef) -> Option<String> {
        match resource.borrow().field("imageFileBuf") {
            Some(Value::File { path: Some(p), .. }) => Some(p.clone()),
            _ => None,
        }
    }

    /// The decoded PNG bytes backing `resource`, or None if it has no embedded file.
    pub fn extract_layer_png(&self, resource: &ObjRef) -> Option<&[u8]> {
        let path = Self::image_file_buf_path(resource)?;
        self.archive.by_path(&path).map(|e| e.content.as_slice())
    }

    /// Replaces `resource`'s embedded PNG and keeps its `imageFileBuf_size` attribute consistent.
    pub fn replace_layer_png(&mut self, resource: &ObjRef, png: Vec<u8>) -> Result<()> {
        let path = match Self::image_file_buf_path(resource) {
            Some(p) => p,
            None => bail!("resource has no embedded imageFileBuf"),
        };
        let size = png.len();
        self.replace_png(&path, png)?;
        resource.borrow_mut().set_attr("imageFileBuf_size", size.to_string());
        Ok(())
    }

    /// Replaces the embedded PNG at `path`, keeping the entry's place, tag and options.
    pub fn replace_png(&mut self, path: &str, png: Vec<u8>) -> Result<()> {
        match self.archive.by_path_mut(path) {
            Some(entry) => {
                entry.content = png;
                Ok(())
            }
            None => bail!("archive holds no entry at '{path}'"),
        }
    }

    /// Embeds a NEW PNG for `resource`, whose `imageFileBuf` must already name an unused path
    /// (mint one with [`Self::next_image_file_buf_path`]).
    pub fn add_layer_png(&mut self, resource: &ObjRef, png: Vec<u8>) -> Result<()> {
        let path = match Self::image_file_buf_path(resource) {
            Some(p) => p,
            None => bail!("resource has no imageFileBuf path to add under"),
        };
        let size = png.len();
        self.add_png(&path, png)?;
        resource.borrow_mut().set_attr("imageFileBuf_size", size.to_string());
        Ok(())
    }

    /// Embeds a NEW PNG at `path` with the corpus conventions for pixel data: empty tag, stored
    /// raw, obfuscated, inserted before the `main_xml` entry (which every corpus file keeps last).
    pub fn add_png(&mut self, path: &str, png: Vec<u8>) -> Result<()> {
        ensure!(self.archive.by_path(path).is_none(), "archive already holds an entry at '{path}'");
        let insert_at = self
            .archive
            .entries
            .iter()
            .position(|e| e.tag == TAG_MAIN_XML)
            .unwrap_or(self.archive.entries.len());
        self.archive.entries.insert(
            insert_at,
            CaffEntry {
                path: path.to_string(),
                tag: String::new(),
                content: png,
                compression: CompressOption::Raw,
                obfuscated: true,
            },
        );
        Ok(())
    }

    /// Removes `resource`'s embedded PNG from the archive. Unlinking the graph node is the
    /// caller's job: a live resource pointing at a missing entry is a load-time hazard.
    pub fn remove_layer_png(&mut self, resource: &ObjRef) -> Result<()> {
        let path = match Self::image_file_buf_path(resource) {
            Some(p) => p,
            None => bail!("resource has no embedded imageFileBuf"),
        };
        self.archive.entries.retain(|e| e.path != path);
        Ok(())
    }

    /// Next unused `imageFileBuf` path: `imageFileBuf.png`, then `imageFileBuf_0.png` onward.
    pub fn next_image_file_buf_path(&self) -> String {
        self.next_path_in_sequence("imageFileBuf")
    }

    /// Next unused icon path: `image.png`, then `image_0.png` onward.
    pub fn next_image_path(&self) -> String {
        self.next_path_in_sequence("image")
    }

    fn next_path_in_sequence(&self, stem: &str) -> String {
        let mut highest: i64 = -2;
        let plain = format!("{stem}.png");
        for entry in &self.archive.entries {
            if entry.path == plain {
                highest = highest.max(-1);
            } else if let Some(mid) =
                entry.path.strip_prefix(&format!("{stem}_")).and_then(|r| r.strip_suffix(".png"))
                && let Ok(n) = mid.parse::<i64>()
                && format!("{stem}_{mid}.png") == entry.path
            {
                highest = highest.max(n);
            }
        }
        let next = highest + 1;
        if next == -1 { plain } else { format!("{stem}_{next}.png") }
    }
}

/// Scans main.xml bytes for the ASCII `fileFormatVersion="..."` attribute without parsing.
fn extract_file_format_version(main_xml: &[u8]) -> Option<String> {
    let needle = b"fileFormatVersion=\"";
    let start = main_xml.windows(needle.len()).position(|w| w == needle)? + needle.len();
    let end = start + main_xml[start..].iter().position(|b| *b == b'"')?;
    Some(String::from_utf8_lossy(&main_xml[start..end]).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caff::CaffArchive;

    fn tiny_model() -> Cmo3Model {
        let main = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<?version SerializeFormatVersion:2?>\r\n<?version CModelSource:16?>\r\n<?import com.live2d.cubism.doc.model.CModelSource?>\r\n<?import com.live2d.graphics.CImageResource?>\r\n<root fileFormatVersion=\"504000000\">\r\n<shared />\r\n<main>\r\n<CModelSource isDefaultKeyformLocked=\"false\">\r\n<i xs.n=\"latestVersionOfLastModelerNo\">5</i>\r\n<CImageResource xs.n=\"res\" width=\"2\" height=\"2\" type=\"png\" imageFileBuf_size=\"3\">\r\n<file xs.n=\"imageFileBuf\" path=\"imageFileBuf.png\" />\r\n</CImageResource>\r\n</CModelSource>\r\n</main>\r\n</root>\r\n\r\n".to_vec();
        let archive = CaffArchive {
            obfuscate_key: 42,
            entries: vec![
                CaffEntry::new("imageFileBuf.png", "", vec![1, 2, 3]),
                CaffEntry::new("main.xml", TAG_MAIN_XML, main)
                    .with_compression(CompressOption::Small),
            ],
            ..CaffArchive::default()
        };
        Cmo3Model::read_archive(archive).unwrap()
    }

    #[test]
    fn full_round_trip_keeps_main_xml_identical() {
        let model = tiny_model();
        let bytes = model.write().unwrap();
        let back = Cmo3Model::read(&bytes).unwrap();
        assert_eq!(back.stored_main_xml(), model.stored_main_xml());
        assert_eq!(Cmo3Model::get_version(&bytes), Some(Cmo3Version::V5));
    }

    #[test]
    fn set_target_version_inserts_before_anchor() {
        let mut model = tiny_model();
        assert_eq!(model.target_version_no(), None);
        model.set_target_version_no(5_030_000).unwrap();
        let xml = String::from_utf8(model.render_main_xml().unwrap()).unwrap();
        assert!(xml.contains("<i xs.n=\"targetVersionNo\">5030000</i>\r\n<i xs.n=\"latestVersionOfLastModelerNo\">5</i>"), "{xml}");
    }

    #[test]
    fn layer_png_helpers() {
        let mut model = tiny_model();
        let resources = model.image_resources();
        assert_eq!(resources.len(), 1);
        assert_eq!(model.extract_layer_png(&resources[0]), Some(&[1u8, 2, 3][..]));
        model.replace_layer_png(&resources[0], vec![9; 10]).unwrap();
        assert_eq!(resources[0].borrow().attr("imageFileBuf_size"), Some("10"));
        assert_eq!(model.next_image_file_buf_path(), "imageFileBuf_0.png");
        model.add_png("imageFileBuf_0.png", vec![7]).unwrap();
        assert_eq!(model.archive.entries[1].path, "imageFileBuf_0.png");
        assert_eq!(model.archive.entries[2].tag, TAG_MAIN_XML);
        assert_eq!(model.next_image_file_buf_path(), "imageFileBuf_1.png");
    }
}
