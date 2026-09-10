//! The Javascript renderer.
//!
//! The generated module is the shell followed by data: one schema value per declared type,
//! one method table per trait, and a class whose methods forward to the shell with a method
//! index and an argument list. The shapes of the values are documented on `Codec` and
//! `Endpoint` in `js/endpoint.mjs`.

use postcard_schema::schema::{DataModelType, DataModelVariant, NamedType, NamedValue};

use super::{
    decls::{collect, is_declared},
    method_at, method_count,
    writer::Output,
    Endpoint, SHELL,
};
use crate::describe::{Desc, Method, Return, Service};

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------

/// A schema reference: the type's name if it is declared, otherwise its body inline.
const fn schema_reference<const CAPACITY: usize>(
    output: &mut Output<CAPACITY>,
    named_type: &'static NamedType,
) {
    if is_declared(named_type) {
        output.put("{ kind: \"ref\", name: ");
        output.put_quoted(named_type.name);
        output.put(" }");
    } else {
        schema_body(output, named_type);
    }
}

const fn schema_list<const CAPACITY: usize>(
    output: &mut Output<CAPACITY>,
    list: &'static [&'static NamedType],
) {
    output.put("[");
    let mut index = 0;
    while index < list.len() {
        if index > 0 {
            output.put(", ");
        }
        schema_reference(output, list[index]);
        index += 1;
    }
    output.put("]");
}

const fn schema_fields<const CAPACITY: usize>(
    output: &mut Output<CAPACITY>,
    fields: &'static [&'static NamedValue],
) {
    output.put("{ kind: \"struct\", fields: [");
    let mut index = 0;
    while index < fields.len() {
        if index > 0 {
            output.put(", ");
        }
        output.put("[");
        output.put_quoted(fields[index].name);
        output.put(", ");
        schema_reference(output, fields[index].ty);
        output.put("]");
        index += 1;
    }
    output.put("] }");
}

/// The body of a schema, never its name.
const fn schema_body<const CAPACITY: usize>(
    output: &mut Output<CAPACITY>,
    named_type: &'static NamedType,
) {
    match named_type.ty {
        DataModelType::Bool => output.put("\"bool\""),
        DataModelType::U8 => output.put("\"u8\""),
        DataModelType::I8 => output.put("\"i8\""),
        DataModelType::U16 => output.put("\"u16\""),
        DataModelType::I16 => output.put("\"i16\""),
        DataModelType::U32 => output.put("\"u32\""),
        DataModelType::I32 => output.put("\"i32\""),
        // `usize` and `isize` are widened to 64 bits by serde before they reach the wire.
        DataModelType::U64 | DataModelType::Usize => output.put("\"u64\""),
        DataModelType::I64 | DataModelType::Isize => output.put("\"i64\""),
        DataModelType::U128 => output.put("\"u128\""),
        DataModelType::I128 => output.put("\"i128\""),
        DataModelType::F32 => output.put("\"f32\""),
        DataModelType::F64 => output.put("\"f64\""),
        DataModelType::Char => output.put("\"char\""),
        DataModelType::String => output.put("\"string\""),
        DataModelType::ByteArray => output.put("\"bytes\""),
        DataModelType::Unit | DataModelType::UnitStruct => output.put("\"unit\""),
        DataModelType::Schema => panic!(
            "web_rpc: postcard-schema's `Schema` data model type cannot be rendered to Javascript"
        ),
        DataModelType::Option(inner) => {
            output.put("{ kind: \"option\", inner: ");
            schema_reference(output, inner);
            output.put(" }");
        }
        DataModelType::NewtypeStruct(inner) => schema_reference(output, inner),
        DataModelType::Seq(inner) => {
            output.put("{ kind: \"seq\", inner: ");
            schema_reference(output, inner);
            output.put(" }");
        }
        DataModelType::Tuple(list) | DataModelType::TupleStruct(list) => {
            output.put("{ kind: \"tuple\", items: ");
            schema_list(output, list);
            output.put(" }");
        }
        DataModelType::Map { key, val } => {
            output.put("{ kind: \"map\", key: ");
            schema_reference(output, key);
            output.put(", value: ");
            schema_reference(output, val);
            output.put(" }");
        }
        DataModelType::Struct(fields) => schema_fields(output, fields),
        DataModelType::Enum(variants) => {
            output.put("{ kind: \"enum\", variants: [");
            let mut index = 0;
            while index < variants.len() {
                if index > 0 {
                    output.put(", ");
                }
                output.put("[");
                output.put_quoted(variants[index].name);
                output.put(", ");
                match variants[index].ty {
                    DataModelVariant::UnitVariant => output.put("\"unit\""),
                    DataModelVariant::NewtypeVariant(inner)
                    | DataModelVariant::TupleVariant([inner]) => {
                        output.put("{ kind: \"newtype\", inner: ");
                        schema_reference(output, inner);
                        output.put(" }");
                    }
                    DataModelVariant::TupleVariant(list) => {
                        output.put("{ kind: \"tuple\", items: ");
                        schema_list(output, list);
                        output.put(" }");
                    }
                    DataModelVariant::StructVariant(fields) => schema_fields(output, fields),
                }
                output.put("]");
                index += 1;
            }
            output.put("] }");
        }
    }
}

/// The wire description of one argument or return value.
const fn wire_description<const CAPACITY: usize>(
    output: &mut Output<CAPACITY>,
    desc: &'static Desc,
) {
    match desc {
        Desc::Js { transfer, .. } => {
            output.put("{ kind: \"js\", transfer: ");
            output.put(if *transfer { "true" } else { "false" });
            output.put(" }");
        }
        Desc::Postcard(named_type) => {
            output.put("{ kind: \"postcard\", schema: ");
            schema_reference(output, named_type);
            output.put(" }");
        }
        Desc::Inline(named_type) => {
            output.put("{ kind: \"inline\", schema: ");
            schema_reference(output, named_type);
            output.put(" }");
        }
        Desc::Option(inner) => {
            output.put("{ kind: \"option\", inner: ");
            wire_description(output, inner);
            output.put(" }");
        }
        Desc::Result(ok, err) => {
            output.put("{ kind: \"result\", ok: ");
            wire_description(output, ok);
            output.put(", err: ");
            wire_description(output, err);
            output.put(" }");
        }
    }
}

const fn schemas<const CAPACITY: usize>(output: &mut Output<CAPACITY>, endpoint: &Endpoint) {
    let declarations = collect(endpoint);
    output.put("const SCHEMAS = {\n");
    let mut index = 0;
    while index < declarations.length {
        let named_type = declarations.items[index];
        output.indent(1);
        output.put(named_type.name);
        output.put(": ");
        schema_body(output, named_type);
        output.put(",\n");
        index += 1;
    }
    output.put("};\n\n");
}

// ---------------------------------------------------------------------------
// Methods
// ---------------------------------------------------------------------------

/// One entry of a method table.
const fn method_entry<const CAPACITY: usize>(
    output: &mut Output<CAPACITY>,
    method: &'static Method,
) {
    output.indent(1);
    output.put("{ name: ");
    output.put_quoted_ident(method.name);
    output.put(", kind: ");
    match method.ret {
        Return::Value(_) => output.put("\"value\""),
        Return::Notify => output.put("\"notify\""),
        Return::Stream(_) => output.put("\"stream\""),
    }
    output.put(", args: [");
    let mut index = 0;
    while index < method.args.len() {
        if index > 0 {
            output.put(", ");
        }
        wire_description(output, method.args[index].desc);
        index += 1;
    }
    output.put("], returns: ");
    match method.ret {
        Return::Notify => output.put("null"),
        Return::Value(desc) | Return::Stream(desc) => wire_description(output, desc),
    }
    output.put(" },\n");
}

const fn method_table<const CAPACITY: usize>(
    output: &mut Output<CAPACITY>,
    name: &str,
    service: Option<&'static Service>,
) {
    output.put("const ");
    output.put(name);
    output.put(" = [\n");
    if let Some(service) = service {
        let count = method_count(service);
        let mut index = 0;
        while index < count {
            method_entry(output, method_at(service, index));
            index += 1;
        }
    }
    output.put("];\n\n");
}

/// One method of the generated class, forwarding to the shell.
const fn class_method<const CAPACITY: usize>(
    output: &mut Output<CAPACITY>,
    method: &'static Method,
    index: usize,
) {
    output.indent(1);
    output.put_ident(method.name);
    output.put("(");
    let mut argument = 0;
    while argument < method.args.len() {
        if argument > 0 {
            output.put(", ");
        }
        output.put_ident(method.args[argument].name);
        argument += 1;
    }
    if matches!(method.ret, Return::Stream(_)) {
        if !method.args.is_empty() {
            output.put(", ");
        }
        output.put("callback");
    }
    output.put(") {\n");
    output.indent(2);
    match method.ret {
        Return::Notify => output.put("this._notify("),
        Return::Value(_) => output.put("return this._request("),
        Return::Stream(_) => output.put("return this._stream("),
    }
    output.put_usize(index);
    output.put(", [");
    let mut argument = 0;
    while argument < method.args.len() {
        if argument > 0 {
            output.put(", ");
        }
        output.put_ident(method.args[argument].name);
        argument += 1;
    }
    output.put("]");
    if matches!(method.ret, Return::Stream(_)) {
        output.put(", callback");
    }
    output.put(");\n");
    output.indent(1);
    output.put("}\n");
}

/// Render the Javascript module for one endpoint.
pub const fn render<const CAPACITY: usize>(endpoint: &Endpoint) -> Output<CAPACITY> {
    let mut output = Output::<CAPACITY>::new();
    output.put("// Generated by web-rpc. Do not edit.\n\n");
    output.put(SHELL);
    output.put("\n");
    schemas(&mut output, endpoint);
    method_table(&mut output, "METHODS", endpoint.client);
    method_table(&mut output, "HANDLERS", endpoint.service);
    output.put("export class ");
    output.put(endpoint.class);
    output.put(" extends Endpoint {\n");
    output.put("  constructor(options) {\n");
    output.put("    super(options, ");
    output.put_quoted(endpoint.class);
    output.put(", new Codec(SCHEMAS), METHODS, HANDLERS);\n");
    output.put("  }\n");
    if let Some(client) = endpoint.client {
        let count = method_count(client);
        let mut index = 0;
        while index < count {
            class_method(&mut output, method_at(client, index), index);
            index += 1;
        }
    }
    output.put("}\n");
    output
}
