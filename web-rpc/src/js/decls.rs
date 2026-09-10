//! Collecting the schema containers that get their own Typescript declaration and their own
//! entry in the generated schema table, and validating them.
//!
//! A container is declared when its schema name is a bare identifier, which is true of every
//! `#[derive(Schema)]` type and false of the std generics postcard-schema names `"Vec<T>"`,
//! `"Result<T, E>"`, `"Box<T>"`, `"[T]"` and so on. Those are always rendered inline.

use postcard_schema::schema::{DataModelType, DataModelVariant, NamedType, NamedValue};

use super::writer::{is_ident, str_eq};
use crate::describe::{Desc, Return, Service};

/// The number of distinct containers one endpoint may reach.
pub const MAX_DECLARATIONS: usize = 128;

/// How deep [`named_type_eq`] compares before assuming two schemas are equal. Only reached by
/// recursive types, where the comparison would otherwise not terminate.
const EQUALITY_DEPTH: usize = 32;

/// Names the generated `.d.ts` declares itself, which a user type may not take.
const RESERVED_TYPE_NAMES: &[&str] = &["Request", "Subscription", "Endpoint"];

const PLACEHOLDER: NamedType = NamedType {
    name: "",
    ty: &DataModelType::Unit,
};

/// The containers reachable from one endpoint, in the order they were first seen.
pub struct Declarations {
    /// The containers; only the first [`Declarations::length`] entries are meaningful.
    pub items: [&'static NamedType; MAX_DECLARATIONS],
    /// How many containers were found.
    pub length: usize,
}

impl Declarations {
    const fn new() -> Self {
        Self {
            items: [&PLACEHOLDER; MAX_DECLARATIONS],
            length: 0,
        }
    }

    const fn get(&self, name: &str) -> Option<&'static NamedType> {
        let mut index = 0;
        while index < self.length {
            if str_eq(self.items[index].name, name) {
                return Some(self.items[index]);
            }
            index += 1;
        }
        None
    }

    const fn push(&mut self, named_type: &'static NamedType) {
        if self.length == MAX_DECLARATIONS {
            panic!(
                "web_rpc: this endpoint reaches more distinct types than the renderer supports; \
                 raise web_rpc::js::MAX_DECLARATIONS"
            );
        }
        self.items[self.length] = named_type;
        self.length += 1;
    }
}

/// True if this container is rendered under its own name rather than inline.
pub const fn is_declared(named_type: &NamedType) -> bool {
    if !is_ident(named_type.name) {
        return false;
    }
    matches!(
        named_type.ty,
        DataModelType::Struct(_) | DataModelType::Enum(_)
    )
}

/// Structural comparison of two schemas. Bounded in depth: two schemas still equal at the
/// limit are taken to be equal.
pub const fn named_type_eq(left: &NamedType, right: &NamedType, depth: usize) -> bool {
    if depth == 0 {
        return true;
    }
    str_eq(left.name, right.name) && type_eq(left.ty, right.ty, depth - 1)
}

const fn named_types_eq(
    left: &[&'static NamedType],
    right: &[&'static NamedType],
    depth: usize,
) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if !named_type_eq(left[index], right[index], depth) {
            return false;
        }
        index += 1;
    }
    true
}

const fn fields_eq(
    left: &[&'static NamedValue],
    right: &[&'static NamedValue],
    depth: usize,
) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if !str_eq(left[index].name, right[index].name)
            || !named_type_eq(left[index].ty, right[index].ty, depth)
        {
            return false;
        }
        index += 1;
    }
    true
}

const fn type_eq(left: &DataModelType, right: &DataModelType, depth: usize) -> bool {
    if depth == 0 {
        return true;
    }
    let depth = depth - 1;
    match (left, right) {
        (DataModelType::Option(left), DataModelType::Option(right))
        | (DataModelType::NewtypeStruct(left), DataModelType::NewtypeStruct(right))
        | (DataModelType::Seq(left), DataModelType::Seq(right)) => {
            named_type_eq(left, right, depth)
        }
        (DataModelType::Tuple(left), DataModelType::Tuple(right))
        | (DataModelType::TupleStruct(left), DataModelType::TupleStruct(right)) => {
            named_types_eq(left, right, depth)
        }
        (
            DataModelType::Map {
                key: left_key,
                val: left_value,
            },
            DataModelType::Map {
                key: right_key,
                val: right_value,
            },
        ) => {
            named_type_eq(left_key, right_key, depth)
                && named_type_eq(left_value, right_value, depth)
        }
        (DataModelType::Struct(left), DataModelType::Struct(right)) => {
            fields_eq(left, right, depth)
        }
        (DataModelType::Enum(left), DataModelType::Enum(right)) => {
            if left.len() != right.len() {
                return false;
            }
            let mut index = 0;
            while index < left.len() {
                if !str_eq(left[index].name, right[index].name)
                    || !variant_eq(left[index].ty, right[index].ty, depth)
                {
                    return false;
                }
                index += 1;
            }
            true
        }
        // The remaining variants carry no payload, so discriminant equality is enough.
        (left, right) => type_rank(left) == type_rank(right),
    }
}

const fn variant_eq(left: &DataModelVariant, right: &DataModelVariant, depth: usize) -> bool {
    match (left, right) {
        (DataModelVariant::UnitVariant, DataModelVariant::UnitVariant) => true,
        (DataModelVariant::NewtypeVariant(left), DataModelVariant::NewtypeVariant(right)) => {
            named_type_eq(left, right, depth)
        }
        (DataModelVariant::TupleVariant(left), DataModelVariant::TupleVariant(right)) => {
            named_types_eq(left, right, depth)
        }
        (DataModelVariant::StructVariant(left), DataModelVariant::StructVariant(right)) => {
            fields_eq(left, right, depth)
        }
        _ => false,
    }
}

/// A discriminant for the payload-free data model types.
const fn type_rank(ty: &DataModelType) -> usize {
    match ty {
        DataModelType::Bool => 0,
        DataModelType::I8 => 1,
        DataModelType::U8 => 2,
        DataModelType::I16 => 3,
        DataModelType::I32 => 4,
        DataModelType::I64 => 5,
        DataModelType::I128 => 6,
        DataModelType::U16 => 7,
        DataModelType::U32 => 8,
        DataModelType::U64 => 9,
        DataModelType::U128 => 10,
        DataModelType::Usize => 11,
        DataModelType::Isize => 12,
        DataModelType::F32 => 13,
        DataModelType::F64 => 14,
        DataModelType::Char => 15,
        DataModelType::String => 16,
        DataModelType::ByteArray => 17,
        DataModelType::Unit => 18,
        DataModelType::UnitStruct => 19,
        DataModelType::Schema => 20,
        DataModelType::Option(_) => 21,
        DataModelType::NewtypeStruct(_) => 22,
        DataModelType::Seq(_) => 23,
        DataModelType::Tuple(_) => 24,
        DataModelType::TupleStruct(_) => 25,
        DataModelType::Map { .. } => 26,
        DataModelType::Struct(_) => 27,
        DataModelType::Enum(_) => 28,
    }
}

/// The checks a declared container must pass. The panics cannot name the offending type; the
/// const-eval backtrace points at the endpoint.
const fn validate(named_type: &'static NamedType) {
    let mut index = 0;
    while index < RESERVED_TYPE_NAMES.len() {
        if str_eq(named_type.name, RESERVED_TYPE_NAMES[index]) {
            panic!(
                "web_rpc: a type reachable from a Javascript endpoint is named `Request`, \
                 `Subscription` or `Endpoint`, which the generated declarations reserve; rename \
                 it or give it a #[serde(rename = \"...\")]"
            );
        }
        index += 1;
    }
    if let DataModelType::Enum(variants) = named_type.ty {
        let mut variant = 0;
        while variant < variants.len() {
            if let DataModelVariant::StructVariant(fields) = variants[variant].ty {
                let mut field = 0;
                while field < fields.len() {
                    if str_eq(fields[field].name, "tag") {
                        panic!(
                            "web_rpc: a struct variant of an enum reachable from a Javascript \
                             endpoint has a field named `tag`, which collides with the \
                             discriminant of the generated Typescript union; rename it or give \
                             it a #[serde(rename = \"...\")]"
                        );
                    }
                    field += 1;
                }
            }
            variant += 1;
        }
    }
}

const fn collect_named_type(declarations: &mut Declarations, named_type: &'static NamedType) {
    if is_declared(named_type) {
        match declarations.get(named_type.name) {
            Some(existing) => {
                if !named_type_eq(existing, named_type, EQUALITY_DEPTH) {
                    panic!(
                        "web_rpc: two different types reachable from this endpoint render to the \
                         same Typescript name; rename one in Rust or give it a \
                         #[serde(rename = \"...\")]"
                    );
                }
                // Already collected, and its children with it.
                return;
            }
            None => {
                validate(named_type);
                declarations.push(named_type);
            }
        }
    }
    collect_type(declarations, named_type.ty);
}

const fn collect_named_types(declarations: &mut Declarations, list: &'static [&'static NamedType]) {
    let mut index = 0;
    while index < list.len() {
        collect_named_type(declarations, list[index]);
        index += 1;
    }
}

const fn collect_fields(declarations: &mut Declarations, fields: &'static [&'static NamedValue]) {
    let mut index = 0;
    while index < fields.len() {
        collect_named_type(declarations, fields[index].ty);
        index += 1;
    }
}

const fn collect_type(declarations: &mut Declarations, ty: &'static DataModelType) {
    match ty {
        DataModelType::Option(inner)
        | DataModelType::NewtypeStruct(inner)
        | DataModelType::Seq(inner) => collect_named_type(declarations, inner),
        DataModelType::Tuple(list) | DataModelType::TupleStruct(list) => {
            collect_named_types(declarations, list)
        }
        DataModelType::Map { key, val } => {
            collect_named_type(declarations, key);
            collect_named_type(declarations, val);
        }
        DataModelType::Struct(fields) => collect_fields(declarations, fields),
        DataModelType::Enum(variants) => {
            let mut index = 0;
            while index < variants.len() {
                match variants[index].ty {
                    DataModelVariant::UnitVariant => {}
                    DataModelVariant::NewtypeVariant(inner) => {
                        collect_named_type(declarations, inner)
                    }
                    DataModelVariant::TupleVariant(list) => collect_named_types(declarations, list),
                    DataModelVariant::StructVariant(fields) => collect_fields(declarations, fields),
                }
                index += 1;
            }
        }
        _ => {}
    }
}

const fn collect_desc(declarations: &mut Declarations, desc: &'static Desc) {
    match desc {
        Desc::Js { .. } => {}
        Desc::Postcard(named_type) | Desc::Inline(named_type) => {
            collect_named_type(declarations, named_type)
        }
        Desc::Option(inner) => collect_desc(declarations, inner),
        Desc::Result(ok, err) => {
            collect_desc(declarations, ok);
            collect_desc(declarations, err);
        }
    }
}

const fn collect_service(declarations: &mut Declarations, service: &'static Service) {
    let mut group = 0;
    while group < service.methods.len() {
        let methods = service.methods[group];
        let mut index = 0;
        while index < methods.len() {
            let method = &methods[index];
            let mut argument = 0;
            while argument < method.args.len() {
                collect_desc(declarations, method.args[argument].desc);
                argument += 1;
            }
            match method.ret {
                Return::Notify => {}
                Return::Value(desc) | Return::Stream(desc) => collect_desc(declarations, desc),
            }
            index += 1;
        }
        group += 1;
    }
}

/// Collect and validate every container reachable from an endpoint's traits.
pub const fn collect(endpoint: &super::Endpoint) -> Declarations {
    let mut declarations = Declarations::new();
    if let Some(service) = endpoint.service {
        collect_service(&mut declarations, service);
    }
    if let Some(client) = endpoint.client {
        collect_service(&mut declarations, client);
    }
    declarations
}
