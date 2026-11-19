//! The dynamic model graph behind main.xml.
//!
//! The editor's serializer writes `<root><shared/><main/></root>`: `<main>` holds the single
//! `CModelSource` root; `<shared>` holds every object referenced more than once, each with an
//! `xs.id` and a pool index `xs.idx`, and every use site is `<Tag xs.n="field" xs.ref="id"/>`.
//! Class instances are elements named after the class with one child per field (`xs.n` carries the
//! field name, `super` nests the superclass level); primitives, arrays and collections have fixed
//! tags (`i`, `f`, `s`, `float-array`, `array_list`, `hash_map`, ...).
//!
//! Every class instance is a generic [`Object`] (tag, own attributes, ordered fields). Identity is carried by
//! `Rc`, so an object reachable from two places is one object, and the writer reproduces the
//! editor's hoist-on-second-use behaviour: the first inline occurrence is replaced by a reference
//! and the definition moves into `<shared>`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::numfmt;
use crate::xml::{self, Document, Element, Node, Pi};
use crate::{Result, bail};

pub const ATTR_NAME: &str = "xs.n";
pub const ATTR_ID: &str = "xs.id";
pub const ATTR_REF: &str = "xs.ref";
pub const ATTR_INDEX: &str = "xs.idx";
pub const ATTR_COUNT: &str = "count";
pub const ATTR_KEY_TYPE: &str = "keyType";
pub const ATTR_ENUM_VALUE: &str = "v";
pub const TAG_NULL: &str = "null";
pub const TAG_ENTRY: &str = "entry";
pub const TAG_FILE: &str = "file";
pub const NAME_SUPER: &str = "super";
pub const NAME_KEY: &str = "key";
pub const NAME_VALUE: &str = "value";
pub const ROOT_ELEMENT: &str = "root";
pub const SHARED_ELEMENT: &str = "shared";
pub const MAIN_ELEMENT: &str = "main";

/// A class instance: identity-bearing handle.
pub type ObjRef = Rc<RefCell<Object>>;
/// A non-object value living in the shared pool (so references can point at it).
pub type SharedRef = Rc<RefCell<Value>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    ArrayList,
    CArrayList,
}

impl ListKind {
    pub fn tag(self) -> &'static str {
        match self {
            ListKind::ArrayList => "array_list",
            ListKind::CArrayList => "carray_list",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapKind {
    HashMap,
    LinkedMap,
}

impl MapKind {
    pub fn tag(self) -> &'static str {
        match self {
            MapKind::HashMap => "hash_map",
            MapKind::LinkedMap => "linked_map",
        }
    }
}

/// One child of an [`Object`]: the `xs.n` field name (None for a nameless child) and its value.
#[derive(Debug, Clone)]
pub struct Field {
    pub name: Option<String>,
    pub value: Value,
}

/// A serialized class instance. `tag` is the element name (the class's simple name), `attrs` its
/// own attributes in order (enum `v`, Guid `uuid`/`note`, Id `idstr`, CAffine `m00`.., any
/// `@SerialAttribute` field), `fields` its child elements in document order.
#[derive(Debug, Clone, Default)]
pub struct Object {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub fields: Vec<Field>,
    /// Character content, which class elements never carry; kept so an odd file still round-trips.
    pub text: Option<String>,
}

impl Object {
    pub fn new(tag: impl Into<String>) -> Self {
        Object { tag: tag.into(), ..Default::default() }
    }

    pub fn into_ref(self) -> ObjRef {
        Rc::new(RefCell::new(self))
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn set_attr(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();
        match self.attrs.iter_mut().find(|(k, _)| *k == name) {
            Some(slot) => slot.1 = value,
            None => self.attrs.push((name, value)),
        }
    }

    pub fn remove_attr(&mut self, name: &str) -> Option<String> {
        let i = self.attrs.iter().position(|(k, _)| k == name)?;
        Some(self.attrs.remove(i).1)
    }

    /// The first field named `name` (searches this level only, not `super`).
    pub fn field(&self, name: &str) -> Option<&Value> {
        self.fields.iter().find(|f| f.name.as_deref() == Some(name)).map(|f| &f.value)
    }

    pub fn field_mut(&mut self, name: &str) -> Option<&mut Value> {
        self.fields.iter_mut().find(|f| f.name.as_deref() == Some(name)).map(|f| &mut f.value)
    }

    /// The `<super>` level object, if this instance has one.
    pub fn super_object(&self) -> Option<ObjRef> {
        match self.field(NAME_SUPER) {
            Some(Value::Object(o)) => Some(o.clone()),
            _ => None,
        }
    }

    /// Looks a field up on this level, then up the `super` chain (how the editor reads a class).
    pub fn lookup(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.field(name) {
            return Some(v.clone());
        }
        self.super_object().and_then(|s| s.borrow().lookup(name))
    }

    /// Replaces the field's value in place, or appends a new field.
    pub fn set_field(&mut self, name: impl Into<String>, value: Value) {
        let name = name.into();
        match self.fields.iter_mut().find(|f| f.name.as_deref() == Some(name.as_str())) {
            Some(f) => f.value = value,
            None => self.fields.push(Field { name: Some(name), value }),
        }
    }

    /// Replaces the field in place, or inserts it before `before` (appending when `before` is
    /// absent). A field the source omitted has to be placed where the editor writes it.
    pub fn set_field_before(&mut self, name: impl Into<String>, value: Value, before: &str) {
        let name = name.into();
        if let Some(f) = self.fields.iter_mut().find(|f| f.name.as_deref() == Some(name.as_str())) {
            f.value = value;
            return;
        }
        let field = Field { name: Some(name), value };
        match self.fields.iter().position(|f| f.name.as_deref() == Some(before)) {
            Some(i) => self.fields.insert(i, field),
            None => self.fields.push(field),
        }
    }

    pub fn remove_field(&mut self, name: &str) -> Option<Value> {
        let i = self.fields.iter().position(|f| f.name.as_deref() == Some(name))?;
        Some(self.fields.remove(i).value)
    }

    // Convenience constructors for the attribute-only leaf classes.

    /// `<Kind uuid=".." note=".."/>`.
    pub fn guid(kind: &str, uuid: &str, note: &str) -> Object {
        let mut o = Object::new(kind);
        o.attrs.push(("uuid".into(), uuid.into()));
        o.attrs.push(("note".into(), note.into()));
        o
    }

    /// `<Kind idstr=".."/>`.
    pub fn id(kind: &str, idstr: &str) -> Object {
        let mut o = Object::new(kind);
        o.attrs.push(("idstr".into(), idstr.into()));
        o
    }

    /// `<EnumTag v="CONSTANT"/>`.
    pub fn enum_value(tag: &str, constant: &str) -> Object {
        let mut o = Object::new(tag);
        o.attrs.push((ATTR_ENUM_VALUE.into(), constant.into()));
        o
    }

    /// The enum constant name when this is an enum element.
    pub fn enum_constant(&self) -> Option<&str> {
        if self.fields.is_empty() && self.attrs.len() == 1 {
            self.attr(ATTR_ENUM_VALUE)
        } else {
            None
        }
    }
}

/// A serialized value.
#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Int(i32),
    Float(f32),
    Double(f64),
    Long(i64),
    Short(i16),
    Byte(i8),
    Char(char),
    Bool(bool),
    Str(String),
    FloatArray(Vec<f32>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
    ShortArray(Vec<i16>),
    DoubleArray(Vec<f64>),
    ByteArray(Vec<i8>),
    CharArray(Vec<char>),
    BoolArray(Vec<bool>),
    List {
        kind: ListKind,
        items: Vec<Value>,
    },
    /// `linked_set`.
    Set(Vec<Value>),
    /// `key_type_first` preserves the attribute order seen on read (`keyType` before `count`),
    /// which the 5.2 writer varies per map class.
    Map {
        kind: MapKind,
        entries: Vec<(Value, Value)>,
        key_type_first: bool,
    },
    /// `<file path="entry.png"/>` (archive entry) or `<file>C:\path</file>` (text path).
    File {
        path: Option<String>,
        text: Option<String>,
    },
    Object(ObjRef),
    /// A non-object pool entry (for example a shared `float-array`), referenced by identity.
    Shared(SharedRef),
    /// An `xs.ref` whose target was not found. Re-emitted as written.
    DanglingRef {
        tag: String,
        id: String,
    },
    /// A subtree kept verbatim because it could not be decoded (a malformed primitive, say).
    Raw(Element),
}

impl Value {
    pub fn object(o: Object) -> Value {
        Value::Object(o.into_ref())
    }

    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    pub fn list(items: Vec<Value>) -> Value {
        Value::List { kind: ListKind::ArrayList, items }
    }

    pub fn as_object(&self) -> Option<&ObjRef> {
        match self {
            Value::Object(o) => Some(o),
            Value::Shared(s) => match &*s.borrow() {
                Value::Object(_) => None, // never nested this way by the reader
                _ => None,
            },
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i32> {
        match self {
            Value::Int(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f32> {
        match self {
            Value::Float(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List { items, .. } => Some(items),
            Value::Set(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_list_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::List { items, .. } => Some(items),
            Value::Set(items) => Some(items),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// The identity of an identity-bearing value (Object or Shared), used for pool membership and
    /// hoisting. Primitives and collections have none.
    pub fn identity(&self) -> Option<*const ()> {
        match self {
            Value::Object(o) => Some(Rc::as_ptr(o) as *const ()),
            Value::Shared(s) => Some(Rc::as_ptr(s) as *const ()),
            _ => None,
        }
    }

    /// The element tag this value serializes as.
    pub fn tag(&self) -> String {
        match self {
            Value::Null => TAG_NULL.into(),
            Value::Int(_) => "i".into(),
            Value::Float(_) => "f".into(),
            Value::Double(_) => "d".into(),
            Value::Long(_) => "l".into(),
            Value::Short(_) => "short".into(),
            Value::Byte(_) => "byte".into(),
            Value::Char(_) => "char".into(),
            Value::Bool(_) => "b".into(),
            Value::Str(_) => "s".into(),
            Value::FloatArray(_) => "float-array".into(),
            Value::IntArray(_) => "int-array".into(),
            Value::LongArray(_) => "long-array".into(),
            Value::ShortArray(_) => "short-array".into(),
            Value::DoubleArray(_) => "double-array".into(),
            Value::ByteArray(_) => "byte-array".into(),
            Value::CharArray(_) => "char-array".into(),
            Value::BoolArray(_) => "bool-array".into(),
            Value::List { kind, .. } => kind.tag().into(),
            Value::Set(_) => "linked_set".into(),
            Value::Map { kind, .. } => kind.tag().into(),
            Value::File { .. } => TAG_FILE.into(),
            Value::Object(o) => o.borrow().tag.clone(),
            Value::Shared(s) => s.borrow().tag(),
            Value::DanglingRef { tag, .. } => tag.clone(),
            Value::Raw(el) => el.name.clone(),
        }
    }
}

/// True for the tags the serializer owns (everything that is not a class element).
pub fn is_builtin_tag(tag: &str) -> bool {
    matches!(
        tag,
        "null"
            | "i"
            | "f"
            | "d"
            | "l"
            | "short"
            | "byte"
            | "char"
            | "b"
            | "s"
            | "float-array"
            | "int-array"
            | "long-array"
            | "short-array"
            | "double-array"
            | "byte-array"
            | "char-array"
            | "bool-array"
            | "array_list"
            | "carray_list"
            | "linked_set"
            | "hash_map"
            | "linked_map"
            | "file"
    )
}

/// A shared-pool entry: preserved `xs.id`, `xs.idx` and def tag, plus the value itself.
#[derive(Debug, Clone)]
pub struct SharedEntry {
    pub id: String,
    pub index: i32,
    pub tag: String,
    pub value: Value,
}

/// A deserialized model with everything needed to re-emit it byte-identically.
#[derive(Debug, Clone, Default)]
pub struct ModelGraph {
    /// The `<main>` root (normally a `CModelSource` object).
    pub root: Option<Value>,
    /// The shared pool in document order.
    pub shared: Vec<SharedEntry>,
    pub prolog: Vec<Pi>,
    pub epilog: Vec<Pi>,
    pub root_attrs: Vec<(String, String)>,
    /// The source document's on-disk layout, replayed on write.
    pub layout: xml::Layout,
    /// Notes about content that could not be decoded as typed values (kept verbatim).
    pub diagnostics: Vec<String>,
}

impl ModelGraph {
    pub fn root_object(&self) -> Option<ObjRef> {
        match &self.root {
            Some(Value::Object(o)) => Some(o.clone()),
            _ => None,
        }
    }

    pub fn root_attr(&self, name: &str) -> Option<&str> {
        self.root_attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn shared_by_id(&self, id: &str) -> Option<&SharedEntry> {
        self.shared.iter().find(|e| e.id == id)
    }

    /// `<?version Name:N?>` PIs as (name, version) pairs.
    pub fn versions(&self) -> Vec<(String, i32)> {
        self.prolog
            .iter()
            .filter(|pi| pi.target == "version")
            .filter_map(|pi| {
                let (name, ver) = pi.data.rsplit_once(':')?;
                Some((name.to_string(), ver.trim().parse().ok()?))
            })
            .collect()
    }

    /// `<?import fqcn?>` PIs in document order.
    pub fn imports(&self) -> Vec<String> {
        self.prolog
            .iter()
            .filter(|pi| pi.target == "import")
            .map(|pi| pi.data.trim().to_string())
            .collect()
    }

    /// Drops pool entries no longer reachable from the root (after structural deletes). Survivors
    /// keep their id and index; entries only verbatim content still references stay live.
    pub fn prune_unreachable_shared(&mut self) {
        let Some(root) = &self.root else { return };
        let by_id: HashMap<&str, usize> =
            self.shared.iter().enumerate().map(|(i, e)| (e.id.as_str(), i)).collect();
        let mut reached_ids: Vec<bool> = vec![false; self.shared.len()];
        let mut seen: std::collections::HashSet<*const ()> = Default::default();
        let mut stack: Vec<Value> = vec![root.clone()];

        while let Some(v) = stack.pop() {
            if let Some(ptr) = v.identity() {
                if !seen.insert(ptr) {
                    continue;
                }
                if let Some(i) = self.shared.iter().position(|e| e.value.identity() == Some(ptr)) {
                    reached_ids[i] = true;
                }
            }
            match &v {
                Value::Object(o) => {
                    for f in &o.borrow().fields {
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
                Value::DanglingRef { id, .. } => {
                    if let Some(&i) = by_id.get(id.as_str()) {
                        reached_ids[i] = true;
                        stack.push(self.shared[i].value.clone());
                    }
                }
                Value::Raw(el) => {
                    collect_raw_refs(el, &by_id, &mut reached_ids, &self.shared, &mut stack)
                }
                _ => {}
            }
        }

        let mut i = 0;
        self.shared.retain(|_| {
            let keep = reached_ids[i];
            i += 1;
            keep
        });
    }
}

fn collect_raw_refs(
    el: &Element,
    by_id: &HashMap<&str, usize>,
    reached: &mut [bool],
    shared: &[SharedEntry],
    stack: &mut Vec<Value>,
) {
    if let Some(id) = el.attr(ATTR_REF)
        && let Some(&i) = by_id.get(id)
        && !reached[i]
    {
        reached[i] = true;
        stack.push(shared[i].value.clone());
    }
    for child in el.child_elements() {
        collect_raw_refs(child, by_id, reached, shared, stack);
    }
}

// ---------------------------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------------------------

struct ReadCtx {
    refs: HashMap<String, Value>,
    diagnostics: Vec<String>,
}

/// Reads a model document into a [`ModelGraph`].
pub fn read_model(doc: &Document) -> Result<ModelGraph> {
    let root = &doc.root;
    let mut ctx = ReadCtx { refs: HashMap::new(), diagnostics: Vec::new() };

    let shared_children: Vec<&Element> =
        root.first_child(SHARED_ELEMENT).map(|s| s.child_elements().collect()).unwrap_or_default();

    // Pass 1: placeholders for every pool entry so forward references resolve.
    let mut placeholders: Vec<Option<Value>> = Vec::with_capacity(shared_children.len());
    for el in &shared_children {
        let Some(id) = el.attr(ATTR_ID) else {
            placeholders.push(None);
            continue;
        };
        let handle = if is_builtin_tag(&el.name) {
            Value::Shared(Rc::new(RefCell::new(Value::Null)))
        } else {
            Value::Object(Object::new(el.name.clone()).into_ref())
        };
        ctx.refs.insert(id.to_string(), handle.clone());
        placeholders.push(Some(handle));
    }

    // Pass 2: fill them in.
    let mut shared = Vec::with_capacity(shared_children.len());
    for (el, handle) in shared_children.iter().zip(placeholders) {
        let index = el.attr(ATTR_INDEX).and_then(|s| s.parse().ok()).unwrap_or(-1);
        let value = match handle {
            None => {
                ctx.diagnostics.push(format!("shared <{}> without xs.id kept verbatim", el.name));
                Value::Raw((*el).clone())
            }
            Some(Value::Object(o)) => match fill_object(&o, el, &mut ctx) {
                Ok(()) => Value::Object(o),
                Err(e) => {
                    ctx.diagnostics.push(format!("shared <{}>: {e}; kept verbatim", el.name));
                    Value::Raw((*el).clone())
                }
            },
            Some(Value::Shared(s)) => {
                let v = parse_builtin(el, &mut ctx);
                *s.borrow_mut() = v;
                Value::Shared(s)
            }
            Some(_) => unreachable!(),
        };
        shared.push(SharedEntry {
            id: el.attr(ATTR_ID).unwrap_or("").to_string(),
            index,
            tag: el.name.clone(),
            value,
        });
    }

    let main_child = root.first_child(MAIN_ELEMENT).and_then(|m| m.child_elements().next());
    let root_value = main_child.map(|el| parse_value(el, &mut ctx));

    Ok(ModelGraph {
        root: root_value,
        shared,
        prolog: doc.prolog.clone(),
        epilog: doc.epilog.clone(),
        root_attrs: root.attrs.clone(),
        layout: doc.layout.clone(),
        diagnostics: ctx.diagnostics,
    })
}

fn is_meta_attr(name: &str) -> bool {
    matches!(name, ATTR_NAME | ATTR_ID | ATTR_REF | ATTR_INDEX)
}

fn fill_object(obj: &ObjRef, el: &Element, ctx: &mut ReadCtx) -> Result<()> {
    let mut o = obj.borrow_mut();
    o.tag = el.name.clone();
    o.attrs = el.attrs.iter().filter(|(k, _)| !is_meta_attr(k)).cloned().collect();
    o.fields.clear();
    let mut text = String::new();
    for node in &el.children {
        match node {
            Node::Element(child) => {
                let name = child.attr(ATTR_NAME).map(str::to_string);
                let value = parse_value(child, ctx);
                o.fields.push(Field { name, value });
            }
            Node::Text(t) => text.push_str(t),
        }
    }
    o.text = if text.is_empty() { None } else { Some(text) };
    Ok(())
}

/// Materializes an element: `<null>`, a resolved `xs.ref`, a builtin, or a class object.
fn parse_value(el: &Element, ctx: &mut ReadCtx) -> Value {
    if el.name == TAG_NULL {
        return Value::Null;
    }
    if let Some(id) = el.attr(ATTR_REF) {
        return match ctx.refs.get(id) {
            Some(v) => v.clone(),
            None => {
                ctx.diagnostics.push(format!("unresolved xs.ref=\"{id}\" on <{}>", el.name));
                Value::DanglingRef { tag: el.name.clone(), id: id.to_string() }
            }
        };
    }
    if is_builtin_tag(&el.name) {
        let v = parse_builtin(el, ctx);
        if let Some(id) = el.attr(ATTR_ID) {
            // An inline xs.id on a builtin: wrap so later references share it.
            let shared = Value::Shared(Rc::new(RefCell::new(v)));
            ctx.refs.insert(id.to_string(), shared.clone());
            return shared;
        }
        return v;
    }

    let obj = Object::new(el.name.clone()).into_ref();
    if let Some(id) = el.attr(ATTR_ID) {
        ctx.refs.insert(id.to_string(), Value::Object(obj.clone()));
    }
    match fill_object(&obj, el, ctx) {
        Ok(()) => Value::Object(obj),
        Err(e) => {
            ctx.diagnostics.push(format!("<{}>: {e}; kept verbatim", el.name));
            Value::Raw(el.clone())
        }
    }
}

/// Decodes a builtin-tag element; a parse failure keeps the element verbatim (reported).
fn parse_builtin(el: &Element, ctx: &mut ReadCtx) -> Value {
    match try_parse_builtin(el, ctx) {
        Ok(v) => v,
        Err(e) => {
            ctx.diagnostics.push(format!(
                "<{} xs.n=\"{}\">: {e}; kept verbatim",
                el.name,
                el.attr(ATTR_NAME).unwrap_or("(item)")
            ));
            Value::Raw(el.clone())
        }
    }
}

fn tokens(el: &Element) -> Vec<String> {
    let text = el.text();
    let trimmed = xml::trim_ascii_control(&text);
    if trimmed.is_empty() { Vec::new() } else { trimmed.split(' ').map(str::to_string).collect() }
}

fn try_parse_builtin(el: &Element, ctx: &mut ReadCtx) -> Result<Value> {
    let text = || el.text();
    Ok(match el.name.as_str() {
        "i" => Value::Int(numfmt::parse_int(&text(), "int")?),
        "f" => Value::Float(numfmt::parse_float(&text())?),
        "d" => Value::Double(numfmt::parse_double(&text())?),
        "l" => Value::Long(numfmt::parse_int(&text(), "long")?),
        "short" => Value::Short(numfmt::parse_int(&text(), "short")?),
        "byte" => Value::Byte(numfmt::parse_int(&text(), "byte")?),
        "char" => match text().chars().next() {
            Some(c) => Value::Char(c),
            None => bail!("empty char"),
        },
        "b" => Value::Bool(numfmt::parse_bool(&text())),
        "s" => Value::Str(text()),
        "float-array" => Value::FloatArray(
            tokens(el).iter().map(|t| numfmt::parse_float(t)).collect::<Result<_>>()?,
        ),
        "int-array" => Value::IntArray(
            tokens(el).iter().map(|t| numfmt::parse_int(t, "int")).collect::<Result<_>>()?,
        ),
        "long-array" => Value::LongArray(
            tokens(el).iter().map(|t| numfmt::parse_int(t, "long")).collect::<Result<_>>()?,
        ),
        "short-array" => Value::ShortArray(
            tokens(el).iter().map(|t| numfmt::parse_int(t, "short")).collect::<Result<_>>()?,
        ),
        "double-array" => Value::DoubleArray(
            tokens(el).iter().map(|t| numfmt::parse_double(t)).collect::<Result<_>>()?,
        ),
        "byte-array" => Value::ByteArray(
            tokens(el).iter().map(|t| numfmt::parse_int(t, "byte")).collect::<Result<_>>()?,
        ),
        "char-array" => Value::CharArray(
            tokens(el)
                .iter()
                .map(|t| t.chars().next().ok_or_else(|| crate::Error::new("empty char token")))
                .collect::<Result<_>>()?,
        ),
        "bool-array" => {
            Value::BoolArray(tokens(el).iter().map(|t| numfmt::parse_bool(t)).collect())
        }
        "array_list" => Value::List {
            kind: ListKind::ArrayList,
            items: el.child_elements().map(|c| parse_value(c, ctx)).collect(),
        },
        "carray_list" => Value::List {
            kind: ListKind::CArrayList,
            items: el.child_elements().map(|c| parse_value(c, ctx)).collect(),
        },
        "linked_set" => Value::Set(el.child_elements().map(|c| parse_value(c, ctx)).collect()),
        "hash_map" => parse_map(el, MapKind::HashMap, ctx),
        "linked_map" => parse_map(el, MapKind::LinkedMap, ctx),
        TAG_FILE => match el.attr("path") {
            Some(p) => Value::File { path: Some(p.to_string()), text: None },
            None => Value::File { path: None, text: Some(text()) },
        },
        other => bail!("not a builtin tag: {other}"),
    })
}

fn parse_map(el: &Element, kind: MapKind, ctx: &mut ReadCtx) -> Value {
    let mut entries = Vec::new();
    if el.attr(ATTR_KEY_TYPE) == Some("string") {
        for child in el.child_elements() {
            let key = match child.attr(ATTR_NAME) {
                Some(k) => Value::Str(k.to_string()),
                None => Value::Null,
            };
            entries.push((key, parse_value(child, ctx)));
        }
    } else {
        for entry in el.child_elements() {
            if entry.name != TAG_ENTRY {
                continue;
            }
            let key = entry
                .child_elements()
                .find(|c| c.attr(ATTR_NAME) == Some(NAME_KEY))
                .map(|c| parse_value(c, ctx))
                .unwrap_or(Value::Null);
            let value = entry
                .child_elements()
                .find(|c| c.attr(ATTR_NAME) == Some(NAME_VALUE))
                .map(|c| parse_value(c, ctx))
                .unwrap_or(Value::Null);
            entries.push((key, value));
        }
    }
    let key_type_first = el.attrs.iter().position(|(k, _)| k == ATTR_KEY_TYPE)
        < el.attrs.iter().position(|(k, _)| k == ATTR_COUNT);
    Value::Map { kind, entries, key_type_first }
}

// ---------------------------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------------------------

/// A mutable element under construction. The hoist step has to re-stamp attributes on elements
/// already placed in the tree, which an owned [`Element`] tree cannot express.
type BElem = Rc<RefCell<Builder>>;

#[derive(Default)]
struct Builder {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<BNode>,
}

enum BNode {
    El(BElem),
    Text(String),
}

impl Builder {
    fn new(name: &str) -> BElem {
        Rc::new(RefCell::new(Builder { name: name.to_string(), ..Default::default() }))
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn set_attr(&mut self, name: &str, value: impl Into<String>) {
        let value = value.into();
        match self.attrs.iter_mut().find(|(k, _)| k == name) {
            Some(slot) => slot.1 = value,
            None => self.attrs.push((name.to_string(), value)),
        }
    }

    fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|(k, _)| k != name);
    }

    fn into_element(self) -> Element {
        Element {
            name: self.name,
            attrs: self.attrs,
            children: self
                .children
                .into_iter()
                .map(|c| match c {
                    BNode::El(e) => Node::Element(
                        Rc::try_unwrap(e)
                            .map(RefCell::into_inner)
                            .unwrap_or_else(|rc| rc.borrow().clone_shallow())
                            .into_element(),
                    ),
                    BNode::Text(t) => Node::Text(t),
                })
                .collect(),
        }
    }

    fn clone_shallow(&self) -> Builder {
        Builder {
            name: self.name.clone(),
            attrs: self.attrs.clone(),
            children: self
                .children
                .iter()
                .map(|c| match c {
                    BNode::El(e) => BNode::El(Rc::new(RefCell::new(e.borrow().clone_shallow()))),
                    BNode::Text(t) => BNode::Text(t.clone()),
                })
                .collect(),
        }
    }

    fn from_element(el: &Element) -> BElem {
        let b = Builder::new(&el.name);
        {
            let mut bm = b.borrow_mut();
            bm.attrs = el.attrs.clone();
            for c in &el.children {
                bm.children.push(match c {
                    Node::Element(e) => BNode::El(Builder::from_element(e)),
                    Node::Text(t) => BNode::Text(t.clone()),
                });
            }
        }
        b
    }
}

fn set_field_name(el: &BElem, name: Option<&str>) {
    if let Some(n) = name {
        el.borrow_mut().set_attr(ATTR_NAME, n);
    }
}

struct WrittenRecord {
    el: BElem,
    index: usize,
}

struct SharedDef {
    el: BElem,
    index: usize,
}

struct WriteCtx {
    float_style: xml::FloatStyle,
    /// Identity -> (xs.id, def tag) of the preserved pool (reconcile mode).
    preassigned: HashMap<*const (), (String, String)>,
    written: HashMap<*const (), WrittenRecord>,
    shared_defs: Vec<SharedDef>,
    ref_id_counter: i64,
    written_index_counter: usize,
}

impl WriteCtx {
    fn new(
        float_style: xml::FloatStyle,
        preassigned: HashMap<*const (), (String, String)>,
        first_fresh_ref_id: i64,
    ) -> Self {
        WriteCtx {
            float_style,
            preassigned,
            written: HashMap::new(),
            shared_defs: Vec::new(),
            ref_id_counter: first_fresh_ref_id,
            written_index_counter: 0,
        }
    }

    /// Records a value's element on first write; later calls are no-ops, matching the editor.
    fn set_written(&mut self, ptr: *const (), el: &BElem) {
        if !self.written.contains_key(&ptr) {
            let index = self.written_index_counter;
            self.written_index_counter += 1;
            self.written.insert(ptr, WrittenRecord { el: el.clone(), index });
        }
    }

    /// `<null>`, a reference (preserved id, or hoisting a repeated object), or the value's element.
    fn create_element_from_object(&mut self, name: Option<&str>, value: &Value) -> BElem {
        if let Value::Null = value {
            let el = Builder::new(TAG_NULL);
            set_field_name(&el, name);
            return el;
        }

        if let Some(ptr) = value.identity() {
            if let Some((id, tag)) = self.preassigned.get(&ptr).cloned() {
                let el = Builder::new(&tag);
                set_field_name(&el, name);
                el.borrow_mut().set_attr(ATTR_REF, id);
                return el;
            }
            if let Some(record) = self.written.get(&ptr) {
                let def = record.el.clone();
                let index = record.index;
                let existing = def.borrow().attr(ATTR_ID).map(str::to_string);
                let id = match existing {
                    Some(id) => id,
                    None => {
                        let id = format!("#{}", self.ref_id_counter);
                        self.ref_id_counter += 1;
                        def.borrow_mut().set_attr(ATTR_ID, id.clone());
                        self.shared_defs.push(SharedDef { el: def.clone(), index });
                        id
                    }
                };
                let el = Builder::new(&def.borrow().name);
                set_field_name(&el, name);
                el.borrow_mut().set_attr(ATTR_REF, id);
                return el;
            }
        }

        match value {
            Value::Raw(raw) => {
                let el = Builder::from_element(raw);
                match name {
                    Some(n) => el.borrow_mut().set_attr(ATTR_NAME, n),
                    None => el.borrow_mut().remove_attr(ATTR_NAME),
                }
                el
            }
            Value::DanglingRef { tag, id } => {
                let el = Builder::new(tag);
                set_field_name(&el, name);
                el.borrow_mut().set_attr(ATTR_REF, id.clone());
                el
            }
            _ => self.create_def_element(name, value),
        }
    }

    /// The value's own element (never a reference), registering identity for hoisting.
    fn create_def_element(&mut self, name: Option<&str>, value: &Value) -> BElem {
        match value {
            Value::Object(o) => {
                let obj = o.borrow();
                let el = Builder::new(&obj.tag);
                set_field_name(&el, name);
                self.set_written(Rc::as_ptr(o) as *const (), &el); // before fields, so cycles resolve
                for (k, v) in &obj.attrs {
                    el.borrow_mut().attrs.push((k.clone(), v.clone()));
                }
                for field in &obj.fields {
                    let child =
                        self.create_element_from_object(field.name.as_deref(), &field.value);
                    el.borrow_mut().children.push(BNode::El(child));
                }
                if let Some(t) = &obj.text {
                    el.borrow_mut().children.push(BNode::Text(t.clone()));
                }
                el
            }
            Value::Shared(s) => {
                let inner = s.borrow();
                let el = self.create_def_element(name, &inner);
                self.set_written(Rc::as_ptr(s) as *const (), &el);
                el
            }
            Value::List { kind, items } => {
                let el = Builder::new(kind.tag());
                set_field_name(&el, name);
                el.borrow_mut().set_attr(ATTR_COUNT, items.len().to_string());
                for item in items {
                    let child = self.create_element_from_object(None, item);
                    el.borrow_mut().children.push(BNode::El(child));
                }
                el
            }
            Value::Set(items) => {
                let el = Builder::new("linked_set");
                set_field_name(&el, name);
                el.borrow_mut().set_attr(ATTR_COUNT, items.len().to_string());
                for item in items {
                    let child = self.create_element_from_object(None, item);
                    el.borrow_mut().children.push(BNode::El(child));
                }
                el
            }
            Value::Map { kind, entries, key_type_first } => {
                let el = Builder::new(kind.tag());
                set_field_name(&el, name);
                // String-keyed maps carry keyType="string" and store each value as <V xs.n="key">;
                // object-keyed maps wrap each pair in <entry><key/><value/></entry>.
                let string_keyed =
                    entries.is_empty() || entries.iter().all(|(k, _)| matches!(k, Value::Str(_)));
                if string_keyed && *key_type_first {
                    el.borrow_mut().set_attr(ATTR_KEY_TYPE, "string");
                }
                el.borrow_mut().set_attr(ATTR_COUNT, entries.len().to_string());
                if string_keyed {
                    if !*key_type_first {
                        el.borrow_mut().set_attr(ATTR_KEY_TYPE, "string");
                    }
                    for (k, v) in entries {
                        let key = k.as_str().unwrap_or("");
                        let child = self.create_element_from_object(Some(key), v);
                        el.borrow_mut().children.push(BNode::El(child));
                    }
                } else {
                    for (k, v) in entries {
                        let entry = Builder::new(TAG_ENTRY);
                        let key = self.create_element_from_object(Some(NAME_KEY), k);
                        let val = self.create_element_from_object(Some(NAME_VALUE), v);
                        entry.borrow_mut().children.push(BNode::El(key));
                        entry.borrow_mut().children.push(BNode::El(val));
                        el.borrow_mut().children.push(BNode::El(entry));
                    }
                }
                el
            }
            Value::File { path, text } => {
                let el = Builder::new(TAG_FILE);
                set_field_name(&el, name);
                match path {
                    Some(p) => el.borrow_mut().set_attr("path", p.clone()),
                    None => {
                        if let Some(t) = text
                            && !t.is_empty()
                        {
                            el.borrow_mut().children.push(BNode::Text(t.clone()));
                        }
                    }
                }
                el
            }
            Value::Null | Value::Raw(_) | Value::DanglingRef { .. } => {
                self.create_element_from_object(name, value)
            }
            scalar => {
                let el = Builder::new(&scalar.tag());
                set_field_name(&el, name);
                if let Some(count) = array_len(scalar) {
                    el.borrow_mut().set_attr(ATTR_COUNT, count.to_string());
                }
                let text = scalar_text(scalar, self.float_style);
                if !text.is_empty() {
                    el.borrow_mut().children.push(BNode::Text(text));
                }
                el
            }
        }
    }
}

fn array_len(v: &Value) -> Option<usize> {
    Some(match v {
        Value::FloatArray(a) => a.len(),
        Value::IntArray(a) => a.len(),
        Value::LongArray(a) => a.len(),
        Value::ShortArray(a) => a.len(),
        Value::DoubleArray(a) => a.len(),
        Value::ByteArray(a) => a.len(),
        Value::CharArray(a) => a.len(),
        Value::BoolArray(a) => a.len(),
        _ => return None,
    })
}

fn join<T, F: Fn(&T) -> String>(items: &[T], f: F) -> String {
    items.iter().map(f).collect::<Vec<_>>().join(" ")
}

/// The element text of a scalar or primitive array, floats spelled per `style`.
pub fn scalar_text(v: &Value, style: xml::FloatStyle) -> String {
    let f = |x: f32| match style {
        xml::FloatStyle::Java => numfmt::float_to_string(x),
        xml::FloatStyle::Plain => numfmt::float_to_plain_string(x),
    };
    let d = |x: f64| match style {
        xml::FloatStyle::Java => numfmt::double_to_string(x),
        xml::FloatStyle::Plain => numfmt::double_to_plain_string(x),
    };
    match v {
        Value::Int(x) => x.to_string(),
        Value::Float(x) => f(*x),
        Value::Double(x) => d(*x),
        Value::Long(x) => x.to_string(),
        Value::Short(x) => x.to_string(),
        Value::Byte(x) => x.to_string(),
        Value::Char(c) => c.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Str(s) => s.clone(),
        Value::FloatArray(a) => join(a, |x| f(*x)),
        Value::IntArray(a) => join(a, |x| x.to_string()),
        Value::LongArray(a) => join(a, |x| x.to_string()),
        Value::ShortArray(a) => join(a, |x| x.to_string()),
        Value::DoubleArray(a) => join(a, |x| d(*x)),
        Value::ByteArray(a) => join(a, |x| x.to_string()),
        Value::CharArray(a) => join(a, |c| c.to_string()),
        Value::BoolArray(a) => join(a, |b| b.to_string()),
        _ => String::new(),
    }
}

fn first_fresh_ref_id(shared: &[SharedEntry]) -> i64 {
    shared
        .iter()
        .filter_map(|e| e.id.strip_prefix('#').and_then(|n| n.parse::<i64>().ok()))
        .max()
        .map(|m| m + 1)
        .unwrap_or(0)
}

/// Replaces the hoisted def in place by a reference and returns the detached def element.
fn hoist(def: &BElem, ref_id: &str) -> BElem {
    let mut inner = def.borrow_mut();
    let name = inner.name.clone();
    let field_name = inner.attr(ATTR_NAME).map(str::to_string);
    let taken = std::mem::take(&mut *inner);
    inner.name = name;
    if let Some(n) = field_name {
        inner.attrs.push((ATTR_NAME.to_string(), n));
    }
    inner.attrs.push((ATTR_REF.to_string(), ref_id.to_string()));
    drop(inner);
    let detached = Rc::new(RefCell::new(taken));
    detached.borrow_mut().remove_attr(ATTR_NAME);
    detached
}

/// Re-emits a graph read by [`read_model`], reusing the preserved shared ids, order and PIs so an
/// unedited graph round-trips byte-identical.
pub fn write_model(graph: &ModelGraph) -> Document {
    let root = Builder::new(ROOT_ELEMENT);
    root.borrow_mut().attrs = graph.root_attrs.clone();
    let shared_el = Builder::new(SHARED_ELEMENT);
    let main_el = Builder::new(MAIN_ELEMENT);

    let preassigned: HashMap<*const (), (String, String)> = graph
        .shared
        .iter()
        .filter_map(|e| e.value.identity().map(|p| (p, (e.id.clone(), e.tag.clone()))))
        .collect();
    let mut ctx =
        WriteCtx::new(graph.layout.float_style, preassigned, first_fresh_ref_id(&graph.shared));

    if let Some(r) = &graph.root {
        let el = ctx.create_element_from_object(None, r);
        main_el.borrow_mut().children.push(BNode::El(el));
    }

    for entry in &graph.shared {
        let def = match &entry.value {
            Value::Raw(raw) => Builder::from_element(raw),
            other => {
                let el = ctx.create_def_element(None, other);
                el.borrow_mut().set_attr(ATTR_ID, entry.id.clone());
                el.borrow_mut().set_attr(ATTR_INDEX, entry.index.to_string());
                el
            }
        };
        shared_el.borrow_mut().children.push(BNode::El(def));
    }

    // Fresh shared defs (objects reused after the read) continue past the preserved maximum index.
    let next_index = graph.shared.iter().map(|e| e.index).max().map(|m| m + 1).unwrap_or(0);
    let mut defs = std::mem::take(&mut ctx.shared_defs);
    defs.sort_by_key(|d| d.index);
    for (k, def) in defs.into_iter().enumerate() {
        let id = def.el.borrow().attr(ATTR_ID).map(str::to_string).unwrap_or_default();
        let detached = hoist(&def.el, &id);
        detached.borrow_mut().set_attr(ATTR_INDEX, (next_index + k as i32).to_string());
        shared_el.borrow_mut().children.push(BNode::El(detached));
    }

    root.borrow_mut().children.push(BNode::El(shared_el));
    root.borrow_mut().children.push(BNode::El(main_el));

    let root = Rc::try_unwrap(root)
        .map(RefCell::into_inner)
        .unwrap_or_else(|rc| rc.borrow().clone_shallow())
        .into_element();
    Document {
        prolog: graph.prolog.clone(),
        root,
        epilog: graph.epilog.clone(),
        layout: graph.layout.clone(),
    }
}

/// Serializes a never-read root into a bare `<root><shared/><main/></root>` document: shared
/// defs hoisted with `#N` ids and `xs.idx` = write order. No PIs and no root attributes; see
/// [`crate::author`] for the full editor prologue.
pub fn write_root(root_value: &Value) -> Document {
    let root = Builder::new(ROOT_ELEMENT);
    let shared_el = Builder::new(SHARED_ELEMENT);
    let main_el = Builder::new(MAIN_ELEMENT);
    let mut ctx = WriteCtx::new(xml::FloatStyle::Java, HashMap::new(), 0);

    let el = ctx.create_element_from_object(None, root_value);
    main_el.borrow_mut().children.push(BNode::El(el));

    let mut defs = std::mem::take(&mut ctx.shared_defs);
    defs.sort_by_key(|d| d.index);
    for def in defs {
        let id = def.el.borrow().attr(ATTR_ID).map(str::to_string).unwrap_or_default();
        let detached = hoist(&def.el, &id);
        detached.borrow_mut().set_attr(ATTR_INDEX, def.index.to_string());
        shared_el.borrow_mut().children.push(BNode::El(detached));
    }

    root.borrow_mut().children.push(BNode::El(shared_el));
    root.borrow_mut().children.push(BNode::El(main_el));
    let root = Rc::try_unwrap(root)
        .map(RefCell::into_inner)
        .unwrap_or_else(|rc| rc.borrow().clone_shallow())
        .into_element();
    Document::new(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<?version SerializeFormatVersion:2?>\r\n<root fileFormatVersion=\"504000000\">\r\n<shared>\r\n<float-array count=\"2\" xs.id=\"#0\" xs.idx=\"0\">1.0 2.0</float-array>\r\n<CDrawableGuid uuid=\"u\" note=\"n\" xs.id=\"#1\" xs.idx=\"1\" />\r\n</shared>\r\n<main>\r\n<CModelSource isDefaultKeyformLocked=\"false\">\r\n<CDrawableGuid xs.n=\"guid\" xs.ref=\"#1\" />\r\n<s xs.n=\"name\">M</s>\r\n<array_list xs.n=\"items\" count=\"2\">\r\n<float-array xs.ref=\"#0\" />\r\n<CDrawableGuid xs.ref=\"#1\" />\r\n</array_list>\r\n<hash_map xs.n=\"m\" count=\"1\" keyType=\"string\">\r\n<i xs.n=\"k\">3</i>\r\n</hash_map>\r\n<linked_map xs.n=\"om\" count=\"1\">\r\n<entry>\r\n<i xs.n=\"key\">1</i>\r\n<b xs.n=\"value\">true</b>\r\n</entry>\r\n</linked_map>\r\n<file xs.n=\"src\">C:\\a.psd</file>\r\n<file xs.n=\"buf\" path=\"imageFileBuf.png\" />\r\n<null xs.n=\"nothing\" />\r\n<CLabelColorType xs.n=\"lt\" v=\"RED\" />\r\n</CModelSource>\r\n</main>\r\n</root>\r\n\r\n";

    #[test]
    fn read_write_round_trip_is_byte_identical() {
        let doc = xml::parse(DOC.as_bytes()).unwrap();
        let graph = read_model(&doc).unwrap();
        assert!(graph.diagnostics.is_empty(), "{:?}", graph.diagnostics);
        let root = graph.root_object().unwrap();
        assert_eq!(root.borrow().field("name").and_then(|v| v.as_str()), Some("M"));
        let out = xml::write(&write_model(&graph));
        assert_eq!(String::from_utf8(out).unwrap(), DOC);
    }

    #[test]
    fn shared_identity_is_one_object() {
        let doc = xml::parse(DOC.as_bytes()).unwrap();
        let graph = read_model(&doc).unwrap();
        let root = graph.root_object().unwrap();
        let guid = root.borrow().field("guid").cloned().unwrap();
        let items = root.borrow().field("items").cloned().unwrap();
        let second = items.as_list().unwrap()[1].clone();
        assert_eq!(guid.identity(), second.identity());
        // Mutating through one handle is visible through the other.
        if let Value::Object(o) = &guid {
            o.borrow_mut().set_attr("note", "changed");
        }
        if let Value::Object(o) = &second {
            assert_eq!(o.borrow().attr("note"), Some("changed"));
        }
    }

    #[test]
    fn fresh_object_used_twice_is_hoisted() {
        let g = Object::guid("CPartGuid", "x", "y").into_ref();
        let mut root = Object::new("CModelSource");
        root.fields.push(Field { name: Some("a".into()), value: Value::Object(g.clone()) });
        root.fields.push(Field { name: Some("b".into()), value: Value::Object(g) });
        let doc = write_root(&Value::object(root));
        let out = String::from_utf8(xml::write(&doc)).unwrap();
        assert!(out.contains("<shared>\r\n<CPartGuid uuid=\"x\" note=\"y\" xs.id=\"#0\" xs.idx=\"1\" />\r\n</shared>"), "{out}");
        assert!(
            out.contains(
                "<CPartGuid xs.n=\"a\" xs.ref=\"#0\" />\r\n<CPartGuid xs.n=\"b\" xs.ref=\"#0\" />"
            ),
            "{out}"
        );
    }

    #[test]
    fn fresh_shared_after_read_continues_past_preserved_ids() {
        let doc = xml::parse(DOC.as_bytes()).unwrap();
        let graph = read_model(&doc).unwrap();
        let root = graph.root_object().unwrap();
        let fresh = Object::guid("CPartGuid", "p", "q").into_ref();
        root.borrow_mut().set_field("p1", Value::Object(fresh.clone()));
        root.borrow_mut().set_field("p2", Value::Object(fresh));
        let out = String::from_utf8(xml::write(&write_model(&graph))).unwrap();
        assert!(
            out.contains("<CPartGuid uuid=\"p\" note=\"q\" xs.id=\"#2\" xs.idx=\"2\" />"),
            "{out}"
        );
        assert!(
            out.contains(
                "<CPartGuid xs.n=\"p1\" xs.ref=\"#2\" />\r\n<CPartGuid xs.n=\"p2\" xs.ref=\"#2\" />"
            ),
            "{out}"
        );
    }

    #[test]
    fn prune_drops_unreferenced_pool_entries() {
        let doc = xml::parse(DOC.as_bytes()).unwrap();
        let mut graph = read_model(&doc).unwrap();
        let root = graph.root_object().unwrap();
        root.borrow_mut().remove_field("items");
        graph.prune_unreachable_shared();
        // The guid is still referenced by "guid"; the float-array was only used by "items".
        assert_eq!(graph.shared.len(), 1);
        assert_eq!(graph.shared[0].id, "#1");
    }
}
