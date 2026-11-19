//! Authoring the main.xml prologue: the `fileFormatVersion` root
//! attribute, the `<?version?>` class-version PIs and the `<?import?>` tag-to-FQCN PIs that the
//! bare serializer does not produce. A read graph replays its recorded prologue verbatim; this is
//! for never-read graphs and for documents that gained classes after the read.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::model::{self, Value};
use crate::pi_tables;
use crate::xml::{self, Document, Element, Pi};
use crate::{Result, bail};

/// The fileFormatVersion a fresh document writes (official editor 5.4 "New -> Save As"). Pinned
/// to 5.4 because the prologue tables are transcribed from the 504000000 corpus.
pub const FRESH_FILE_FORMAT_VERSION: &str = "504000000";

fn collect_tags(el: &Element, into: &mut HashSet<String>) {
    into.insert(el.name.clone());
    for child in el.child_elements() {
        collect_tags(child, into);
    }
}

fn model_tags(root: &Element) -> BTreeSet<String> {
    let mut tags = HashSet::new();
    collect_tags(root, &mut tags);
    tags.into_iter().filter(|t| !pi_tables::is_structural_tag(t)).collect()
}

/// Serializes a fresh root as a complete main.xml with the 5.4-era prologue.
///
/// The `<?import?>` list is one FQCN per distinct non-structural element tag, sorted. A tag with
/// no table entry is a hard error: the official reader could not resolve it.
pub fn write_fresh_main_xml(root: &Value) -> Result<Vec<u8>> {
    let mut doc = model::write_root(root);
    let tags = model_tags(&doc.root);

    let mut imports: Vec<&str> = Vec::with_capacity(tags.len());
    for tag in &tags {
        match pi_tables::fqcn_for_tag(tag) {
            Some(f) => imports.push(f),
            None => bail!(
                "element tag <{tag}> has no FQCN table entry; add it to pi_tables from a corpus sample"
            ),
        }
    }
    imports.sort_unstable();

    let mut versions: Vec<(String, i32)> =
        pi_tables::VERSIONS_ALWAYS_5_4.iter().map(|(n, v)| (n.to_string(), *v)).collect();
    for tag in &tags {
        if let Some((name, v)) = pi_tables::version_for_tag(tag) {
            versions.push((name.to_string(), v));
        }
    }

    doc.root.set_attr("fileFormatVersion", FRESH_FILE_FORMAT_VERSION);
    doc.prolog.clear();
    for (name, v) in versions {
        doc.prolog.push(Pi::new("version", format!("{name}:{v}")));
    }
    for fqcn in imports {
        doc.prolog.push(Pi::new("import", fqcn));
    }
    Ok(xml::write(&doc))
}

/// Reconciles a written document's prologue with the tags it actually contains.
///
/// A replayed prologue misses any class first introduced after the read. Missing imports are
/// added (the document's own FQCN spelling wins over the 5.4 table, so an older era's package
/// layout is never rewritten); imports the editor wrote for classes with no element are kept,
/// since the official reader tolerates extras and real files carry them. Version PIs are only
/// appended, never pruned, and only for a 5.4 document. A document whose prologue already covers
/// every tag is left untouched, so an unedited round trip stays byte-identical.
pub fn complete_prologue(doc: &mut Document) -> Result<()> {
    let tags = model_tags(&doc.root);

    let replayed_imports: Vec<String> = doc
        .prolog
        .iter()
        .filter(|p| p.target == "import")
        .map(|p| p.data.trim().to_string())
        .collect();
    let replayed_by_tag: HashMap<String, &str> = replayed_imports
        .iter()
        .map(|f| {
            let after_dot = f.rsplit('.').next().unwrap_or(f);
            let tag = after_dot.rsplit('$').next().unwrap_or(after_dot);
            (tag.to_string(), f.as_str())
        })
        .collect();

    let mut missing: Vec<String> = Vec::new();
    for tag in &tags {
        if replayed_by_tag.contains_key(tag) {
            continue;
        }
        match pi_tables::fqcn_for_tag(tag) {
            Some(f) => missing.push(f.to_string()),
            None => bail!(
                "element tag <{tag}> has no FQCN table entry and no replayed import; add it to pi_tables"
            ),
        }
    }
    let mut desired: Vec<String> =
        replayed_imports.iter().cloned().chain(missing.iter().cloned()).collect();
    desired.sort_unstable();
    desired.dedup();

    let replayed_version_names: HashSet<String> = doc
        .prolog
        .iter()
        .filter(|p| p.target == "version")
        .map(|p| {
            p.data
                .trim()
                .rsplit_once(':')
                .map(|(n, _)| n.to_string())
                .unwrap_or_else(|| p.data.trim().to_string())
        })
        .collect();
    let appended: Vec<(String, i32)> =
        if doc.root.attr("fileFormatVersion") == Some(FRESH_FILE_FORMAT_VERSION) {
            tags.iter()
                .filter_map(|tag| pi_tables::version_for_tag(tag))
                .filter(|(name, _)| !replayed_version_names.contains(*name))
                .map(|(name, v)| (name.to_string(), v))
                .collect()
        } else {
            Vec::new()
        };

    if missing.is_empty() && appended.is_empty() {
        return Ok(());
    }

    doc.prolog.retain(|p| p.target != "import");
    let mut insert_at =
        doc.prolog.iter().rposition(|p| p.target == "version").map(|i| i + 1).unwrap_or(0);
    for (name, v) in appended {
        doc.prolog.insert(insert_at, Pi::new("version", format!("{name}:{v}")));
        insert_at += 1;
    }
    for fqcn in desired {
        doc.prolog.insert(insert_at, Pi::new("import", fqcn));
        insert_at += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Field, Object};

    #[test]
    fn fresh_document_gets_sorted_imports_and_versions() {
        let mut root = Object::new("CModelSource");
        root.fields.push(Field {
            name: Some("guid".into()),
            value: Value::object(Object::guid("CModelGuid", "u", "n")),
        });
        root.fields.push(Field { name: Some("name".into()), value: Value::str("x") });
        let bytes = write_fresh_main_xml(&Value::object(root)).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("<?version SerializeFormatVersion:2?>\r\n<?version CModelSource:16?>\r\n<?import com.live2d.cubism.doc.model.CModelSource?>\r\n<?import com.live2d.type.CModelGuid?>\r\n<root fileFormatVersion=\"504000000\">"), "{text}");
    }

    #[test]
    fn complete_prologue_leaves_complete_documents_alone() {
        let mut root = Object::new("CModelSource");
        root.fields.push(Field { name: Some("name".into()), value: Value::str("x") });
        let bytes = write_fresh_main_xml(&Value::object(root)).unwrap();
        let mut doc = xml::parse(&bytes).unwrap();
        let before = doc.clone();
        complete_prologue(&mut doc).unwrap();
        assert_eq!(doc, before);
    }

    #[test]
    fn complete_prologue_adds_missing_import() {
        let mut root = Object::new("CModelSource");
        root.fields.push(Field {
            name: Some("guid".into()),
            value: Value::object(Object::guid("CModelGuid", "u", "n")),
        });
        let mut doc = model::write_root(&Value::object(root));
        doc.root.set_attr("fileFormatVersion", "401010001");
        doc.prolog.push(Pi::new("import", "com.live2d.cubism.doc.model.CModelSource"));
        complete_prologue(&mut doc).unwrap();
        let imports: Vec<_> =
            doc.prolog.iter().filter(|p| p.target == "import").map(|p| p.data.clone()).collect();
        assert_eq!(
            imports,
            vec!["com.live2d.cubism.doc.model.CModelSource", "com.live2d.type.CModelGuid"]
        );
        assert!(
            doc.prolog.iter().all(|p| p.target != "version"),
            "non-5.4 doc gains no version PIs"
        );
    }
}
