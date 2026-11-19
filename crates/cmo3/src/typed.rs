//! Hand-written support for the generated typed layer (`typed_gen`): the wrapper trait, the
//! shared `Guid`/`Id`/`FileRef` views, and the field accessors every generated getter calls.

use crate::model::{ListKind, ObjRef, Object, Value};

/// A typed view over one class element.
pub trait Typed: Sized {
    const TAG: &'static str;
    fn from_ref(r: ObjRef) -> Self;
    fn obj(&self) -> &ObjRef;

    /// Wraps `r` only when its tag matches.
    fn try_from_ref(r: &ObjRef) -> Option<Self> {
        if r.borrow().tag == Self::TAG { Some(Self::from_ref(r.clone())) } else { None }
    }

    /// Wraps the object behind a value when the tag matches.
    fn from_value(v: &Value) -> Option<Self> {
        match v {
            Value::Object(o) => Self::try_from_ref(o),
            Value::Shared(s) => Self::from_value(&s.borrow()),
            _ => None,
        }
    }
}

#[macro_export]
macro_rules! impl_typed {
    ($name:ident) => {
        impl $crate::typed::Typed for $name {
            const TAG: &'static str = stringify!($name);
            fn from_ref(r: ObjRef) -> Self {
                Self(r)
            }
            fn obj(&self) -> &ObjRef {
                &self.0
            }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({} fields)", stringify!($name), self.0.borrow().fields.len())
            }
        }
    };
}
pub use impl_typed;

/// Any `<*Guid uuid=".." note=".."/>`.
#[derive(Clone)]
pub struct Guid(pub ObjRef);

impl Guid {
    pub fn new(kind: &str, uuid: &str, note: &str) -> Self {
        Guid(Object::guid(kind, uuid, note).into_ref())
    }

    pub fn kind(&self) -> String {
        self.0.borrow().tag.clone()
    }

    pub fn uuid(&self) -> String {
        self.0.borrow().attr("uuid").unwrap_or("").to_string()
    }

    pub fn note(&self) -> String {
        self.0.borrow().attr("note").unwrap_or("").to_string()
    }

    pub fn set_uuid(&self, uuid: &str) {
        self.0.borrow_mut().set_attr("uuid", uuid);
    }

    pub fn set_note(&self, note: &str) {
        self.0.borrow_mut().set_attr("note", note);
    }
}

impl std::fmt::Debug for Guid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({}, {:?})", self.kind(), self.uuid(), self.note())
    }
}

/// Any `<*Id idstr=".."/>`.
#[derive(Clone)]
pub struct Id(pub ObjRef);

impl Id {
    pub fn new(kind: &str, idstr: &str) -> Self {
        Id(Object::id(kind, idstr).into_ref())
    }

    pub fn kind(&self) -> String {
        self.0.borrow().tag.clone()
    }

    pub fn idstr(&self) -> String {
        self.0.borrow().attr("idstr").unwrap_or("").to_string()
    }

    pub fn set_idstr(&self, idstr: &str) {
        self.0.borrow_mut().set_attr("idstr", idstr);
    }
}

impl std::fmt::Debug for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({})", self.kind(), self.idstr())
    }
}

/// A `<file>` element: an archive entry path or a plain filesystem path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileRef {
    Archive(String),
    Text(String),
}

impl FileRef {
    pub fn into_value(self) -> Value {
        match self {
            FileRef::Archive(p) => Value::File { path: Some(p), text: None },
            FileRef::Text(t) => Value::File { path: None, text: Some(t) },
        }
    }

    pub fn archive_path(&self) -> Option<&str> {
        match self {
            FileRef::Archive(p) => Some(p),
            FileRef::Text(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Accessors used by generated code
// ---------------------------------------------------------------------------------------------

fn unshare(v: Value) -> Value {
    match v {
        Value::Shared(s) => s.borrow().clone(),
        other => other,
    }
}

/// The field's value with any shared wrapper unwrapped; None when absent.
pub fn get(obj: &ObjRef, name: &str) -> Option<Value> {
    obj.borrow().field(name).cloned().map(unshare)
}

pub fn set(obj: &ObjRef, name: &str, value: Value) {
    obj.borrow_mut().set_field(name, value);
}

pub fn set_object(obj: &ObjRef, name: &str, target: &ObjRef) {
    set(obj, name, Value::Object(target.clone()));
}

pub fn attr(obj: &ObjRef, name: &str) -> Option<String> {
    obj.borrow().attr(name).map(str::to_string)
}

pub fn set_attr(obj: &ObjRef, name: &str, value: &str) {
    obj.borrow_mut().set_attr(name, value);
}

pub fn get_class<T: Typed>(obj: &ObjRef, name: &str) -> Option<T> {
    obj.borrow().field(name).and_then(T::from_value)
}

pub fn get_guid(obj: &ObjRef, name: &str) -> Option<Guid> {
    match get(obj, name)? {
        Value::Object(o) if o.borrow().tag.ends_with("Guid") => Some(Guid(o)),
        _ => None,
    }
}

pub fn get_id(obj: &ObjRef, name: &str) -> Option<Id> {
    match get(obj, name)? {
        Value::Object(o) if o.borrow().tag.ends_with("Id") => Some(Id(o)),
        _ => None,
    }
}

pub fn get_file(obj: &ObjRef, name: &str) -> Option<FileRef> {
    match get(obj, name)? {
        Value::File { path: Some(p), .. } => Some(FileRef::Archive(p)),
        Value::File { path: None, text } => Some(FileRef::Text(text.unwrap_or_default())),
        _ => None,
    }
}

pub fn get_list(obj: &ObjRef, name: &str) -> Vec<Value> {
    match get(obj, name) {
        Some(Value::List { items, .. }) | Some(Value::Set(items)) => items,
        _ => Vec::new(),
    }
}

pub fn get_list_typed<T: Typed>(obj: &ObjRef, name: &str) -> Vec<T> {
    get_list(obj, name).iter().filter_map(T::from_value).collect()
}

/// Appends to the list field, creating an `array_list` when the field is absent or `<null>`.
pub fn push_list(obj: &ObjRef, name: &str, value: Value) {
    let mut o = obj.borrow_mut();
    match o.field_mut(name) {
        Some(Value::List { items, .. }) | Some(Value::Set(items)) => items.push(value),
        Some(Value::Shared(s)) => {
            if let Value::List { items, .. } | Value::Set(items) = &mut *s.borrow_mut() {
                items.push(value);
            }
        }
        Some(slot @ Value::Null) => {
            *slot = Value::List { kind: ListKind::ArrayList, items: vec![value] }
        }
        Some(_) => {}
        None => o.set_field(name, Value::List { kind: ListKind::ArrayList, items: vec![value] }),
    }
}

pub fn new_object(tag: &str) -> ObjRef {
    Object::new(tag).into_ref()
}

pub fn new_enum(tag: &str, constant: &str) -> ObjRef {
    Object::enum_value(tag, constant).into_ref()
}

pub fn enum_constant(obj: &ObjRef) -> Option<String> {
    obj.borrow().attr("v").map(str::to_string)
}

pub fn set_enum_constant(obj: &ObjRef, constant: &str) {
    obj.borrow_mut().set_attr("v", constant);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed_gen::{CModelSource, CParameterSource, Type};
    use crate::{model, xml};

    const DOC: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<root fileFormatVersion=\"504000000\">\r\n<shared />\r\n<main>\r\n<CModelSource isDefaultKeyformLocked=\"false\">\r\n<s xs.n=\"name\">M</s>\r\n<CParameterSourceSet xs.n=\"parameterSourceSet\">\r\n<carray_list xs.n=\"_sources\" count=\"1\">\r\n<CParameterSource>\r\n<CParameterGuid xs.n=\"guid\" uuid=\"u\" note=\"n\" />\r\n<f xs.n=\"minValue\">-30.0</f>\r\n<f xs.n=\"maxValue\">30.0</f>\r\n<CParameterId xs.n=\"id\" idstr=\"ParamAngleX\" />\r\n<Type xs.n=\"paramType\" v=\"NORMAL\" />\r\n<s xs.n=\"name\">Angle X</s>\r\n</CParameterSource>\r\n</carray_list>\r\n</CParameterSourceSet>\r\n</CModelSource>\r\n</main>\r\n</root>\r\n\r\n";

    #[test]
    fn typed_view_reads_and_edits_the_same_graph() {
        let doc = xml::parse(DOC.as_bytes()).unwrap();
        let graph = model::read_model(&doc).unwrap();
        let root = CModelSource::try_from_ref(&graph.root_object().unwrap()).unwrap();
        assert_eq!(root.name().as_deref(), Some("M"));
        let params: Vec<CParameterSource> = root.parameter_source_set().unwrap().sources();
        assert_eq!(params.len(), 1);
        let p = &params[0];
        assert_eq!(p.min_value(), Some(-30.0));
        assert_eq!(p.id().unwrap().idstr(), "ParamAngleX");
        assert_eq!(p.guid().unwrap().uuid(), "u");
        assert_eq!(p.param_type().unwrap().constant().as_deref(), Some(Type::NORMAL));

        p.set_max_value(45.0);
        p.set_name("Angle X (wide)");
        p.id().unwrap().set_idstr("ParamAngleXWide");
        p.param_type().unwrap().set_constant("EXTRA");
        let out = String::from_utf8(xml::write(&model::write_model(&graph))).unwrap();
        assert!(out.contains("<f xs.n=\"maxValue\">45.0</f>"), "{out}");
        assert!(out.contains("<CParameterId xs.n=\"id\" idstr=\"ParamAngleXWide\" />"), "{out}");
        assert!(out.contains("<Type xs.n=\"paramType\" v=\"EXTRA\" />"), "{out}");
        assert!(out.contains("<s xs.n=\"name\">Angle X (wide)</s>"), "{out}");
        // Untouched parts stay byte-identical.
        assert!(out.contains("<f xs.n=\"minValue\">-30.0</f>\r\n"), "{out}");
    }
}
