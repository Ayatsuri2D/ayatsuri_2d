//! Schema inference over model graphs and the generator for the typed wrapper layer.
//!
//! The dynamic graph is the source of truth; the typed layer is a view over it. `infer` walks one
//! or more graphs and records, per class tag, the ordered field names, the value kinds each field
//! was seen holding, the own attributes, and the `super` tag. `emit_rust` turns that into
//! `src/typed_gen.rs`: one wrapper struct per class over an `ObjRef`, with typed getters and
//! setters, so edits made through the typed view are edits to the same object the writer emits.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Write as _;

use crate::model::{ListKind, MapKind, ModelGraph, Object, Value};

/// A value shape a field was observed with.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Null,
    Int,
    Float,
    Double,
    Long,
    Short,
    Byte,
    Char,
    Bool,
    Str,
    FloatArray,
    IntArray,
    LongArray,
    ShortArray,
    DoubleArray,
    ByteArray,
    CharArray,
    BoolArray,
    List(ListKind),
    Set,
    Map(MapKind),
    File,
    /// A class element (also covers enum, Guid and Id tags; the class record says which).
    Class(String),
    Other,
}

impl PartialOrd for ListKind {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ListKind {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.tag().cmp(other.tag())
    }
}

impl PartialOrd for MapKind {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MapKind {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.tag().cmp(other.tag())
    }
}

#[derive(Debug, Clone, Default)]
pub struct FieldSchema {
    pub name: String,
    pub kinds: BTreeSet<Kind>,
    /// Item kinds when the field held a list or set.
    pub item_kinds: BTreeSet<Kind>,
    /// Instances of the owning class that carried this field.
    pub present: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassShape {
    /// Fields as children, optional own attributes.
    Class,
    /// `<Tag v="CONSTANT"/>`.
    Enum,
    /// `<Tag uuid=".." note=".."/>`.
    Guid,
    /// `<Tag idstr=".."/>`.
    Id,
}

#[derive(Debug, Clone)]
pub struct ClassSchema {
    pub tag: String,
    pub shape: ClassShape,
    pub instances: usize,
    /// Own attributes in first-seen order.
    pub attrs: Vec<String>,
    /// Fields in merged document order (`super` included where it appears).
    pub fields: Vec<FieldSchema>,
    /// The `super` level's tag, when any instance had one.
    pub super_tag: Option<String>,
    /// Enum constants seen.
    pub constants: BTreeSet<String>,
}

impl ClassSchema {
    fn new(tag: &str) -> Self {
        ClassSchema {
            tag: tag.to_string(),
            shape: ClassShape::Class,
            instances: 0,
            attrs: Vec::new(),
            fields: Vec::new(),
            super_tag: None,
            constants: BTreeSet::new(),
        }
    }

    fn field_mut(&mut self, name: &str, after: Option<&str>) -> &mut FieldSchema {
        if let Some(i) = self.fields.iter().position(|f| f.name == name) {
            return &mut self.fields[i];
        }
        let at = match after.and_then(|a| self.fields.iter().position(|f| f.name == a)) {
            Some(i) => i + 1,
            None => self.fields.len(),
        };
        self.fields.insert(at, FieldSchema { name: name.to_string(), ..Default::default() });
        &mut self.fields[at]
    }
}

#[derive(Debug, Clone, Default)]
pub struct Schema {
    pub classes: BTreeMap<String, ClassSchema>,
}

fn kind_of(v: &Value) -> Kind {
    match v {
        Value::Null => Kind::Null,
        Value::Int(_) => Kind::Int,
        Value::Float(_) => Kind::Float,
        Value::Double(_) => Kind::Double,
        Value::Long(_) => Kind::Long,
        Value::Short(_) => Kind::Short,
        Value::Byte(_) => Kind::Byte,
        Value::Char(_) => Kind::Char,
        Value::Bool(_) => Kind::Bool,
        Value::Str(_) => Kind::Str,
        Value::FloatArray(_) => Kind::FloatArray,
        Value::IntArray(_) => Kind::IntArray,
        Value::LongArray(_) => Kind::LongArray,
        Value::ShortArray(_) => Kind::ShortArray,
        Value::DoubleArray(_) => Kind::DoubleArray,
        Value::ByteArray(_) => Kind::ByteArray,
        Value::CharArray(_) => Kind::CharArray,
        Value::BoolArray(_) => Kind::BoolArray,
        Value::List { kind, .. } => Kind::List(*kind),
        Value::Set(_) => Kind::Set,
        Value::Map { kind, .. } => Kind::Map(*kind),
        Value::File { .. } => Kind::File,
        Value::Object(o) => Kind::Class(o.borrow().tag.clone()),
        Value::Shared(s) => kind_of(&s.borrow()),
        Value::DanglingRef { tag, .. } => Kind::Class(tag.clone()),
        Value::Raw(_) => Kind::Other,
    }
}

fn shape_of(o: &Object) -> ClassShape {
    if !o.fields.is_empty() || o.text.is_some() {
        return ClassShape::Class;
    }
    let names: Vec<&str> = o.attrs.iter().map(|(k, _)| k.as_str()).collect();
    match names.as_slice() {
        ["v"] => ClassShape::Enum,
        ["uuid", "note"] if o.tag.ends_with("Guid") => ClassShape::Guid,
        ["idstr"] if o.tag.ends_with("Id") => ClassShape::Id,
        _ => ClassShape::Class,
    }
}

impl Schema {
    /// Merges every class instance reachable in `graph` into the schema.
    pub fn absorb(&mut self, graph: &ModelGraph) {
        let mut seen: HashSet<*const ()> = HashSet::new();
        let mut stack: Vec<Value> = Vec::new();
        if let Some(r) = &graph.root {
            stack.push(r.clone());
        }
        for e in &graph.shared {
            stack.push(e.value.clone());
        }
        while let Some(v) = stack.pop() {
            if let Some(p) = v.identity()
                && !seen.insert(p)
            {
                continue;
            }
            match &v {
                Value::Object(o) => {
                    let obj = o.borrow();
                    self.absorb_object(&obj);
                    for f in &obj.fields {
                        stack.push(f.value.clone());
                    }
                }
                Value::Shared(s) => stack.push(s.borrow().clone()),
                Value::List { items, .. } | Value::Set(items) => {
                    stack.extend(items.iter().cloned())
                }
                Value::Map { entries, .. } => {
                    for (k, val) in entries {
                        stack.push(k.clone());
                        stack.push(val.clone());
                    }
                }
                _ => {}
            }
        }
    }

    fn absorb_object(&mut self, obj: &Object) {
        let shape = shape_of(obj);
        let class =
            self.classes.entry(obj.tag.clone()).or_insert_with(|| ClassSchema::new(&obj.tag));
        let first = class.instances == 0;
        class.instances += 1;
        // A tag seen both as enum-shaped and class-shaped is a class.
        class.shape = if first || class.shape == shape { shape } else { ClassShape::Class };
        if shape == ClassShape::Enum
            && let Some(c) = obj.attr("v")
        {
            class.constants.insert(c.to_string());
        }
        for (k, _) in &obj.attrs {
            if !class.attrs.iter().any(|a| a == k) {
                class.attrs.push(k.clone());
            }
        }
        let mut prev: Option<String> = None;
        for f in &obj.fields {
            let Some(name) = &f.name else { continue };
            if name == "super"
                && let Value::Object(s) = &f.value
            {
                class.super_tag = Some(s.borrow().tag.clone());
            }
            let field = class.field_mut(name, prev.as_deref());
            field.present += 1;
            field.kinds.insert(kind_of(&f.value));
            match &f.value {
                Value::List { items, .. } | Value::Set(items) => {
                    for it in items {
                        field.item_kinds.insert(kind_of(it));
                    }
                }
                Value::Shared(s) => {
                    if let Value::List { items, .. } | Value::Set(items) = &*s.borrow() {
                        for it in items {
                            field.item_kinds.insert(kind_of(it));
                        }
                    }
                }
                _ => {}
            }
            prev = Some(name.clone());
        }
    }

    /// Human-readable dump.
    pub fn describe(&self) -> String {
        let mut out = String::new();
        for c in self.classes.values() {
            let _ = writeln!(
                out,
                "{} [{:?}, {} instances]{}",
                c.tag,
                c.shape,
                c.instances,
                c.super_tag.as_ref().map(|s| format!(" : {s}")).unwrap_or_default()
            );
            if !c.attrs.is_empty() {
                let _ = writeln!(out, "  @ {}", c.attrs.join(", "));
            }
            if c.shape == ClassShape::Enum {
                let _ = writeln!(
                    out,
                    "  = {}",
                    c.constants.iter().cloned().collect::<Vec<_>>().join(" | ")
                );
            }
            for f in &c.fields {
                let kinds: Vec<String> = f.kinds.iter().map(kind_label).collect();
                let items = if f.item_kinds.is_empty() {
                    String::new()
                } else {
                    format!(
                        "<{}>",
                        f.item_kinds.iter().map(kind_label).collect::<Vec<_>>().join("|")
                    )
                };
                let opt = if f.present < c.instances { "?" } else { "" };
                let _ = writeln!(out, "  {}{}: {}{}", f.name, opt, kinds.join("|"), items);
            }
        }
        out
    }
}

fn kind_label(k: &Kind) -> String {
    match k {
        Kind::Class(t) => t.clone(),
        Kind::List(l) => l.tag().to_string(),
        Kind::Map(m) => m.tag().to_string(),
        other => format!("{other:?}").to_lowercase(),
    }
}

// ---------------------------------------------------------------------------------------------
// Rust emitter
// ---------------------------------------------------------------------------------------------

const KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
];

/// `camelCase`, `_leading`, `dotted.name` to a Rust method name.
pub fn method_name(field: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for ch in field.trim_start_matches('_').chars() {
        if ch.is_ascii_uppercase() {
            if prev_lower {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_lower = false;
        } else if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        } else {
            if !out.ends_with('_') {
                out.push('_');
            }
            prev_lower = false;
        }
    }
    let out = out.trim_end_matches('_').to_string();
    let out = if out.is_empty() { "field".to_string() } else { out };
    let out = if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("f{out}")
    } else {
        out
    };
    if KEYWORDS.contains(&out.as_str()) { format!("{out}_") } else { out }
}

/// What a getter returns for a field, and how a setter wraps its argument.
enum RustType {
    Prim { ty: &'static str, variant: &'static str },
    Str,
    Array { ty: &'static str, variant: &'static str },
    Enum(String),
    Guid,
    Id,
    File,
    Class(String),
    List(Option<String>),
    Raw,
}

fn prim(ty: &'static str, variant: &'static str) -> RustType {
    RustType::Prim { ty, variant }
}

fn array(ty: &'static str, variant: &'static str) -> RustType {
    RustType::Array { ty, variant }
}

fn rust_type(field: &FieldSchema, schema: &Schema) -> RustType {
    let kinds: Vec<&Kind> = field.kinds.iter().filter(|k| **k != Kind::Null).collect();
    if kinds.len() != 1 {
        return RustType::Raw;
    }
    match kinds[0] {
        Kind::Int => prim("i32", "Int"),
        Kind::Float => prim("f32", "Float"),
        Kind::Double => prim("f64", "Double"),
        Kind::Long => prim("i64", "Long"),
        Kind::Short => prim("i16", "Short"),
        Kind::Byte => prim("i8", "Byte"),
        Kind::Char => prim("char", "Char"),
        Kind::Bool => prim("bool", "Bool"),
        Kind::Str => RustType::Str,
        Kind::FloatArray => array("f32", "FloatArray"),
        Kind::IntArray => array("i32", "IntArray"),
        Kind::LongArray => array("i64", "LongArray"),
        Kind::ShortArray => array("i16", "ShortArray"),
        Kind::DoubleArray => array("f64", "DoubleArray"),
        Kind::ByteArray => array("i8", "ByteArray"),
        Kind::CharArray => array("char", "CharArray"),
        Kind::BoolArray => array("bool", "BoolArray"),
        Kind::File => RustType::File,
        Kind::List(_) | Kind::Set => {
            let items: Vec<&Kind> = field.item_kinds.iter().filter(|k| **k != Kind::Null).collect();
            match items.as_slice() {
                [Kind::Class(t)] => match schema.classes.get(t).map(|c| c.shape) {
                    Some(ClassShape::Class) => RustType::List(Some(t.clone())),
                    _ => RustType::List(None),
                },
                _ => RustType::List(None),
            }
        }
        Kind::Class(t) => match schema.classes.get(t).map(|c| c.shape) {
            Some(ClassShape::Enum) => RustType::Enum(t.clone()),
            Some(ClassShape::Guid) => RustType::Guid,
            Some(ClassShape::Id) => RustType::Id,
            Some(ClassShape::Class) => RustType::Class(t.clone()),
            None => RustType::Raw,
        },
        Kind::Map(_) | Kind::Other | Kind::Null => RustType::Raw,
    }
}

/// Emits `typed_gen.rs`.
pub fn emit_rust(schema: &Schema) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "//! Typed wrappers over the dynamic model graph. GENERATED by `cmo3 schema --rust`; do not edit."
    );
    let _ = writeln!(out, "//!");
    let _ = writeln!(
        out,
        "//! Each struct wraps the `ObjRef` of one class element. Getters return `None` when the field is"
    );
    let _ = writeln!(
        out,
        "//! absent, `<null>`, or holds an unexpected shape; setters replace the field in place (appending"
    );
    let _ = writeln!(
        out,
        "//! it when absent). Fields whose shape varied across the corpus are exposed as raw `Value`."
    );
    let _ = writeln!(
        out,
        "#![allow(non_camel_case_types, clippy::too_many_lines, clippy::module_name_repetitions, dead_code)]"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "use crate::model::{{ObjRef, Value}};");
    let _ = writeln!(out, "use crate::typed::*;");
    let _ = writeln!(out);

    for class in schema.classes.values() {
        match class.shape {
            ClassShape::Guid | ClassShape::Id => continue,
            ClassShape::Enum => emit_enum(&mut out, class),
            ClassShape::Class => emit_class(&mut out, class, schema),
        }
    }

    // Tag registry: which Guid/Id tags exist, for documentation and validation.
    let guids: Vec<&str> = schema
        .classes
        .values()
        .filter(|c| c.shape == ClassShape::Guid)
        .map(|c| c.tag.as_str())
        .collect();
    let ids: Vec<&str> = schema
        .classes
        .values()
        .filter(|c| c.shape == ClassShape::Id)
        .map(|c| c.tag.as_str())
        .collect();
    let _ = writeln!(out, "/// Every `*Guid` tag seen in the corpus.");
    let _ = writeln!(
        out,
        "pub const GUID_TAGS: &[&str] = &[{}];",
        guids.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(", ")
    );
    let _ = writeln!(out, "/// Every `*Id` tag seen in the corpus.");
    let _ = writeln!(
        out,
        "pub const ID_TAGS: &[&str] = &[{}];",
        ids.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(", ")
    );
    out
}

fn emit_enum(out: &mut String, class: &ClassSchema) {
    let _ = writeln!(out, "/// Enum `<{} v=\"..\"/>` ({} instances).", class.tag, class.instances);
    let _ = writeln!(out, "#[derive(Clone)]");
    let _ = writeln!(out, "pub struct {}(pub ObjRef);", class.tag);
    let _ = writeln!(out, "impl_typed!({});", class.tag);
    let _ = writeln!(out, "impl {} {{", class.tag);
    for c in &class.constants {
        let name = const_name(c);
        let _ = writeln!(out, "    pub const {name}: &'static str = \"{c}\";");
    }
    let _ = writeln!(out, "    /// A fresh `<{} v=\"constant\"/>`.", class.tag);
    let _ = writeln!(
        out,
        "    pub fn new(constant: &str) -> Self {{ Self(new_enum(\"{}\", constant)) }}",
        class.tag
    );
    let _ =
        writeln!(out, "    pub fn constant(&self) -> Option<String> {{ enum_constant(&self.0) }}");
    let _ = writeln!(
        out,
        "    pub fn set_constant(&self, constant: &str) {{ set_enum_constant(&self.0, constant) }}"
    );
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
}

fn const_name(c: &str) -> String {
    let mut s: String = c
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_uppercase() } else { '_' })
        .collect();
    if s.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        s.insert(0, '_');
    }
    if s.is_empty() { "EMPTY".into() } else { s }
}

fn emit_class(out: &mut String, class: &ClassSchema, schema: &Schema) {
    let sup =
        class.super_tag.as_deref().map(|s| format!(" Super level: `{s}`.")).unwrap_or_default();
    let _ = writeln!(out, "/// `<{}>` ({} instances).{}", class.tag, class.instances, sup);
    if !class.attrs.is_empty() {
        let _ = writeln!(
            out,
            "/// Attributes: {}.",
            class.attrs.iter().map(|a| format!("`{a}`")).collect::<Vec<_>>().join(", ")
        );
    }
    let _ = writeln!(out, "#[derive(Clone)]");
    let _ = writeln!(out, "pub struct {}(pub ObjRef);", class.tag);
    let _ = writeln!(out, "impl_typed!({});", class.tag);
    let _ = writeln!(out, "impl {} {{", class.tag);
    let _ = writeln!(
        out,
        "    /// A fresh, empty `<{}>` (no fields; add them in editor order).",
        class.tag
    );
    let _ =
        writeln!(out, "    pub fn new_empty() -> Self {{ Self(new_object(\"{}\")) }}", class.tag);

    let mut used: HashSet<String> = HashSet::new();
    let mut unique = |base: String| -> String {
        let mut name = base;
        while !used.insert(name.clone()) {
            name.push('_');
        }
        name
    };

    for attr in &class.attrs {
        let m = unique(format!("attr_{}", method_name(attr)));
        let _ = writeln!(out, "    /// Attribute `{attr}` as text.");
        let _ = writeln!(
            out,
            "    pub fn {m}(&self) -> Option<String> {{ attr(&self.0, \"{attr}\") }}"
        );
        let _ = writeln!(
            out,
            "    pub fn set_{m}(&self, value: &str) {{ set_attr(&self.0, \"{attr}\", value) }}"
        );
    }

    for field in &class.fields {
        let n = &field.name;
        let opt = if field.present < class.instances { " (optional in corpus)" } else { "" };
        if n == "super" {
            if let Some(st) = &class.super_tag
                && schema.classes.get(st).is_some_and(|c| c.shape == ClassShape::Class)
            {
                let _ = writeln!(out, "    /// The `<{st} xs.n=\"super\">` level.");
                let _ = writeln!(
                    out,
                    "    pub fn super_(&self) -> Option<{st}> {{ get_class(&self.0, \"super\") }}"
                );
            }
            continue;
        }
        let m = unique(method_name(n));
        let kinds: Vec<String> = field.kinds.iter().map(kind_label).collect();
        let _ = writeln!(out, "    /// `xs.n=\"{n}\"`: {}{opt}.", kinds.join(" | "));
        match rust_type(field, schema) {
            RustType::Prim { ty, variant } => {
                let _ = writeln!(
                    out,
                    "    pub fn {m}(&self) -> Option<{ty}> {{ match get(&self.0, \"{n}\") {{ Some(Value::{variant}(v)) => Some(v), _ => None }} }}"
                );
                let _ = writeln!(
                    out,
                    "    pub fn set_{m}(&self, value: {ty}) {{ set(&self.0, \"{n}\", Value::{variant}(value)) }}"
                );
            }
            RustType::Str => {
                let _ = writeln!(
                    out,
                    "    pub fn {m}(&self) -> Option<String> {{ match get(&self.0, \"{n}\") {{ Some(Value::Str(v)) => Some(v), _ => None }} }}"
                );
                let _ = writeln!(
                    out,
                    "    pub fn set_{m}(&self, value: &str) {{ set(&self.0, \"{n}\", Value::Str(value.to_string())) }}"
                );
            }
            RustType::Array { ty, variant } => {
                let _ = writeln!(
                    out,
                    "    pub fn {m}(&self) -> Option<Vec<{ty}>> {{ match get(&self.0, \"{n}\") {{ Some(Value::{variant}(v)) => Some(v), _ => None }} }}"
                );
                let _ = writeln!(
                    out,
                    "    pub fn set_{m}(&self, value: Vec<{ty}>) {{ set(&self.0, \"{n}\", Value::{variant}(value)) }}"
                );
            }
            RustType::Enum(t) => {
                let _ = writeln!(
                    out,
                    "    pub fn {m}(&self) -> Option<{t}> {{ get_class(&self.0, \"{n}\") }}"
                );
                let _ = writeln!(
                    out,
                    "    pub fn set_{m}(&self, value: &{t}) {{ set_object(&self.0, \"{n}\", &value.0) }}"
                );
            }
            RustType::Guid => {
                let _ = writeln!(
                    out,
                    "    pub fn {m}(&self) -> Option<Guid> {{ get_guid(&self.0, \"{n}\") }}"
                );
                let _ = writeln!(
                    out,
                    "    pub fn set_{m}(&self, value: &Guid) {{ set_object(&self.0, \"{n}\", &value.0) }}"
                );
            }
            RustType::Id => {
                let _ = writeln!(
                    out,
                    "    pub fn {m}(&self) -> Option<Id> {{ get_id(&self.0, \"{n}\") }}"
                );
                let _ = writeln!(
                    out,
                    "    pub fn set_{m}(&self, value: &Id) {{ set_object(&self.0, \"{n}\", &value.0) }}"
                );
            }
            RustType::File => {
                let _ = writeln!(
                    out,
                    "    pub fn {m}(&self) -> Option<FileRef> {{ get_file(&self.0, \"{n}\") }}"
                );
                let _ = writeln!(
                    out,
                    "    pub fn set_{m}(&self, value: FileRef) {{ set(&self.0, \"{n}\", value.into_value()) }}"
                );
            }
            RustType::Class(t) => {
                let _ = writeln!(
                    out,
                    "    pub fn {m}(&self) -> Option<{t}> {{ get_class(&self.0, \"{n}\") }}"
                );
                let _ = writeln!(
                    out,
                    "    pub fn set_{m}(&self, value: &{t}) {{ set_object(&self.0, \"{n}\", &value.0) }}"
                );
            }
            RustType::List(Some(t)) => {
                let _ = writeln!(
                    out,
                    "    pub fn {m}(&self) -> Vec<{t}> {{ get_list_typed(&self.0, \"{n}\") }}"
                );
                let _ = writeln!(
                    out,
                    "    /// Appends to the list, creating an `array_list` when absent."
                );
                let _ = writeln!(
                    out,
                    "    pub fn push_{m}(&self, value: &{t}) {{ push_list(&self.0, \"{n}\", Value::Object(value.0.clone())) }}"
                );
                let _ = writeln!(
                    out,
                    "    pub fn {m}_value(&self) -> Option<Value> {{ get(&self.0, \"{n}\") }}"
                );
                let _ = writeln!(
                    out,
                    "    pub fn set_{m}_value(&self, value: Value) {{ set(&self.0, \"{n}\", value) }}"
                );
            }
            RustType::List(None) => {
                let _ = writeln!(
                    out,
                    "    pub fn {m}(&self) -> Vec<Value> {{ get_list(&self.0, \"{n}\") }}"
                );
                let _ = writeln!(
                    out,
                    "    pub fn push_{m}(&self, value: Value) {{ push_list(&self.0, \"{n}\", value) }}"
                );
                let _ = writeln!(
                    out,
                    "    pub fn {m}_value(&self) -> Option<Value> {{ get(&self.0, \"{n}\") }}"
                );
                let _ = writeln!(
                    out,
                    "    pub fn set_{m}_value(&self, value: Value) {{ set(&self.0, \"{n}\", value) }}"
                );
            }
            RustType::Raw => {
                let _ = writeln!(
                    out,
                    "    pub fn {m}(&self) -> Option<Value> {{ get(&self.0, \"{n}\") }}"
                );
                let _ = writeln!(
                    out,
                    "    pub fn set_{m}(&self, value: Value) {{ set(&self.0, \"{n}\", value) }}"
                );
            }
        }
    }
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_names() {
        assert_eq!(method_name("minValue"), "min_value");
        assert_eq!(method_name("_sources"), "sources");
        assert_eq!(method_name("parameters.keys"), "parameters_keys");
        assert_eq!(method_name("useLegacyDrawOrder__testImpl"), "use_legacy_draw_order_test_impl");
        assert_eq!(method_name("type"), "type_");
        assert_eq!(method_name("_icon64"), "icon64");
        assert_eq!(method_name("isReflectX"), "is_reflect_x");
        assert_eq!(method_name("uvs"), "uvs");
    }
}
