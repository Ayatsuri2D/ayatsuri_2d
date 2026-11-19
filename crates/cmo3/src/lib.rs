//! Live2D Cubism `.cmo3` reader and writer.
//!
//! A `.cmo3` is a CAFF container (header, preview block, XOR-obfuscated file table, blobs) whose
//! `main_xml` entry holds the serialized editor model graph. Format knowledge comes from black-box
//! observation of editor output. Modules:
//!
//! - [`io`]: big-endian, XOR-keyed binary reader/writer.
//! - [`caff`]: the container codec (entries, preview, single-entry zip framing).
//! - [`xml`]: a small XML parser and a writer that reproduces the two known editor layouts (the
//!   JDOM 1.1.3 pretty format with indent `""`, TRIM and CRLF; and the indented LF form of
//!   fileFormatVersion 502030002 files). An unedited main.xml re-emits byte for byte.
//! - [`model`]: the dynamic model graph (`Value`/`Object`), the shared pool, and the
//!   `<root><shared/><main/></root>` serializer with the editor's hoist-on-second-use rule.
//! - [`version`], [`pi_tables`], [`author`]: schema generation sniffing, target SDK versions,
//!   and the `<?import?>`/`<?version?>` prologue tables.
//! - [`cmo3`]: the high-level [`Cmo3Model`] API (read, mutate, embed PNGs, write).
//!
//! - [`schema`], [`typed`], [`typed_gen`]: schema inference over real files and the generated
//!   typed wrapper layer (one struct per class over the same `ObjRef`, so typed edits are graph
//!   edits).
//!
//! Every class element is a generic [`model::Object`] (tag, attributes, ordered fields); the
//! typed layer is a view over it, never a copy, which keeps round-trips lossless.

pub mod author;
pub mod caff;
pub mod cmo3;
pub mod error;
pub mod io;
pub mod model;
pub mod numfmt;
pub mod pi_tables;
pub mod schema;
pub mod typed;
pub mod typed_gen;
pub mod version;
pub mod xml;

pub use cmo3::Cmo3Model;
pub use error::{Error, Result};
pub use model::{ModelGraph, ObjRef, Object, Value};
pub use typed::Typed;
