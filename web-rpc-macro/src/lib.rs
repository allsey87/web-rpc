use std::collections::HashSet;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, quote_spanned, ToTokens};
use syn::{
    braced,
    ext::IdentExt,
    parenthesized,
    parse::{Parse, ParseStream},
    parse_macro_input, parse_quote,
    punctuated::Punctuated,
    spanned::Spanned,
    token::Comma,
    Attribute, FnArg, Ident, Lifetime, Pat, PatType, ReturnType, Token, Type, Visibility,
};

macro_rules! extend_errors {
    ($errors: ident, $e: expr) => {
        match $errors {
            Ok(_) => $errors = Err($e),
            Err(ref mut errors) => errors.extend($e),
        }
    };
}

/// If `ty` is `impl Stream<Item = T>`, returns Some(T).
fn stream_item_type(ty: &Type) -> Option<&Type> {
    if let Type::ImplTrait(impl_trait) = ty {
        for bound in &impl_trait.bounds {
            if let syn::TypeParamBound::Trait(trait_bound) = bound {
                let last_segment = trait_bound.path.segments.last()?;
                if last_segment.ident == "Stream" {
                    if let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments {
                        for arg in &args.args {
                            if let syn::GenericArgument::Binding(binding) = arg {
                                if binding.ident == "Item" {
                                    return Some(&binding.ty);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// If `ty` is `Option<T>`, returns Some(T).
fn option_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(type_path) = ty {
        let last_seg = type_path.path.segments.last()?;
        if last_seg.ident == "Option" {
            if let syn::PathArguments::AngleBracketed(args) = &last_seg.arguments {
                if args.args.len() == 1 {
                    if let syn::GenericArgument::Type(inner) = &args.args[0] {
                        return Some(inner);
                    }
                }
            }
        }
    }
    None
}

/// If `ty` is `Result<T, E>`, returns Some((T, E)).
fn result_inner_types(ty: &Type) -> Option<(&Type, &Type)> {
    if let Type::Path(type_path) = ty {
        let last_seg = type_path.path.segments.last()?;
        if last_seg.ident == "Result" {
            if let syn::PathArguments::AngleBracketed(args) = &last_seg.arguments {
                if args.args.len() == 2 {
                    if let (syn::GenericArgument::Type(ok_ty), syn::GenericArgument::Type(err_ty)) =
                        (&args.args[0], &args.args[1])
                    {
                        return Some((ok_ty, err_ty));
                    }
                }
            }
        }
    }
    None
}

/// True if `ty` is `&str` or `&[u8]` — the only reference shapes we route through
/// the existing serde-borrowing path (zero-copy, with an `'a` lifetime injected
/// into the request enum). Any other reference shape goes through the JS path.
fn is_borrowed_serde_ref(ty: &Type) -> bool {
    if let Type::Reference(r) = ty {
        match &*r.elem {
            Type::Path(p) if p.path.is_ident("str") => return true,
            Type::Slice(s) => {
                if let Type::Path(p) = &*s.elem {
                    if p.path.is_ident("u8") {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// True if `ty` is a reference to a presumed JS type (anything other than
/// `&str`/`&[u8]`). The receiver side decodes these via `JsCast::dyn_ref`.
fn is_js_ref(ty: &Type) -> bool {
    matches!(ty, Type::Reference(_)) && !is_borrowed_serde_ref(ty)
}

/// True if `attr` is a cfg-style attribute (`#[cfg(...)]` or `#[cfg_attr(...)]`).
/// These are propagated onto every generated artifact derived from a method so
/// that rustc strips them in lockstep after macro expansion.
fn is_cfg_attr(attr: &Attribute) -> bool {
    attr.path.is_ident("cfg") || attr.path.is_ident("cfg_attr")
}

/// Recursively emit code that encodes a value of type `ty` into a `WireArg`,
/// pushing JS values onto `post` as a side-effect.
///
/// Caller supplies `value` as a token-tree expression (typically an ident binding).
/// The emitted code matches structure on `Option`/`Result` and recurses; bare
/// leaves dispatch through the autoref encoder traits.
///
/// Reference-to-JS types should be handled by the caller before invoking this
/// helper — they cannot be encoded as nested elements (no `Decoder<&T>` impl on
/// the receiver side).
fn emit_encode(ty: &Type, value: TokenStream2, post: &TokenStream2) -> TokenStream2 {
    // Match against `&#value` so the original binding remains accessible to any
    // transfer-side code emitted alongside the encoder. Match ergonomics binds
    // `__inner` as a reference inside each arm.
    if let Some(inner) = option_inner_type(ty) {
        let inner_enc = emit_encode(inner, quote!(__inner), post);
        quote_spanned! {ty.span()=>
            match &#value {
                ::core::option::Option::Some(__inner) =>
                    web_rpc::codec::WireArg::Some(std::boxed::Box::new(#inner_enc)),
                ::core::option::Option::None =>
                    web_rpc::codec::WireArg::None,
            }
        }
    } else if let Some((ok, err)) = result_inner_types(ty) {
        let ok_enc = emit_encode(ok, quote!(__inner), post);
        let err_enc = emit_encode(err, quote!(__inner), post);
        quote_spanned! {ty.span()=>
            match &#value {
                ::core::result::Result::Ok(__inner) =>
                    web_rpc::codec::WireArg::Ok(std::boxed::Box::new(#ok_enc)),
                ::core::result::Result::Err(__inner) =>
                    web_rpc::codec::WireArg::Err(std::boxed::Box::new(#err_enc)),
            }
        }
    } else {
        quote_spanned! {ty.span()=>
            {
                #[allow(unused_imports)]
                use web_rpc::codec::{
                    __RpcJsEncode as _,
                    __RpcSerialEncode as _,
                };
                (&#value).__rpc_encode(#post)
            }
        }
    }
}

/// Recursively emit code that decodes a `WireArg` of type `ty` into a Rust value,
/// shifting JS values off `post` as needed.
///
/// Caller supplies `wire` as a token-tree expression evaluating to a `WireArg`.
/// Reference-to-JS types should be handled by the caller — see `emit_encode`.
fn emit_decode(ty: &Type, wire: TokenStream2, post: &TokenStream2) -> TokenStream2 {
    if let Some(inner) = option_inner_type(ty) {
        let inner_dec = emit_decode(inner, quote!(*__inner), post);
        quote_spanned! {ty.span()=>
            match #wire {
                web_rpc::codec::WireArg::Some(__inner) =>
                    ::core::option::Option::Some(#inner_dec),
                web_rpc::codec::WireArg::None =>
                    ::core::option::Option::None,
                _ => panic!("web_rpc: wire/type mismatch — expected Some or None"),
            }
        }
    } else if let Some((ok, err)) = result_inner_types(ty) {
        let ok_dec = emit_decode(ok, quote!(*__inner), post);
        let err_dec = emit_decode(err, quote!(*__inner), post);
        quote_spanned! {ty.span()=>
            match #wire {
                web_rpc::codec::WireArg::Ok(__inner) =>
                    ::core::result::Result::Ok(#ok_dec),
                web_rpc::codec::WireArg::Err(__inner) =>
                    ::core::result::Result::Err(#err_dec),
                _ => panic!("web_rpc: wire/type mismatch — expected Ok or Err"),
            }
        }
    } else {
        quote_spanned! {ty.span()=>
            {
                #[allow(unused_imports)]
                use web_rpc::codec::{
                    __RpcJsDecode as _,
                    __RpcSerialDecode as _,
                };
                (&web_rpc::codec::Decoder::<#ty>::default()).__rpc_decode(#wire, #post)
            }
        }
    }
}

struct Service {
    attrs: Vec<Attribute>,
    vis: Visibility,
    ident: Ident,
    rpcs: Vec<RpcMethod>,
}

struct RpcMethod {
    is_async: Option<Token![async]>,
    attrs: Vec<Attribute>,
    receiver: syn::Receiver,
    ident: Ident,
    args: Vec<PatType>,
    transfer: Vec<TransferClause>,
    output: ReturnType,
}

/// One entry inside a `#[transfer(...)]` attribute.
#[allow(dead_code)]
enum TransferClause {
    /// `name` — push the parameter itself, unconditionally.
    BareParam(Ident),
    /// `data => data.buffer()` — push the expression's result, unconditionally.
    ParamExpr { name: Ident, body: syn::Expr },
    /// `data => |Some(d)| d.buffer()` (closure) or
    /// `data => match { Some(d) => d.buffer(), ... }` (match-block).
    /// Each `Gate` becomes one `if let pat = &name { __transfer.push(body) }`.
    ParamGated { name: Ident, gates: Vec<Gate> },
    /// `return` — push the response value itself, unconditionally.
    BareReturn,
    /// `return => |Ok(o)| o.buffer()` or `return => match { ... }`.
    ReturnGated { gates: Vec<Gate> },
}

#[allow(dead_code)]
struct Gate {
    pat: syn::Pat,
    body: syn::Expr,
}

struct ServiceGenerator<'a> {
    trait_ident: &'a Ident,
    service_ident: &'a Ident,
    client_ident: &'a Ident,
    request_ident: &'a Ident,
    response_ident: &'a Ident,
    vis: &'a Visibility,
    attrs: &'a [Attribute],
    rpcs: &'a [RpcMethod],
    camel_case_idents: &'a [Ident],
    has_streaming_methods: bool,
}

impl<'a> ServiceGenerator<'a> {
    fn enum_request(&self) -> TokenStream2 {
        let &Self {
            vis,
            request_ident,
            camel_case_idents,
            rpcs,
            ..
        } = self;
        let variants = rpcs.iter().zip(camel_case_idents.iter()).map(
            |(RpcMethod { attrs, args, .. }, camel_case_ident)| {
                let cfg_attrs = attrs.iter().filter(|a| is_cfg_attr(a));
                let fields = args.iter().map(|arg| {
                    let pat = &arg.pat;
                    if is_borrowed_serde_ref(&arg.ty) {
                        // `&str` / `&[u8]` — keep zero-copy serde borrowing path.
                        let mut type_ref = match &*arg.ty {
                            Type::Reference(r) => r.clone(),
                            _ => unreachable!("is_borrowed_serde_ref guarantees a reference"),
                        };
                        type_ref.lifetime =
                            Some(Lifetime::new("'a", type_ref.and_token.span()));
                        quote_spanned! {arg.ty.span()=> #pat: #type_ref }
                    } else {
                        // Everything else (including `&JsT`) uses the universal
                        // recursive WireArg representation.
                        quote_spanned! {arg.ty.span()=>
                            #pat: web_rpc::codec::WireArg
                        }
                    }
                });
                quote! {
                    #(#cfg_attrs)*
                    #camel_case_ident { #( #fields ),* }
                }
            },
        );
        // `<'a>` is always emitted, with a hidden variant that uses it via
        // `PhantomData`. This keeps the enum well-formed regardless of which
        // methods rustc strips via cfg-evaluation after macro expansion: a
        // service whose only borrowing methods get cfg'd out would otherwise
        // hit E0392. The macro never constructs this variant on the wire; the
        // server's match arm panics if it ever appears.
        quote! {
            #[derive(web_rpc::serde::Serialize, web_rpc::serde::Deserialize)]
            #vis enum #request_ident<'a> {
                #( #variants, )*
                #[doc(hidden)]
                __WebRpcPhantom(std::marker::PhantomData<&'a ()>),
            }
        }
    }

    fn enum_response(&self) -> TokenStream2 {
        let &Self {
            vis,
            response_ident,
            camel_case_idents,
            rpcs,
            ..
        } = self;
        let variants = rpcs.iter().zip(camel_case_idents.iter()).map(
            |(RpcMethod { attrs, .. }, camel_case_ident)| {
                let cfg_attrs = attrs.iter().filter(|a| is_cfg_attr(a));
                // Every method's response variant carries a single uniform
                // `WireArg`. Notification methods (no return) still get a
                // variant — the macro fills it with a placeholder that the
                // client never reads.
                quote! {
                    #(#cfg_attrs)*
                    #camel_case_ident ( web_rpc::codec::WireArg )
                }
            },
        );
        quote! {
            #[derive(web_rpc::serde::Serialize, web_rpc::serde::Deserialize)]
            #vis enum #response_ident {
                #( #variants ),*
            }
        }
    }

    fn trait_service(&self) -> TokenStream2 {
        let &Self {
            attrs,
            rpcs,
            vis,
            trait_ident,
            ..
        } = self;

        let unit_type: &Type = &parse_quote!(());
        let rpc_fns = rpcs.iter().map(
            |RpcMethod {
                 attrs,
                 args,
                 receiver,
                 ident,
                 is_async,
                 output,
                 ..
             }| {
                if let ReturnType::Type(_, ref ty) = output {
                    if let Some(item_ty) = stream_item_type(ty) {
                        return quote_spanned! {ident.span()=>
                            #( #attrs )*
                            #is_async fn #ident(#receiver, #( #args ),*) -> impl web_rpc::futures_core::Stream<Item = #item_ty>;
                        };
                    }
                }
                let output = match output {
                    ReturnType::Type(_, ref ty) => ty,
                    ReturnType::Default => unit_type,
                };
                quote_spanned! {ident.span()=>
                    #( #attrs )*
                    #is_async fn #ident(#receiver, #( #args ),*) -> #output;
                }
            },
        );

        let forward_fns = rpcs
            .iter()
            .map(
                |RpcMethod {
                     attrs,
                     args,
                     receiver,
                     ident,
                     is_async,
                     output,
                     ..
                 }| {
                    {
                        let output = if let ReturnType::Type(_, ref ty) = output {
                            if let Some(item_ty) = stream_item_type(ty) {
                                quote! { impl web_rpc::futures_core::Stream<Item = #item_ty> }
                            } else {
                                let ty: &Type = ty;
                                quote! { #ty }
                            }
                        } else {
                            let ty = unit_type;
                            quote! { #ty }
                        };
                        let do_await = match is_async {
                            Some(token) => quote_spanned!(token.span=> .await),
                            None => quote!(),
                        };
                        let forward_args = args.iter().filter_map(|arg| match &*arg.pat {
                            Pat::Ident(ident) => Some(&ident.ident),
                            _ => None,
                        });
                        quote_spanned! {ident.span()=>
                            #( #attrs )*
                            #is_async fn #ident(#receiver, #( #args ),*) -> #output {
                                T::#ident(self, #( #forward_args ),*)#do_await
                            }
                        }
                    }
                },
            )
            .collect::<Vec<_>>();

        quote! {
            #( #attrs )*
            #[allow(async_fn_in_trait)]
            #vis trait #trait_ident {
                #( #rpc_fns )*
            }

            impl<T> #trait_ident for std::sync::Arc<T> where T: #trait_ident {
                #( #forward_fns )*
            }
            impl<T> #trait_ident for std::boxed::Box<T> where T: #trait_ident {
                #( #forward_fns )*
            }
            impl<T> #trait_ident for std::rc::Rc<T> where T: #trait_ident {
                #( #forward_fns )*
            }
        }
    }

    fn struct_client(&self) -> TokenStream2 {
        let &Self {
            vis,
            client_ident,
            request_ident,
            response_ident,
            camel_case_idents,
            rpcs,
            has_streaming_methods,
            ..
        } = self;

        let rpc_fns = rpcs
            .iter()
            .zip(camel_case_idents.iter())
            .map(|(RpcMethod { attrs, args, transfer, ident, output, .. }, camel_case_ident)| {
                // 1. Per-arg encoding: borrowed `&str`/`&[u8]` pass through inline;
                // everything else routes through the autoref-dispatched `__rpc_encode`.
                let mut arg_encodings = Vec::<TokenStream2>::new();
                let mut request_struct_fields = Vec::<TokenStream2>::new();
                for arg in args {
                    let id = match &*arg.pat {
                        Pat::Ident(p) => &p.ident,
                        _ => continue,
                    };
                    if is_borrowed_serde_ref(&arg.ty) {
                        request_struct_fields.push(quote! { #id });
                    } else {
                        let wire_ident = format_ident!("__wire_{}", id);
                        let post = quote!(&__post);
                        let enc = emit_encode(&arg.ty, quote!(#id), &post);
                        arg_encodings.push(quote! { let #wire_ident = #enc; });
                        request_struct_fields.push(quote! { #id: #wire_ident });
                    }
                }

                // 2. Per-method transfer pushes (param-side only; return-side
                // clauses are handled in struct_server).
                let transfer_pushes = transfer.iter().filter_map(|c| match c {
                    TransferClause::BareParam(name) => Some(quote! {
                        __transfer.push(#name.as_ref());
                    }),
                    TransferClause::ParamExpr { name, body } => Some(quote_spanned! {body.span()=>
                        {
                            let _ = &#name; // ensure name is referenced
                            __transfer.push((#body).as_ref());
                        }
                    }),
                    TransferClause::ParamGated { name, gates } => {
                        let arms = gates.iter().map(|g| {
                            let pat = &g.pat;
                            let body = &g.body;
                            quote_spanned! {body.span()=>
                                if let #pat = &#name {
                                    __transfer.push((#body).as_ref());
                                }
                            }
                        });
                        Some(quote! { #( #arms )* })
                    }
                    TransferClause::BareReturn | TransferClause::ReturnGated { .. } => None,
                });

                let send_request = quote! {
                    let __seq_id = self.seq_id.replace_with(|seq_id| seq_id.wrapping_add(1));
                    let __post = web_rpc::js_sys::Array::new();
                    let __transfer = web_rpc::js_sys::Array::new();
                    #( #arg_encodings )*
                    let __request = #request_ident::#camel_case_ident {
                        #( #request_struct_fields ),*
                    };
                    let __header = web_rpc::MessageHeader::Request(__seq_id);
                    let __header_bytes = web_rpc::bincode::serialize(&__header).unwrap();
                    let __header_buffer = web_rpc::js_sys::Uint8Array::from(&__header_bytes[..]).buffer();
                    let __payload_bytes = web_rpc::bincode::serialize(&__request).unwrap();
                    let __payload_buffer = web_rpc::js_sys::Uint8Array::from(&__payload_bytes[..]).buffer();
                    // Prepend [header, payload] in front of the encoded JS values.
                    __post.unshift(&__payload_buffer);
                    __post.unshift(&__header_buffer);
                    __transfer.push(__header_buffer.as_ref());
                    __transfer.push(__payload_buffer.as_ref());
                    #( #transfer_pushes )*
                    self.port.post_message(&__post, &__transfer).unwrap();
                };

                let is_streaming = matches!(
                    output,
                    ReturnType::Type(_, ref ty) if stream_item_type(ty).is_some()
                );

                if is_streaming {
                    let item_ty = match output {
                        ReturnType::Type(_, ref ty) => stream_item_type(ty).unwrap(),
                        _ => unreachable!(),
                    };
                    let dec = emit_decode(item_ty, quote!(__wire), &quote!(&__post_array));

                    let unpack_stream_item = quote! {
                        |(__response, __post_array): (#response_ident, web_rpc::js_sys::Array)| {
                            let #response_ident::#camel_case_ident(__wire) = __response else {
                                panic!("web_rpc: received incorrect response variant")
                            };
                            #dec
                        }
                    };

                    quote! {
                        #( #attrs )*
                        #vis fn #ident(
                            &self,
                            #( #args ),*
                        ) -> web_rpc::client::StreamReceiver<#item_ty> {
                            #send_request
                            let (__item_tx, __item_rx) = web_rpc::futures_channel::mpsc::unbounded();
                            self.stream_callback_map.borrow_mut().insert(__seq_id, __item_tx);
                            let __mapped_rx = web_rpc::futures_util::StreamExt::map(
                                __item_rx,
                                #unpack_stream_item
                            );
                            let __abort_sender = self.abort_sender.clone();
                            let __stream_callback_map = self.stream_callback_map.clone();
                            let __dispatcher = self.dispatcher.clone();
                            web_rpc::client::StreamReceiver::new(
                                __mapped_rx,
                                __dispatcher,
                                std::boxed::Box::new(move || {
                                    __stream_callback_map.borrow_mut().remove(&__seq_id);
                                    (__abort_sender)(__seq_id);
                                }),
                            )
                        }
                    }
                } else {
                    let return_type = match output {
                        ReturnType::Type(_, ref ty) => quote! {
                            web_rpc::client::RequestFuture<#ty>
                        },
                        _ => quote!(()),
                    };
                    let maybe_register_callback = match output {
                        ReturnType::Type(_, _) => quote! {
                            let (__response_tx, __response_rx) =
                                web_rpc::futures_channel::oneshot::channel();
                            self.callback_map.borrow_mut().insert(__seq_id, __response_tx);
                        },
                        _ => Default::default(),
                    };

                    let maybe_unpack_and_return_future = match output {
                        ReturnType::Type(_, ref ret_ty) => {
                            let dec = emit_decode(ret_ty, quote!(__wire), &quote!(&__post_array));
                            quote! {
                                let __response_future = web_rpc::futures_util::FutureExt::map(
                                    __response_rx,
                                    |response| {
                                        let (__serialize_response, __post_array) = response.unwrap();
                                        let #response_ident::#camel_case_ident(__wire) = __serialize_response else {
                                            panic!("web_rpc: received incorrect response variant")
                                        };
                                        #dec
                                    }
                                );
                                let __abort_sender = self.abort_sender.clone();
                                let __dispatcher = self.dispatcher.clone();
                                web_rpc::client::RequestFuture::new(
                                    __response_future,
                                    __dispatcher,
                                    std::boxed::Box::new(move || (__abort_sender)(__seq_id)))
                            }
                        }
                        _ => Default::default(),
                    };

                    quote! {
                        #( #attrs )*
                        #vis fn #ident(
                            &self,
                            #( #args ),*
                        ) -> #return_type {
                            #send_request
                            #maybe_register_callback
                            #maybe_unpack_and_return_future
                        }
                    }
                }
            });

        let stream_callback_map_field = if has_streaming_methods {
            // `#[allow(dead_code)]` covers the case where every streaming method
            // is stripped via cfg — the field is still bound by the
            // `From<Configuration>` impl but no surviving method reads it.
            quote! {
                #[allow(dead_code)]
                stream_callback_map: std::rc::Rc<
                    std::cell::RefCell<
                        web_rpc::client::StreamCallbackMap<#response_ident>
                    >
                >,
            }
        } else {
            quote!()
        };

        let stream_callback_map_pat = if has_streaming_methods {
            quote! { stream_callback_map, }
        } else {
            quote! { _, }
        };

        let stream_callback_map_init = if has_streaming_methods {
            quote! { stream_callback_map, }
        } else {
            quote! {}
        };

        quote! {
            #[derive(core::clone::Clone)]
            #vis struct #client_ident {
                callback_map: std::rc::Rc<
                    std::cell::RefCell<
                        web_rpc::client::CallbackMap<#response_ident>
                    >
                >,
                #stream_callback_map_field
                port: web_rpc::port::Port,
                listener: std::rc::Rc<web_rpc::gloo_events::EventListener>,
                dispatcher: web_rpc::futures_util::future::Shared<
                    web_rpc::futures_core::future::LocalBoxFuture<'static, ()>
                >,
                abort_sender: std::rc::Rc<dyn std::ops::Fn(usize)>,
                seq_id: std::rc::Rc<std::cell::RefCell<usize>>
            }
            impl std::fmt::Debug for #client_ident {
                fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.debug_struct(std::stringify!(#client_ident))
                        .finish()
                }
            }
            impl web_rpc::client::Client for #client_ident {
                type Response = #response_ident;
            }
            impl From<web_rpc::client::Configuration<#response_ident>>
                for #client_ident {
                fn from((callback_map, #stream_callback_map_pat port, listener, dispatcher, abort_sender):
                    web_rpc::client::Configuration<#response_ident>) -> Self {
                    Self {
                        callback_map,
                        #stream_callback_map_init
                        port,
                        listener,
                        dispatcher,
                        abort_sender,
                        seq_id: std::default::Default::default()
                    }
                }
            }
            impl #client_ident {
                #( #rpc_fns )*
            }
        }
    }

    fn struct_server(&self) -> TokenStream2 {
        let &Self {
            vis,
            trait_ident,
            service_ident,
            request_ident,
            response_ident,
            camel_case_idents,
            rpcs,
            ..
        } = self;

        let request_type = quote! { #request_ident<'_> };

        let handlers = rpcs.iter()
            .zip(camel_case_idents.iter())
            .map(|(RpcMethod { is_async, ident, args, transfer, output, attrs, .. }, camel_case_ident)| {
                let cfg_attrs: Vec<_> = attrs.iter().filter(|a| is_cfg_attr(a)).collect();
                // 1. Destructure pattern for the request enum variant.
                // Borrowed args use their own ident; non-borrowed args bind to __wire_<id>.
                let destructure_fields: Vec<_> = args.iter()
                    .filter_map(|arg| {
                        let id = match &*arg.pat {
                            Pat::Ident(p) => &p.ident,
                            _ => return None,
                        };
                        Some(if is_borrowed_serde_ref(&arg.ty) {
                            quote! { #id }
                        } else {
                            let wire_ident = format_ident!("__wire_{}", id);
                            quote! { #id: #wire_ident }
                        })
                    })
                    .collect();

                // 2. Per-arg decoding statements.
                let arg_decodes: Vec<_> = args.iter()
                    .filter_map(|arg| {
                        let id = match &*arg.pat {
                            Pat::Ident(p) => &p.ident,
                            _ => return None,
                        };
                        if is_borrowed_serde_ref(&arg.ty) {
                            // Already bound by destructuring.
                            None
                        } else if is_js_ref(&arg.ty) {
                            // `&T` where T: JsCast — no `Decoder<&T>` impl, so we
                            // shift from the post-array and bind via dyn_ref locally.
                            let inner_ty = match &*arg.ty {
                                Type::Reference(r) => &*r.elem,
                                _ => unreachable!(),
                            };
                            let tmp_ident = format_ident!("__tmp_{}", id);
                            let wire_ident = format_ident!("__wire_{}", id);
                            let arg_ty = &arg.ty;
                            Some(quote! {
                                let #tmp_ident = match #wire_ident {
                                    web_rpc::codec::WireArg::Js => __js_args.shift(),
                                    _ => panic!("web_rpc: expected Js wire variant for reference arg"),
                                };
                                let #id: #arg_ty = web_rpc::wasm_bindgen::JsCast::dyn_ref::<#inner_ty>(&#tmp_ident)
                                    .unwrap();
                            })
                        } else {
                            let wire_ident = format_ident!("__wire_{}", id);
                            let dec = emit_decode(&arg.ty, quote!(#wire_ident), &quote!(&__js_args));
                            Some(quote! { let #id = #dec; })
                        }
                    })
                    .collect();

                let call_args: Vec<_> = args.iter().filter_map(|arg| match &*arg.pat {
                    Pat::Ident(ident) => Some(&ident.ident),
                    _ => None,
                }).collect();

                // Return-side transfer clauses (BareReturn / ReturnGated).
                // The scrutinee is `__response` for non-streaming and `__item` for streaming.
                let make_return_transfer = |scrutinee_ident: &Ident| -> TokenStream2 {
                    let pushes = transfer.iter().filter_map(|c| match c {
                        TransferClause::BareReturn => Some(quote! {
                            __transfer.push(#scrutinee_ident.as_ref());
                        }),
                        TransferClause::ReturnGated { gates } => {
                            let arms = gates.iter().map(|g| {
                                let pat = &g.pat;
                                let body = &g.body;
                                quote_spanned! {body.span()=>
                                    if let #pat = &#scrutinee_ident {
                                        __transfer.push((#body).as_ref());
                                    }
                                }
                            });
                            Some(quote! { #( #arms )* })
                        }
                        _ => None,
                    });
                    quote! { #( #pushes )* }
                };

                let is_streaming = matches!(
                    output,
                    ReturnType::Type(_, ref ty) if stream_item_type(ty).is_some()
                );

                if is_streaming {
                    let item_ty = match output {
                        ReturnType::Type(_, ref ty) => stream_item_type(ty).unwrap(),
                        _ => unreachable!(),
                    };
                    let item_enc = emit_encode(item_ty, quote!(__item), &quote!(&__post));
                    let item_ident = Ident::new("__item", proc_macro2::Span::call_site());
                    let return_transfer = make_return_transfer(&item_ident);

                    let wrap_item = quote! {
                        let __post = web_rpc::js_sys::Array::new();
                        let __transfer = web_rpc::js_sys::Array::new();
                        let __wire_item = #item_enc;
                        #return_transfer
                        let __response = #response_ident::#camel_case_ident(__wire_item);
                    };

                    let fwd_body = quote! {
                        let __stream_tx_clone = __stream_tx.clone();
                        web_rpc::pin_utils::pin_mut!(__user_rx);
                        let __fwd = async move {
                            while let Some(__item) = web_rpc::futures_util::StreamExt::next(&mut __user_rx).await {
                                #wrap_item
                                if __stream_tx_clone.unbounded_send((__seq_id, Some((__response, __post, __transfer)))).is_err() {
                                    break;
                                }
                            }
                        };
                        let __fwd = web_rpc::futures_util::FutureExt::fuse(__fwd);
                        web_rpc::pin_utils::pin_mut!(__fwd);
                        web_rpc::futures_util::select! {
                            _ = __abort_rx => {},
                            _ = __fwd => {},
                        }
                        let _ = __stream_tx.unbounded_send((__seq_id, None));
                        web_rpc::service::ExecuteResult::StreamComplete
                    };

                    match is_async {
                        Some(_) => quote! {
                            #( #cfg_attrs )*
                            #request_ident::#camel_case_ident { #( #destructure_fields ),* } => {
                                #( #arg_decodes )*
                                let __get_rx = web_rpc::futures_util::FutureExt::fuse(
                                    self.server_impl.#ident(#( #call_args ),*)
                                );
                                web_rpc::pin_utils::pin_mut!(__get_rx);
                                let __maybe_rx = web_rpc::futures_util::select! {
                                    _ = __abort_rx => None,
                                    __rx = __get_rx => Some(__rx),
                                };
                                if let Some(mut __user_rx) = __maybe_rx {
                                    #fwd_body
                                } else {
                                    let _ = __stream_tx.unbounded_send((__seq_id, None));
                                    web_rpc::service::ExecuteResult::StreamComplete
                                }
                            }
                        },
                        None => quote! {
                            #( #cfg_attrs )*
                            #request_ident::#camel_case_ident { #( #destructure_fields ),* } => {
                                #( #arg_decodes )*
                                let mut __user_rx = self.server_impl.#ident(#( #call_args ),*);
                                #fwd_body
                            }
                        },
                    }
                } else {
                    // Non-streaming.
                    let resp_ident = Ident::new("__response", proc_macro2::Span::call_site());
                    let return_transfer = make_return_transfer(&resp_ident);
                    let return_response = match output {
                        ReturnType::Type(_, ref ret_ty) => {
                            let enc = emit_encode(ret_ty, quote!(__response), &quote!(&__post));
                            quote! {
                                let __post = web_rpc::js_sys::Array::new();
                                let __transfer = web_rpc::js_sys::Array::new();
                                let __wire = #enc;
                                #return_transfer
                                (#response_ident::#camel_case_ident(__wire), __post, __transfer)
                            }
                        }
                        _ => {
                            // Notification — emit a placeholder WireArg.
                            quote! {
                                let _ = __response;
                                let __post = web_rpc::js_sys::Array::new();
                                let __transfer = web_rpc::js_sys::Array::new();
                                let __wire = web_rpc::codec::WireArg::Bytes(
                                    web_rpc::bincode::serialize(&()).unwrap()
                                );
                                (#response_ident::#camel_case_ident(__wire), __post, __transfer)
                            }
                        }
                    };

                    match is_async {
                        Some(_) => quote! {
                            #( #cfg_attrs )*
                            #request_ident::#camel_case_ident { #( #destructure_fields ),* } => {
                                #( #arg_decodes )*
                                let __task =
                                    web_rpc::futures_util::FutureExt::fuse(self.server_impl.#ident(#( #call_args ),*));
                                web_rpc::pin_utils::pin_mut!(__task);
                                web_rpc::service::ExecuteResult::Response(
                                    web_rpc::futures_util::select! {
                                        _ = __abort_rx => None,
                                        __response = __task => Some({
                                            #return_response
                                        })
                                    }
                                )
                            }
                        },
                        None => quote! {
                            #( #cfg_attrs )*
                            #request_ident::#camel_case_ident { #( #destructure_fields ),* } => {
                                #( #arg_decodes )*
                                let __response = self.server_impl.#ident(#( #call_args ),*);
                                web_rpc::service::ExecuteResult::Response(
                                    Some({
                                        #return_response
                                    })
                                )
                            }
                        }
                    }
                }
            });

        quote! {
            #vis struct #service_ident<T> {
                server_impl: T
            }
            impl<T: #trait_ident> web_rpc::service::Service for #service_ident<T> {
                type Response = #response_ident;
                async fn execute(
                    &self,
                    __seq_id: usize,
                    mut __abort_rx: web_rpc::futures_channel::oneshot::Receiver<()>,
                    __payload: std::vec::Vec<u8>,
                    __js_args: web_rpc::js_sys::Array,
                    __stream_tx: web_rpc::futures_channel::mpsc::UnboundedSender<
                        web_rpc::service::StreamMessage<Self::Response>
                    >,
                ) -> (usize, web_rpc::service::ExecuteResult<Self::Response>) {
                    let __request: #request_type = web_rpc::bincode::deserialize(&__payload).unwrap();
                    let __result = match __request {
                        #( #handlers )*
                        #request_ident::__WebRpcPhantom(_) => {
                            unreachable!("web_rpc: __WebRpcPhantom variant received on wire")
                        }
                    };
                    (__seq_id, __result)
                }
            }
            impl<T: #trait_ident> std::convert::From<T> for #service_ident<T> {
                fn from(server_impl: T) -> Self {
                    Self { server_impl }
                }
            }
        }
    }
}

impl<'a> ToTokens for ServiceGenerator<'a> {
    fn to_tokens(&self, output: &mut TokenStream2) {
        output.extend(vec![
            self.enum_request(),
            self.enum_response(),
            self.trait_service(),
            self.struct_client(),
            self.struct_server(),
        ])
    }
}

impl Parse for Service {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis = input.parse()?;
        input.parse::<Token![trait]>()?;
        let ident: Ident = input.parse()?;
        let content;
        braced!(content in input);
        let mut rpcs = Vec::<RpcMethod>::new();
        while !content.is_empty() {
            rpcs.push(content.parse()?);
        }

        Ok(Self {
            attrs,
            vis,
            ident,
            rpcs,
        })
    }
}

/// Parsed RHS of a `name => ...` or `return => ...` clause.
enum TransferRhs {
    Expr(syn::Expr),
    Gates(Vec<Gate>),
}

fn parse_transfer_rhs(input: ParseStream) -> syn::Result<TransferRhs> {
    if input.peek(Token![|]) || input.peek(Token![||]) {
        // Closure form: `|pat| body` (or `|| body` — rejected).
        let closure: syn::ExprClosure = input.parse()?;
        if closure.inputs.len() != 1 {
            return Err(syn::Error::new_spanned(
                &closure,
                "transfer closure must have exactly one parameter",
            ));
        }
        let pat = closure.inputs.into_iter().next().unwrap();
        let body = *closure.body;
        Ok(TransferRhs::Gates(vec![Gate { pat, body }]))
    } else if input.peek(Token![match]) {
        // `match { arms }` — no scrutinee. Bespoke syntax.
        input.parse::<Token![match]>()?;
        let content;
        braced!(content in input);
        let arms: Punctuated<syn::Arm, Token![,]> =
            content.parse_terminated(syn::Arm::parse)?;
        let gates = arms
            .into_iter()
            .map(|a| Gate {
                pat: a.pat,
                body: *a.body,
            })
            .collect();
        Ok(TransferRhs::Gates(gates))
    } else {
        // Bare expression — only valid for params; the caller checks.
        let body: syn::Expr = input.parse()?;
        Ok(TransferRhs::Expr(body))
    }
}

impl Parse for TransferClause {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let is_return = input.peek(Token![return]);
        let lhs_name: Option<Ident> = if is_return {
            input.parse::<Token![return]>()?;
            None
        } else {
            Some(input.parse()?)
        };

        if input.peek(Token![=>]) {
            input.parse::<Token![=>]>()?;
            let rhs = parse_transfer_rhs(input)?;
            match (lhs_name, rhs) {
                (Some(name), TransferRhs::Expr(body)) => {
                    Ok(TransferClause::ParamExpr { name, body })
                }
                (Some(name), TransferRhs::Gates(gates)) => {
                    Ok(TransferClause::ParamGated { name, gates })
                }
                (None, TransferRhs::Gates(gates)) => {
                    Ok(TransferClause::ReturnGated { gates })
                }
                (None, TransferRhs::Expr(_)) => Err(syn::Error::new(
                    input.span(),
                    "`return =>` requires a closure (`|pat| body`) or `match { arms }` block",
                )),
            }
        } else {
            Ok(match lhs_name {
                Some(name) => TransferClause::BareParam(name),
                None => TransferClause::BareReturn,
            })
        }
    }
}

impl Parse for RpcMethod {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut errors = Ok(());
        let attrs = input.call(Attribute::parse_outer)?;

        // Reject the removed `#[post(...)]` attribute with a migration message.
        for attr in &attrs {
            if attr
                .path
                .segments
                .last()
                .is_some_and(|seg| seg.ident == "post")
            {
                extend_errors!(
                    errors,
                    syn::Error::new_spanned(
                        attr,
                        "`#[post(...)]` has been removed. JS-vs-serialize routing is now \
                         inferred from each argument and return type. For transfer semantics, \
                         use `#[transfer(...)]` (e.g. `#[transfer(canvas)]`, \
                         `#[transfer(data => data.buffer())]`, or \
                         `#[transfer(return => |Ok(o)| o.buffer())]`)."
                    )
                );
            }
        }

        // Partition out the new `#[transfer(...)]` attribute(s).
        let (transfer_attrs, attrs): (Vec<_>, Vec<_>) = attrs.into_iter().partition(|attr| {
            attr.path
                .segments
                .last()
                .is_some_and(|last_segment| last_segment.ident == "transfer")
        });
        let mut transfer: Vec<TransferClause> = Vec::new();
        for transfer_attr in transfer_attrs {
            let parsed = transfer_attr
                .parse_args_with(Punctuated::<TransferClause, Token![,]>::parse_terminated)?;
            transfer.extend(parsed.into_iter());
        }

        let is_async = input.parse::<Token![async]>().ok();
        input.parse::<Token![fn]>()?;
        let ident: Ident = input.parse()?;

        // Reject generic methods up front — autoref dispatch needs concrete types.
        if input.peek(Token![<]) {
            let generics: syn::Generics = input.parse()?;
            extend_errors!(
                errors,
                syn::Error::new_spanned(
                    generics,
                    "web_rpc::service trait methods may not have generic parameters; \
                     concrete types are required so the macro can route each argument."
                )
            );
        }

        let content;
        parenthesized!(content in input);
        let mut receiver: Option<syn::Receiver> = None;
        let mut args = Vec::new();
        for arg in content.parse_terminated::<FnArg, Comma>(FnArg::parse)? {
            match arg {
                FnArg::Typed(captured) => match &*captured.pat {
                    Pat::Ident(_) => {
                        // Reject reference args other than `&str`/`&[u8]`/`&JsT`.
                        // (The is_js_ref / is_borrowed_serde_ref classifiers will
                        // accept any reference; we let them through here and rely
                        // on the receiver-side decoder to fail on unsupported
                        // shapes. A dedicated diagnostic comes later.)
                        args.push(captured)
                    }
                    _ => extend_errors!(
                        errors,
                        syn::Error::new(
                            captured.pat.span(),
                            "patterns are not allowed in RPC arguments"
                        )
                    ),
                },
                FnArg::Receiver(ref recv) => {
                    if recv.reference.is_none() || recv.mutability.is_some() {
                        extend_errors!(
                            errors,
                            syn::Error::new(
                                arg.span(),
                                "RPC methods only support `&self` as a receiver"
                            )
                        );
                    }
                    receiver = Some(recv.clone());
                }
            }
        }
        let receiver = match receiver {
            Some(r) => r,
            None => {
                extend_errors!(
                    errors,
                    syn::Error::new(
                        ident.span(),
                        "RPC methods must include `&self` as the first parameter"
                    )
                );
                parse_quote!(&self)
            }
        };
        let output: ReturnType = input.parse()?;
        input.parse::<Token![;]>()?;

        // Validate that every transfer clause references a real parameter
        // (or `return`, which has no name to check).
        let arg_names: HashSet<_> = args
            .iter()
            .filter_map(|arg| match &*arg.pat {
                Pat::Ident(pat_ident) => Some(pat_ident.ident.clone()),
                _ => None,
            })
            .collect();
        for clause in &transfer {
            let name_ref = match clause {
                TransferClause::BareParam(name)
                | TransferClause::ParamExpr { name, .. }
                | TransferClause::ParamGated { name, .. } => Some(name),
                TransferClause::BareReturn | TransferClause::ReturnGated { .. } => None,
            };
            if let Some(name) = name_ref {
                if !arg_names.contains(name) {
                    extend_errors!(
                        errors,
                        syn::Error::new(
                            name.span(),
                            format!(
                                "`{}` in #[transfer(...)] does not match any parameter",
                                name
                            )
                        )
                    );
                }
            }
        }
        errors?;

        Ok(Self {
            is_async,
            attrs,
            receiver,
            ident,
            args,
            transfer,
            output,
        })
    }
}

/// This attribute macro should applied to traits that need to be turned into RPCs. The
/// macro will consume the trait and output three items in its place. For example,
/// a trait `Calculator` will be replaced with two structs `CalculatorClient` and
/// `CalculatorService` and a new trait by the same name. All methods must include
/// `&self` as their first parameter.
#[proc_macro_attribute]
pub fn service(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let Service {
        ref attrs,
        ref vis,
        ref ident,
        ref rpcs,
    } = parse_macro_input!(input as Service);

    let camel_case_fn_names: &Vec<_> = &rpcs
        .iter()
        .map(|rpc| snake_to_camel(&rpc.ident.unraw().to_string()))
        .collect();

    let has_streaming_methods = rpcs.iter().any(
        |rpc| matches!(&rpc.output, ReturnType::Type(_, ref ty) if stream_item_type(ty).is_some()),
    );

    ServiceGenerator {
        trait_ident: ident,
        service_ident: &format_ident!("{}Service", ident),
        client_ident: &format_ident!("{}Client", ident),
        request_ident: &format_ident!("{}Request", ident),
        response_ident: &format_ident!("{}Response", ident),
        vis,
        attrs,
        rpcs,
        camel_case_idents: &rpcs
            .iter()
            .zip(camel_case_fn_names.iter())
            .map(|(rpc, name)| Ident::new(name, rpc.ident.span()))
            .collect::<Vec<_>>(),
        has_streaming_methods,
    }
    .into_token_stream()
    .into()
}

fn snake_to_camel(ident_str: &str) -> String {
    let mut camel_ty = String::with_capacity(ident_str.len());

    let mut last_char_was_underscore = true;
    for c in ident_str.chars() {
        match c {
            '_' => last_char_was_underscore = true,
            c if last_char_was_underscore => {
                camel_ty.extend(c.to_uppercase());
                last_char_was_underscore = false;
            }
            c => camel_ty.extend(c.to_lowercase()),
        }
    }

    camel_ty.shrink_to_fit();
    camel_ty
}
