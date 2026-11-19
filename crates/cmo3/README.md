# cmo3

Local Rust reader/writer for Live2D Cubism `.cmo3` editor files. Library plus a small CLI.
For experiments only.

## Layout of a .cmo3

```
CAFF container (big-endian, XOR-obfuscated with the header key)
  header (26 B, cleartext) -> preview block (28 B) -> file table -> blobs -> 'b' 'c'
  entries: imageFileBuf*.png / image*.png (raw PNG), main.xml (tag main_xml, deflated)

main.xml
  <?version Class:N?> / <?import fqcn?> PIs
  <root fileFormatVersion="...">
    <shared>  pool of objects used more than once: xs.id / xs.idx
    <main>    the CModelSource root; fields are children with xs.n, super level is xs.n="super"
```

Two on-disk XML layouts are known and auto-detected, both re-emitted byte for byte:

| fileFormatVersion | line ends | indent | empty tag | text | floats |
|---|---|---|---|---|---|
| 5.4 (504000000) | CRLF | none, blank line at end | `<x />` | trimmed | `Float.toString` (`2.4E-4`) |
| 5.2 (502030002) | LF | 2 spaces under shared/main | `<x/>` | verbatim | plain decimal (`0.00024`) |

## Build

```
cargo build --release
./target/release/cmo3 dump model.cmo3
```

## CLI

```
cmo3 dump <file.cmo3>              header, file table, version, root overview
cmo3 xml <file.cmo3>               stored main.xml to stdout
cmo3 render <file.cmo3>            main.xml re-emitted from the parsed graph
cmo3 extract <file.cmo3> <dir>     unpack entries (main.xml + PNGs)
cmo3 roundtrip <file.cmo3> [out]   parse + re-emit, report byte identity, optionally resave
cmo3 convert <in> <out>            resave (overwrites <out>)
cmo3 images <file.cmo3>            CImageResource list
cmo3 tags <file.cmo3>              element tag histogram
cmo3 set-target <in> <out> <no>    set CModelSource.targetVersionNo
```

## Library sketch

```rust
use cmo3::{Cmo3Model, Value, Object};

let mut model = Cmo3Model::read_file("model.cmo3")?;
let root = model.root().unwrap();                 // Rc<RefCell<Object>>, tag CModelSource
let name = root.borrow().field("name").cloned();  // Some(Value::Str(..))
root.borrow_mut().set_field("name", Value::str("renamed"));

for res in model.image_resources() {              // every CImageResource
    let png = model.extract_layer_png(&res);
}
model.set_target_version_no(5_030_000)?;
model.write_file("out.cmo3")?;
```

Objects are generic: `tag`, `attrs` (enum `v`, Guid `uuid`/`note`, Id `idstr`, ...), `fields`
(ordered, `xs.n` names). Shared pool entries are the same `Rc` wherever referenced; a fresh
object placed in two fields is hoisted into `<shared>` on write, like the editor does.

## Typed layer

`src/typed_gen.rs` is generated from real files and wraps the dynamic graph: one struct per
class over the same `ObjRef`, so typed edits are graph edits and round-trips stay lossless.

```rust
use cmo3::{Cmo3Model, Typed};
use cmo3::typed_gen::CModelSource;

let model = Cmo3Model::read_file("model.cmo3")?;
let root = CModelSource::try_from_ref(&model.root().unwrap()).unwrap();
for p in root.parameter_source_set().unwrap().sources() {
    println!("{} {:?}..{:?}", p.id().unwrap().idstr(), p.min_value(), p.max_value());
    p.set_max_value(45.0);               // writes into the graph
}
```

Regenerate after adding files to the corpus (new classes or fields appear only if some file has
them):

```
cmo3 schema a.cmo3 b.cmo3 --rust src/typed_gen.rs
cmo3 schema a.cmo3                 # human-readable schema dump
```

Getter shapes: scalars `Option<T>`, strings `Option<String>`, arrays `Option<Vec<T>>`, class
fields `Option<Wrapper>`, lists of one class `Vec<Wrapper>` plus `push_x`, enums a wrapper with
`constant()` and `pub const` names, `*Guid`/`*Id` as `typed::Guid`/`typed::Id`, `<file>` as
`typed::FileRef`. Fields whose shape varied in the corpus are raw `Option<Value>`. Attributes are
`attr_name()` text accessors. `super_()` returns the parent-class level.

## Notes

- The whole file is not byte-identical after a resave: compressed entries get a new DOS
  timestamp and a different DEFLATE encoder. The decompressed main.xml is.
- `Value::Raw` holds any element that failed to decode as a typed value; it is re-emitted
  unchanged. `graph.diagnostics` lists them.
- Unknown: how the 5.2 writer spells floats of magnitude 1e7 and above. Plain decimal is assumed.
