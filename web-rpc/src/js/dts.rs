//! The Typescript renderer.

use postcard_schema::schema::{DataModelType, DataModelVariant, NamedType};

use super::{
    decls::{collect, is_declared},
    method_at, method_count,
    writer::Output,
};
use crate::describe::{Desc, Method, Return, Service};

const fn is_unit(named_type: &NamedType) -> bool {
    matches!(named_type.ty, DataModelType::Unit)
}

const fn is_byte(named_type: &NamedType) -> bool {
    matches!(named_type.ty, DataModelType::U8)
}

const fn is_unit_only(variants: &[&postcard_schema::schema::NamedVariant]) -> bool {
    let mut index = 0;
    while index < variants.len() {
        if !matches!(variants[index].ty, DataModelVariant::UnitVariant) {
            return false;
        }
        index += 1;
    }
    true
}

/// Render a reference to a schema: its name if it is declared, otherwise its body inline.
const fn type_reference<const CAPACITY: usize>(
    output: &mut Output<CAPACITY>,
    named_type: &'static NamedType,
) {
    if is_declared(named_type) {
        output.put(named_type.name);
    } else {
        type_body(output, named_type);
    }
}

const fn type_list<const CAPACITY: usize>(
    output: &mut Output<CAPACITY>,
    list: &'static [&'static NamedType],
) {
    output.put("[");
    let mut index = 0;
    while index < list.len() {
        if index > 0 {
            output.put(", ");
        }
        type_reference(output, list[index]);
        index += 1;
    }
    output.put("]");
}

/// Render the body of a schema, never its name.
const fn type_body<const CAPACITY: usize>(
    output: &mut Output<CAPACITY>,
    named_type: &'static NamedType,
) {
    match named_type.ty {
        DataModelType::Bool => output.put("boolean"),
        DataModelType::I8
        | DataModelType::U8
        | DataModelType::I16
        | DataModelType::U16
        | DataModelType::I32
        | DataModelType::U32
        | DataModelType::F32
        | DataModelType::F64 => output.put("number"),
        // `usize` and `isize` are widened to 64 bits by serde before they reach the wire.
        DataModelType::I64
        | DataModelType::U64
        | DataModelType::I128
        | DataModelType::U128
        | DataModelType::Usize
        | DataModelType::Isize => output.put("bigint"),
        DataModelType::Char | DataModelType::String => output.put("string"),
        DataModelType::ByteArray => output.put("Uint8Array"),
        DataModelType::Unit | DataModelType::UnitStruct => output.put("undefined"),
        DataModelType::Schema => output.put("unknown"),
        DataModelType::Option(inner) => {
            output.put("(");
            type_reference(output, inner);
            output.put(" | undefined)");
        }
        DataModelType::NewtypeStruct(inner) => type_reference(output, inner),
        DataModelType::Seq(inner) => {
            if is_byte(inner) {
                output.put("Uint8Array");
            } else {
                output.put("(");
                type_reference(output, inner);
                output.put(")[]");
            }
        }
        DataModelType::Tuple(list) | DataModelType::TupleStruct(list) => type_list(output, list),
        DataModelType::Map { key, val } => {
            output.put("Map<");
            type_reference(output, key);
            output.put(", ");
            type_reference(output, val);
            output.put(">");
        }
        DataModelType::Struct(fields) => {
            output.put("{ ");
            let mut index = 0;
            while index < fields.len() {
                output.put(fields[index].name);
                output.put(": ");
                type_reference(output, fields[index].ty);
                output.put("; ");
                index += 1;
            }
            output.put("}");
        }
        DataModelType::Enum(variants) => {
            if variants.is_empty() {
                output.put("never");
                return;
            }
            let unit_only = is_unit_only(variants);
            let mut index = 0;
            while index < variants.len() {
                if index > 0 {
                    output.put(" | ");
                }
                let variant = variants[index];
                if unit_only {
                    output.put_quoted(variant.name);
                } else {
                    output.put("{ tag: ");
                    output.put_quoted(variant.name);
                    match variant.ty {
                        DataModelVariant::UnitVariant => {}
                        DataModelVariant::NewtypeVariant(inner)
                        | DataModelVariant::TupleVariant([inner]) => {
                            output.put("; value: ");
                            type_reference(output, inner);
                        }
                        DataModelVariant::TupleVariant(list) => {
                            output.put("; value: ");
                            type_list(output, list);
                        }
                        DataModelVariant::StructVariant(fields) => {
                            let mut field = 0;
                            while field < fields.len() {
                                output.put("; ");
                                output.put(fields[field].name);
                                output.put(": ");
                                type_reference(output, fields[field].ty);
                                field += 1;
                            }
                        }
                    }
                    output.put(" }");
                }
                index += 1;
            }
        }
    }
}

/// Render the Typescript type of one wire value.
const fn desc_type<const CAPACITY: usize>(output: &mut Output<CAPACITY>, desc: &'static Desc) {
    match desc {
        Desc::Js { name, .. } => output.put(name),
        Desc::Postcard(named_type) | Desc::Inline(named_type) => type_reference(output, named_type),
        Desc::Option(inner) => {
            output.put("(");
            desc_type(output, inner);
            output.put(" | undefined)");
        }
        Desc::Result(ok, err) => {
            output.put("({ tag: \"Ok\"; value: ");
            desc_type(output, ok);
            output.put(" } | { tag: \"Err\"; value: ");
            desc_type(output, err);
            output.put(" })");
        }
    }
}

/// Render the Typescript type of the return value of a non-streaming method.
///
/// Two rules apply only here. `()` is `void` rather than `undefined`. And a `Result` is not
/// the discriminated union it is in value position: the promise resolves the `Ok` payload and
/// rejects with the decoded `Err`, and a handler returns the `Ok` payload or throws.
const fn return_type<const CAPACITY: usize>(output: &mut Output<CAPACITY>, desc: &'static Desc) {
    match desc {
        Desc::Postcard(named_type) if is_unit(named_type) => output.put("void"),
        Desc::Result(ok, _) => return_type(output, ok),
        other => desc_type(output, other),
    }
}

/// True when this argument may be rendered as an optional parameter: it is an `Option` and
/// every argument after it is too. A streaming method takes a trailing callback, so its
/// arguments are never optional.
const fn is_optional_argument(method: &'static Method, index: usize) -> bool {
    if matches!(method.ret, Return::Stream(_)) {
        return false;
    }
    let mut position = index;
    while position < method.args.len() {
        if !matches!(method.args[position].desc, Desc::Option(_)) {
            return false;
        }
        position += 1;
    }
    true
}

const fn parameters<const CAPACITY: usize>(output: &mut Output<CAPACITY>, method: &'static Method) {
    let mut index = 0;
    while index < method.args.len() {
        if index > 0 {
            output.put(", ");
        }
        let argument = &method.args[index];
        output.put_ident(argument.name);
        if is_optional_argument(method, index) {
            output.put("?: ");
            // An optional parameter is already `T | undefined`, so unwrap one layer.
            match argument.desc {
                Desc::Option(inner) => desc_type(output, inner),
                other => desc_type(output, other),
            }
        } else {
            output.put(": ");
            desc_type(output, argument.desc);
        }
        index += 1;
    }
}

/// One method of the class this endpoint exposes, which calls the other side.
const fn client_method<const CAPACITY: usize>(
    output: &mut Output<CAPACITY>,
    method: &'static Method,
) {
    output.indent(1);
    output.put_ident(method.name);
    output.put("(");
    parameters(output, method);
    match method.ret {
        Return::Notify => output.put("): void;\n"),
        Return::Value(desc) => {
            output.put("): Request<");
            return_type(output, desc);
            output.put(">;\n");
        }
        Return::Stream(item) => {
            if !method.args.is_empty() {
                output.put(", ");
            }
            output.put("callback: (item: ");
            desc_type(output, item);
            output.put(") => void | Promise<void>): Subscription;\n");
        }
    }
}

/// One method of the handler interface, which this endpoint implements.
const fn handler_method<const CAPACITY: usize>(
    output: &mut Output<CAPACITY>,
    method: &'static Method,
) {
    output.indent(1);
    output.put_ident(method.name);
    output.put("(");
    parameters(output, method);
    match method.ret {
        Return::Notify => output.put("): void;\n"),
        Return::Value(desc) => {
            output.put("): ");
            return_type(output, desc);
            output.put(" | Promise<");
            return_type(output, desc);
            output.put(">;\n");
        }
        Return::Stream(item) => {
            output.put("): Iterable<");
            desc_type(output, item);
            output.put("> | AsyncIterable<");
            desc_type(output, item);
            output.put(">;\n");
        }
    }
}

const fn methods<const CAPACITY: usize>(
    output: &mut Output<CAPACITY>,
    service: &'static Service,
    handler_side: bool,
) {
    let count = method_count(service);
    let mut index = 0;
    while index < count {
        let method = method_at(service, index);
        if handler_side {
            handler_method(output, method);
        } else {
            client_method(output, method);
        }
        index += 1;
    }
}

/// Render the `.d.ts` for one endpoint.
pub const fn render<const CAPACITY: usize>(endpoint: &super::Endpoint) -> Output<CAPACITY> {
    let mut output = Output::<CAPACITY>::new();
    output.put("// Generated by web-rpc. Do not edit.\n\n");

    let declarations = collect(endpoint);
    let mut index = 0;
    while index < declarations.length {
        let named_type = declarations.items[index];
        output.put("export type ");
        output.put(named_type.name);
        output.put(" = ");
        type_body(&mut output, named_type);
        output.put(";\n");
        index += 1;
    }
    if declarations.length > 0 {
        output.put("\n");
    }

    output
        .put("/** A request in flight. Awaiting it yields the response; `abort` cancels it. */\n");
    output.put("export type Request<T> = Promise<T> & { abort(): void };\n\n");
    output.put(
        "/** A stream in flight. `close` stops the producer; `done` settles when it ends. */\n",
    );
    output.put("export interface Subscription {\n");
    output.put("  close(): void;\n");
    output.put("  done: Promise<void>;\n");
    output.put("}\n\n");
    output.put(
        "/** A transport this endpoint can be handed. It is never created or closed here. */\n",
    );
    output.put("export type Endpoint = Worker | MessagePort | DedicatedWorkerGlobalScope;\n\n");

    if let Some(service) = endpoint.service {
        output.put("/** The methods this endpoint must implement. Every one is required. */\n");
        output.put("export interface ");
        output.put(service.name);
        output.put(" {\n");
        methods(&mut output, service, true);
        output.put("}\n\n");
    }

    output.put("export type ");
    output.put(endpoint.class);
    output.put("Options = {\n");
    output.put("  endpoint: Endpoint;\n");
    if let Some(service) = endpoint.service {
        output.put("  handlers: ");
        output.put(service.name);
        output.put(";\n");
    }
    output.put("};\n\n");

    output.put("export declare class ");
    output.put(endpoint.class);
    output.put(" {\n");
    output.put("  constructor(options: ");
    output.put(endpoint.class);
    output.put("Options);\n");
    if let Some(client) = endpoint.client {
        methods(&mut output, client, false);
    }
    output.put("  /** Detach listeners, reject pending requests and close open streams. */\n");
    output.put("  close(): void;\n");
    output.put("}\n");
    output
}
