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
    Attribute, FnArg, Ident, Lifetime, Pat, PatType, Path, ReturnType, Token, Type, Visibility,
};

macro_rules! extend_errors {
    ($errors: ident, $e: expr) => {
        match $errors {
            Ok(_) => $errors = Err($e),
            Err(ref mut errors) => errors.extend($e),
        }
    };
}

// ---------------------------------------------------------------------------
// Signature types
// ---------------------------------------------------------------------------

/// If `ty` is `impl Stream<Item = T>`, returns Some(T).
fn stream_item_type(ty: &Type) -> Option<&Type> {
    let Type::ImplTrait(impl_trait) = ty else {
        return None;
    };
    for bound in &impl_trait.bounds {
        let syn::TypeParamBound::Trait(trait_bound) = bound else {
            continue;
        };
        let last_segment = trait_bound.path.segments.last()?;
        if last_segment.ident != "Stream" {
            continue;
        }
        let syn::PathArguments::AngleBracketed(arguments) = &last_segment.arguments else {
            continue;
        };
        for argument in &arguments.args {
            if let syn::GenericArgument::AssocType(associated) = argument {
                if associated.ident == "Item" {
                    return Some(&associated.ty);
                }
            }
        }
    }
    None
}

/// The type arguments of `ty` if its last path segment is `wrapper<..>`.
fn type_arguments<'a>(ty: &'a Type, wrapper: &str) -> Option<Vec<&'a Type>> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let last_segment = type_path.path.segments.last()?;
    if last_segment.ident != wrapper {
        return None;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &last_segment.arguments else {
        return None;
    };
    arguments
        .args
        .iter()
        .map(|argument| match argument {
            syn::GenericArgument::Type(ty) => Some(ty),
            _ => None,
        })
        .collect()
}

/// If `ty` is `Option<T>`, returns Some(T).
fn option_inner_type(ty: &Type) -> Option<&Type> {
    match type_arguments(ty, "Option")?.as_slice() {
        [inner] => Some(inner),
        _ => None,
    }
}

/// If `ty` is `Result<T, E>`, returns Some((T, E)).
fn result_inner_types(ty: &Type) -> Option<(&Type, &Type)> {
    match type_arguments(ty, "Result")?.as_slice() {
        [ok, err] => Some((ok, err)),
        _ => None,
    }
}

/// If `ty` is `Post<T>` or `Transfer<T>`, returns the inner type and whether it is transferred.
fn js_inner_type(ty: &Type) -> Option<(&Type, bool)> {
    for (wrapper, transfer) in [("Post", false), ("Transfer", true)] {
        if let Some([inner]) = type_arguments(ty, wrapper).as_deref() {
            return Some((inner, transfer));
        }
    }
    None
}

/// True if `ty` is `&str` or `&[u8]`, the two reference shapes that keep serde's zero-copy
/// borrowing path, with an `'a` lifetime injected into the request enum.
fn is_borrowed_serde_ref(ty: &Type) -> bool {
    let Type::Reference(reference) = ty else {
        return false;
    };
    match &*reference.elem {
        Type::Path(path) => path.path.is_ident("str"),
        Type::Slice(slice) => matches!(&*slice.elem, Type::Path(path) if path.path.is_ident("u8")),
        _ => false,
    }
}

/// True if `attr` is a cfg-style attribute (`#[cfg(...)]` or `#[cfg_attr(...)]`).
/// These are propagated onto every generated artifact derived from a method so
/// that rustc strips them in lockstep after macro expansion.
fn is_cfg_attr(attr: &Attribute) -> bool {
    attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr")
}

/// The `#[cfg(...)]` predicates on an item, which decide whether it survives compilation.
/// `#[cfg_attr(...)]` rewrites attributes rather than presence and is not included.
fn cfg_predicates(attrs: &[Attribute]) -> Vec<TokenStream2> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg"))
        .filter_map(|attr| attr.parse_args::<TokenStream2>().ok())
        .collect()
}

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

/// Recursively emit code that encodes a value of type `ty` into a `WireArg`, pushing Javascript
/// values onto `post_args` and, for `Transfer`, onto `transfer_args` as a side effect.
///
/// The emitted code matches on `&value`, so the caller's binding stays usable, and match
/// ergonomics binds `__inner` as a reference inside each arm.
fn emit_encode(
    ty: &Type,
    value: TokenStream2,
    post_args: &TokenStream2,
    transfer_args: &TokenStream2,
) -> TokenStream2 {
    if let Some(inner) = option_inner_type(ty) {
        let inner_encode = emit_encode(inner, quote!(__inner), post_args, transfer_args);
        quote_spanned! {ty.span()=>
            match &#value {
                ::core::option::Option::Some(__inner) =>
                    web_rpc::codec::WireArg::Some(::std::boxed::Box::new(#inner_encode)),
                ::core::option::Option::None =>
                    web_rpc::codec::WireArg::None,
            }
        }
    } else if let Some((ok, err)) = result_inner_types(ty) {
        let ok_encode = emit_encode(ok, quote!(__inner), post_args, transfer_args);
        let err_encode = emit_encode(err, quote!(__inner), post_args, transfer_args);
        quote_spanned! {ty.span()=>
            match &#value {
                ::core::result::Result::Ok(__inner) =>
                    web_rpc::codec::WireArg::Ok(::std::boxed::Box::new(#ok_encode)),
                ::core::result::Result::Err(__inner) =>
                    web_rpc::codec::WireArg::Err(::std::boxed::Box::new(#err_encode)),
            }
        }
    } else if let Some((_, transfer)) = js_inner_type(ty) {
        let push_transfer = transfer.then(|| {
            quote! { (#transfer_args).push(web_rpc::wrap::js_value(&#value.0)); }
        });
        quote_spanned! {ty.span()=>
            {
                (#post_args).push(web_rpc::wrap::js_value(&#value.0));
                #push_transfer
                web_rpc::codec::WireArg::Js
            }
        }
    } else {
        quote_spanned! {ty.span()=>
            web_rpc::codec::WireArg::Bytes(
                web_rpc::postcard::to_allocvec(&#value).unwrap()
            )
        }
    }
}

/// Recursively emit code that decodes a `WireArg` of type `ty` into a Rust value, shifting
/// Javascript values off `js_values` as needed.
fn emit_decode(ty: &Type, wire: TokenStream2, js_values: &TokenStream2) -> TokenStream2 {
    if let Some(inner) = option_inner_type(ty) {
        let inner_decode = emit_decode(inner, quote!(*__inner), js_values);
        quote_spanned! {ty.span()=>
            match #wire {
                web_rpc::codec::WireArg::Some(__inner) =>
                    ::core::option::Option::Some(#inner_decode),
                web_rpc::codec::WireArg::None =>
                    ::core::option::Option::None,
                _ => panic!("web_rpc: wire/type mismatch, expected Some or None"),
            }
        }
    } else if let Some((ok, err)) = result_inner_types(ty) {
        let ok_decode = emit_decode(ok, quote!(*__inner), js_values);
        let err_decode = emit_decode(err, quote!(*__inner), js_values);
        quote_spanned! {ty.span()=>
            match #wire {
                web_rpc::codec::WireArg::Ok(__inner) =>
                    ::core::result::Result::Ok(#ok_decode),
                web_rpc::codec::WireArg::Err(__inner) =>
                    ::core::result::Result::Err(#err_decode),
                _ => panic!("web_rpc: wire/type mismatch, expected Ok or Err"),
            }
        }
    } else if let Some((inner, _)) = js_inner_type(ty) {
        quote_spanned! {ty.span()=>
            match #wire {
                web_rpc::codec::WireArg::Js => <#ty>::new(
                    web_rpc::wasm_bindgen::JsCast::dyn_into::<#inner>((#js_values).shift()).unwrap()
                ),
                _ => panic!("web_rpc: wire/type mismatch, expected a Javascript value"),
            }
        }
    } else {
        quote_spanned! {ty.span()=>
            match #wire {
                web_rpc::codec::WireArg::Bytes(__bytes) =>
                    web_rpc::postcard::from_bytes::<#ty>(&__bytes).unwrap(),
                _ => panic!("web_rpc: wire/type mismatch, expected postcard bytes"),
            }
        }
    }
}

/// Emit the `&'static Desc` describing how a value of type `ty` crosses the channel.
///
/// The `Schema` and `JsName` bounds are expressed at the signature type's own span, so a
/// missing derive is reported at the argument or return type rather than inside the expansion.
fn emit_desc(ty: &Type) -> TokenStream2 {
    if let Some(inner) = option_inner_type(ty) {
        let inner_desc = emit_desc(inner);
        quote_spanned!(ty.span()=> &web_rpc::describe::Desc::Option(#inner_desc))
    } else if let Some((ok, err)) = result_inner_types(ty) {
        let ok_desc = emit_desc(ok);
        let err_desc = emit_desc(err);
        quote_spanned!(ty.span()=> &web_rpc::describe::Desc::Result(#ok_desc, #err_desc))
    } else if let Some((inner, transfer)) = js_inner_type(ty) {
        quote_spanned! {inner.span()=>
            &web_rpc::describe::Desc::Js {
                name: <#inner as web_rpc::describe::JsName>::NAME,
                transfer: #transfer,
            }
        }
    } else if is_borrowed_serde_ref(ty) {
        // postcard-schema implements `Schema` for `[T]` but not for `str`; the borrowed forms
        // encode identically to their owned counterparts.
        let schema_ty: Type = match ty {
            Type::Reference(reference) => match &*reference.elem {
                Type::Path(path) if path.path.is_ident("str") => {
                    parse_quote!(::std::string::String)
                }
                other => other.clone(),
            },
            other => other.clone(),
        };
        quote_spanned! {ty.span()=>
            &web_rpc::describe::Desc::Inline(
                <#schema_ty as web_rpc::postcard_schema::Schema>::SCHEMA
            )
        }
    } else {
        quote_spanned! {ty.span()=>
            &web_rpc::describe::Desc::Postcard(
                <#ty as web_rpc::postcard_schema::Schema>::SCHEMA
            )
        }
    }
}

// ---------------------------------------------------------------------------
// The parsed trait
// ---------------------------------------------------------------------------

struct Service {
    attrs: Vec<Attribute>,
    vis: Visibility,
    ident: Ident,
    methods: Vec<RpcMethod>,
}

/// What a method sends back.
enum MethodOutput {
    Notify,
    Value(Type),
    Stream(Type),
}

struct RpcMethod {
    is_async: Option<Token![async]>,
    attrs: Vec<Attribute>,
    receiver: syn::Receiver,
    ident: Ident,
    args: Vec<PatType>,
    output: MethodOutput,
}

impl RpcMethod {
    /// The identifiers of the arguments. Patterns are rejected at parse time, so every
    /// argument has one.
    fn argument_idents(&self) -> impl Iterator<Item = &Ident> {
        self.args.iter().map(|argument| match &*argument.pat {
            Pat::Ident(pattern) => &pattern.ident,
            _ => unreachable!("argument patterns are rejected while parsing"),
        })
    }

    /// The `#[cfg]` and `#[cfg_attr]` attributes, propagated to everything derived from
    /// this method.
    fn cfg_attrs(&self) -> impl Iterator<Item = &Attribute> {
        self.attrs.iter().filter(|attr| is_cfg_attr(attr))
    }

    /// The name of this method's variant in the request and response enums.
    fn variant_ident(&self) -> Ident {
        Ident::new(
            &snake_to_camel(&self.ident.unraw().to_string()),
            self.ident.span(),
        )
    }

    /// The name of this method on the Javascript side.
    fn wire_name(&self) -> String {
        to_lower_camel(&self.ident.unraw().to_string())
    }

    /// The return type as written in the generated trait and forwarding impls.
    fn return_tokens(&self) -> TokenStream2 {
        match &self.output {
            MethodOutput::Notify => quote!(),
            MethodOutput::Value(ty) => quote!(-> #ty),
            MethodOutput::Stream(item) => {
                quote!(-> impl web_rpc::futures_core::Stream<Item = #item>)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Code generation
// ---------------------------------------------------------------------------

struct ServiceGenerator<'a> {
    trait_ident: &'a Ident,
    service_ident: Ident,
    client_ident: Ident,
    request_ident: Ident,
    response_ident: Ident,
    description_ident: Ident,
    vis: &'a Visibility,
    attrs: &'a [Attribute],
    methods: &'a [RpcMethod],
}

impl ServiceGenerator<'_> {
    fn enum_request(&self) -> TokenStream2 {
        let Self {
            vis,
            request_ident,
            methods,
            ..
        } = self;
        let variants = methods.iter().map(|method| {
            let cfg_attrs = method.cfg_attrs();
            let variant_ident = method.variant_ident();
            let fields = method.args.iter().map(|argument| {
                let pat = &argument.pat;
                if is_borrowed_serde_ref(&argument.ty) {
                    // `&str` / `&[u8]` keep the zero-copy serde borrowing path.
                    let Type::Reference(reference) = &*argument.ty else {
                        unreachable!("is_borrowed_serde_ref guarantees a reference")
                    };
                    let mut reference = reference.clone();
                    reference.lifetime = Some(Lifetime::new("'a", reference.and_token.span()));
                    quote_spanned! {argument.ty.span()=> #pat: #reference }
                } else {
                    quote_spanned! {argument.ty.span()=> #pat: web_rpc::codec::WireArg }
                }
            });
            quote! {
                #(#cfg_attrs)*
                #variant_ident { #( #fields ),* }
            }
        });
        // The hidden variant uses `'a` so that the enum stays well-formed when every borrowing
        // method is stripped by cfg. It is never constructed; the server's match arm panics on
        // it.
        quote! {
            #[derive(web_rpc::serde::Serialize, web_rpc::serde::Deserialize)]
            #vis enum #request_ident<'a> {
                #( #variants, )*
                #[doc(hidden)]
                __WebRpcPhantom(::std::marker::PhantomData<&'a ()>),
            }
        }
    }

    fn enum_response(&self) -> TokenStream2 {
        let Self {
            vis,
            response_ident,
            methods,
            ..
        } = self;
        // Every method gets a variant so that variant indices match the request enum. A
        // notification's variant is never constructed.
        let variants = methods.iter().map(|method| {
            let cfg_attrs = method.cfg_attrs();
            let variant_ident = method.variant_ident();
            quote! {
                #(#cfg_attrs)*
                #variant_ident ( web_rpc::codec::WireArg )
            }
        });
        quote! {
            #[derive(web_rpc::serde::Serialize, web_rpc::serde::Deserialize)]
            #vis enum #response_ident {
                #( #variants ),*
            }
        }
    }

    /// The compile-time description of the trait, from which `js::endpoint!` renders
    /// Javascript and Typescript.
    fn const_description(&self) -> TokenStream2 {
        let Self {
            vis,
            attrs,
            trait_ident,
            description_ident,
            methods,
            ..
        } = self;

        let method_const_idents = (0..methods.len())
            .map(|index| format_ident!("__WEB_RPC_{}_M{}", screaming_snake(trait_ident), index))
            .collect::<Vec<_>>();

        let method_consts =
            methods
                .iter()
                .zip(&method_const_idents)
                .map(|(method, const_ident)| {
                    let args = method.args.iter().zip(method.argument_idents()).map(
                        |(argument, ident)| {
                            let name = to_lower_camel(&ident.unraw().to_string());
                            let desc = emit_desc(&argument.ty);
                            quote! { web_rpc::describe::Arg { name: #name, desc: #desc } }
                        },
                    );
                    let ret = match &method.output {
                        MethodOutput::Notify => quote!(web_rpc::describe::Return::Notify),
                        MethodOutput::Value(ty) => {
                            let desc = emit_desc(ty);
                            quote!(web_rpc::describe::Return::Value(#desc))
                        }
                        MethodOutput::Stream(item) => {
                            let desc = emit_desc(item);
                            quote!(web_rpc::describe::Return::Stream(#desc))
                        }
                    };
                    let wire_name = method.wire_name();
                    let predicates = cfg_predicates(&method.attrs);
                    let (enabled, disabled) = if predicates.is_empty() {
                        (quote!(), quote!(#[cfg(any())]))
                    } else {
                        (
                            quote!(#[cfg(all(#( #predicates ),*))]),
                            quote!(#[cfg(not(all(#( #predicates ),*)))]),
                        )
                    };
                    quote! {
                        #enabled
                        #[doc(hidden)]
                        const #const_ident: &'static [web_rpc::describe::Method] =
                            &[web_rpc::describe::Method {
                                name: #wire_name,
                                args: &[ #( #args ),* ],
                                ret: #ret,
                            }];
                        #disabled
                        #[doc(hidden)]
                        const #const_ident: &'static [web_rpc::describe::Method] = &[];
                    }
                });

        let trait_name = trait_ident.to_string();
        let trait_cfgs = attrs
            .iter()
            .filter(|attr| is_cfg_attr(attr))
            .collect::<Vec<_>>();
        quote! {
            #( #method_consts )*
            #( #trait_cfgs )*
            #[doc(hidden)]
            #[allow(non_upper_case_globals)]
            #vis const #description_ident: &'static web_rpc::describe::Service =
                &web_rpc::describe::Service {
                    name: #trait_name,
                    methods: &[ #( #method_const_idents ),* ],
                };
        }
    }

    fn trait_service(&self) -> TokenStream2 {
        let Self {
            attrs,
            methods,
            vis,
            trait_ident,
            ..
        } = self;

        let declarations = methods.iter().map(|method| {
            let RpcMethod {
                attrs,
                args,
                receiver,
                ident,
                is_async,
                ..
            } = method;
            let output = method.return_tokens();
            quote_spanned! {ident.span()=>
                #( #attrs )*
                #is_async fn #ident(#receiver, #( #args ),*) #output;
            }
        });

        let forwards = methods
            .iter()
            .map(|method| {
                let RpcMethod {
                    attrs,
                    args,
                    receiver,
                    ident,
                    is_async,
                    ..
                } = method;
                let output = method.return_tokens();
                let do_await = is_async.map(|token| quote_spanned!(token.span=> .await));
                let argument_idents = method.argument_idents();
                quote_spanned! {ident.span()=>
                    #( #attrs )*
                    #is_async fn #ident(#receiver, #( #args ),*) #output {
                        T::#ident(self, #( #argument_idents ),*)#do_await
                    }
                }
            })
            .collect::<Vec<_>>();

        quote! {
            #( #attrs )*
            #[allow(async_fn_in_trait)]
            #vis trait #trait_ident {
                #( #declarations )*
            }

            impl<T> #trait_ident for ::std::sync::Arc<T> where T: #trait_ident {
                #( #forwards )*
            }
            impl<T> #trait_ident for ::std::boxed::Box<T> where T: #trait_ident {
                #( #forwards )*
            }
            impl<T> #trait_ident for ::std::rc::Rc<T> where T: #trait_ident {
                #( #forwards )*
            }
        }
    }

    fn struct_client(&self) -> TokenStream2 {
        let Self {
            vis,
            client_ident,
            request_ident,
            response_ident,
            methods,
            ..
        } = self;

        let rpc_fns = methods.iter().map(|method| {
            let RpcMethod {
                attrs, args, ident, ..
            } = method;
            let variant_ident = method.variant_ident();

            // Borrowed `&str`/`&[u8]` pass through inline; everything else becomes a
            // `WireArg`, pushing onto the post and transfer arrays as it goes.
            let mut encodings = Vec::new();
            let mut request_fields = Vec::new();
            for (argument, argument_ident) in args.iter().zip(method.argument_idents()) {
                if is_borrowed_serde_ref(&argument.ty) {
                    request_fields.push(quote! { #argument_ident });
                } else {
                    let wire_ident = format_ident!("__wire_{}", argument_ident);
                    let encode = emit_encode(
                        &argument.ty,
                        quote!(#argument_ident),
                        &quote!(&__post_args),
                        &quote!(&__transfer_args),
                    );
                    encodings.push(quote! { let #wire_ident = #encode; });
                    request_fields.push(quote! { #argument_ident: #wire_ident });
                }
            }

            let send = quote! {
                let __post_args = web_rpc::js_sys::Array::new();
                let __transfer_args = web_rpc::js_sys::Array::new();
                #( #encodings )*
                let __request = #request_ident::#variant_ident { #( #request_fields ),* };
                let __sequence = self.state.send(&__request, &__post_args, &__transfer_args);
            };

            let unpack = |ty: &Type| {
                let decode = emit_decode(ty, quote!(__wire), &quote!(&__js_values));
                quote! {
                    |__response: #response_ident, __js_values: web_rpc::js_sys::Array| {
                        let #response_ident::#variant_ident(__wire) = __response else {
                            panic!("web_rpc: received a response for another method")
                        };
                        #decode
                    }
                }
            };

            let (return_type, body) = match &method.output {
                MethodOutput::Notify => (quote!(()), quote! { #send }),
                MethodOutput::Value(ty) => {
                    let unpack = unpack(ty);
                    (
                        quote!(web_rpc::client::RequestFuture<#ty>),
                        quote! {
                            #send
                            self.state.request(__sequence, #unpack)
                        },
                    )
                }
                MethodOutput::Stream(item) => {
                    let unpack = unpack(item);
                    (
                        quote!(web_rpc::client::StreamReceiver<#item>),
                        quote! {
                            #send
                            self.state.stream(__sequence, #unpack)
                        },
                    )
                }
            };

            quote! {
                #( #attrs )*
                #vis fn #ident(&self, #( #args ),*) -> #return_type {
                    #body
                }
            }
        });

        quote! {
            #[derive(::core::clone::Clone)]
            #vis struct #client_ident {
                state: web_rpc::client::State<#response_ident>,
            }
            impl ::std::fmt::Debug for #client_ident {
                fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                    formatter.debug_struct(::std::stringify!(#client_ident)).finish()
                }
            }
            impl web_rpc::client::Client for #client_ident {
                type Response = #response_ident;
            }
            impl ::std::convert::From<web_rpc::client::State<#response_ident>> for #client_ident {
                fn from(state: web_rpc::client::State<#response_ident>) -> Self {
                    Self { state }
                }
            }
            impl #client_ident {
                #( #rpc_fns )*
            }
        }
    }

    fn struct_server(&self) -> TokenStream2 {
        let Self {
            vis,
            trait_ident,
            service_ident,
            request_ident,
            response_ident,
            methods,
            ..
        } = self;

        let handlers = methods.iter().map(|method| {
            let RpcMethod {
                is_async,
                ident,
                args,
                ..
            } = method;
            let cfg_attrs = method.cfg_attrs();
            let variant_ident = method.variant_ident();

            // Destructure the request variant. Borrowed arguments bind to their own ident;
            // everything else binds to `__wire_<ident>` and is decoded below.
            let mut destructure_fields = Vec::new();
            let mut decodings = Vec::new();
            for (argument, argument_ident) in args.iter().zip(method.argument_idents()) {
                if is_borrowed_serde_ref(&argument.ty) {
                    destructure_fields.push(quote! { #argument_ident });
                } else {
                    let wire_ident = format_ident!("__wire_{}", argument_ident);
                    let decode =
                        emit_decode(&argument.ty, quote!(#wire_ident), &quote!(&__js_args));
                    destructure_fields.push(quote! { #argument_ident: #wire_ident });
                    decodings.push(quote! { let #argument_ident = #decode; });
                }
            }
            let argument_idents = method.argument_idents().collect::<Vec<_>>();
            let call = quote! { self.implementation.#ident(#( #argument_idents ),*) };

            let encode_outgoing = |ty: &Type, value: TokenStream2| {
                let encode = emit_encode(
                    ty,
                    value,
                    &quote!(&__post_args),
                    &quote!(&__transfer_args),
                );
                quote! {
                    let __post_args = web_rpc::js_sys::Array::new();
                    let __transfer_args = web_rpc::js_sys::Array::new();
                    let __wire = #encode;
                    (#response_ident::#variant_ident(__wire), __post_args, __transfer_args)
                }
            };

            let body = match (&method.output, is_async) {
                (MethodOutput::Notify, None) => quote! {
                    #call;
                    web_rpc::service::ExecuteResult::Response(None)
                },
                (MethodOutput::Notify, Some(_)) => quote! {
                    #call.await;
                    web_rpc::service::ExecuteResult::Response(None)
                },
                (MethodOutput::Value(ty), None) => {
                    let outgoing = encode_outgoing(ty, quote!(__response));
                    quote! {
                        let __response = #call;
                        web_rpc::service::ExecuteResult::Response(Some({ #outgoing }))
                    }
                }
                (MethodOutput::Value(ty), Some(_)) => {
                    let outgoing = encode_outgoing(ty, quote!(__response));
                    quote! {
                        let mut __task = ::std::pin::pin!(web_rpc::futures_util::FutureExt::fuse(#call));
                        web_rpc::service::ExecuteResult::Response(
                            web_rpc::futures_util::select! {
                                _ = __abort_rx => None,
                                __response = __task => Some({ #outgoing }),
                            }
                        )
                    }
                }
                (MethodOutput::Stream(item), is_async) => {
                    let outgoing = encode_outgoing(item, quote!(__item));
                    let forward = quote! {
                        let mut __items = ::std::pin::pin!(__items);
                        let mut __forward = ::std::pin::pin!(web_rpc::futures_util::FutureExt::fuse(async {
                            while let Some(__item) = web_rpc::futures_util::StreamExt::next(&mut __items).await {
                                let __outgoing = { #outgoing };
                                if __stream_tx.unbounded_send((__sequence, Some(__outgoing))).is_err() {
                                    break;
                                }
                            }
                        }));
                        web_rpc::futures_util::select! {
                            _ = __abort_rx => {},
                            _ = __forward => {},
                        }
                        let _ = __stream_tx.unbounded_send((__sequence, None));
                        web_rpc::service::ExecuteResult::StreamComplete
                    };
                    match is_async {
                        None => quote! {
                            let __items = #call;
                            #forward
                        },
                        Some(_) => quote! {
                            let mut __task = ::std::pin::pin!(web_rpc::futures_util::FutureExt::fuse(#call));
                            let __items = web_rpc::futures_util::select! {
                                _ = __abort_rx => None,
                                __items = __task => Some(__items),
                            };
                            match __items {
                                Some(__items) => { #forward }
                                None => {
                                    let _ = __stream_tx.unbounded_send((__sequence, None));
                                    web_rpc::service::ExecuteResult::StreamComplete
                                }
                            }
                        },
                    }
                }
            };

            quote! {
                #( #cfg_attrs )*
                #request_ident::#variant_ident { #( #destructure_fields ),* } => {
                    #( #decodings )*
                    #body
                }
            }
        });

        quote! {
            #vis struct #service_ident<T> {
                implementation: T
            }
            impl<T: #trait_ident> web_rpc::service::Service for #service_ident<T> {
                type Response = #response_ident;
                #[allow(unused_mut, unused_variables)]
                async fn execute(
                    &self,
                    __sequence: u32,
                    mut __abort_rx: web_rpc::futures_channel::oneshot::Receiver<()>,
                    __payload: ::std::vec::Vec<u8>,
                    __js_args: web_rpc::js_sys::Array,
                    __stream_tx: web_rpc::futures_channel::mpsc::UnboundedSender<
                        web_rpc::service::StreamMessage<Self::Response>
                    >,
                ) -> (u32, web_rpc::service::ExecuteResult<Self::Response>) {
                    let __request: #request_ident<'_> =
                        web_rpc::postcard::from_bytes(&__payload).unwrap();
                    let __result = match __request {
                        #( #handlers )*
                        #request_ident::__WebRpcPhantom(_) => {
                            unreachable!("web_rpc: __WebRpcPhantom variant received on wire")
                        }
                    };
                    (__sequence, __result)
                }
            }
            impl<T: #trait_ident> ::std::convert::From<T> for #service_ident<T> {
                fn from(implementation: T) -> Self {
                    Self { implementation }
                }
            }
        }
    }
}

impl ToTokens for ServiceGenerator<'_> {
    fn to_tokens(&self, output: &mut TokenStream2) {
        output.extend([
            self.enum_request(),
            self.enum_response(),
            self.const_description(),
            self.trait_service(),
            self.struct_client(),
            self.struct_server(),
        ])
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

impl Parse for Service {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis = input.parse()?;
        input.parse::<Token![trait]>()?;
        let ident: Ident = input.parse()?;
        let content;
        braced!(content in input);
        let mut methods = Vec::new();
        while !content.is_empty() {
            methods.push(content.parse()?);
        }
        Ok(Self {
            attrs,
            vis,
            ident,
            methods,
        })
    }
}

impl Parse for RpcMethod {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut errors = Ok(());
        let attrs = input.call(Attribute::parse_outer)?;

        let is_async = input.parse::<Token![async]>().ok();
        input.parse::<Token![fn]>()?;
        let ident: Ident = input.parse()?;

        // Reject generic methods up front: the description needs concrete types.
        if input.peek(Token![<]) {
            let generics: syn::Generics = input.parse()?;
            extend_errors!(
                errors,
                syn::Error::new_spanned(
                    generics,
                    "web_rpc::service trait methods may not have generic parameters; \
                     concrete types are required so the macro can route and describe each \
                     argument."
                )
            );
        }

        let content;
        parenthesized!(content in input);
        let mut receiver: Option<syn::Receiver> = None;
        let mut args = Vec::new();
        for argument in content.parse_terminated(FnArg::parse, Token![,])? {
            match argument {
                FnArg::Typed(typed) => match &*typed.pat {
                    Pat::Ident(_) => args.push(typed),
                    _ => extend_errors!(
                        errors,
                        syn::Error::new(
                            typed.pat.span(),
                            "patterns are not allowed in RPC arguments"
                        )
                    ),
                },
                FnArg::Receiver(ref parsed) => {
                    if parsed.reference.is_none() || parsed.mutability.is_some() {
                        extend_errors!(
                            errors,
                            syn::Error::new(
                                argument.span(),
                                "RPC methods only support `&self` as a receiver"
                            )
                        );
                    }
                    receiver = Some(parsed.clone());
                }
            }
        }
        let receiver = receiver.unwrap_or_else(|| {
            extend_errors!(
                errors,
                syn::Error::new(
                    ident.span(),
                    "RPC methods must include `&self` as the first parameter"
                )
            );
            parse_quote!(&self)
        });
        let output = match input.parse::<ReturnType>()? {
            ReturnType::Default => MethodOutput::Notify,
            ReturnType::Type(_, ty) => match stream_item_type(&ty) {
                Some(item) => MethodOutput::Stream(item.clone()),
                None => MethodOutput::Value(*ty),
            },
        };
        input.parse::<Token![;]>()?;
        errors?;

        Ok(Self {
            is_async,
            attrs,
            receiver,
            ident,
            args,
            output,
        })
    }
}

/// This attribute macro should be applied to traits that need to be turned into RPCs. The macro
/// consumes the trait and outputs four items in its place. For a trait `Calculator` those are
/// the structs `CalculatorClient` and `CalculatorService`, a new trait by the same name, and a
/// `CALCULATOR_DESCRIPTION` const describing the trait for
/// [`web_rpc::js::endpoint!`](../web_rpc/js/macro.endpoint.html). All methods must include
/// `&self` as their first parameter.
#[proc_macro_attribute]
pub fn service(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let Service {
        ref attrs,
        ref vis,
        ref ident,
        ref methods,
    } = parse_macro_input!(input as Service);

    ServiceGenerator {
        trait_ident: ident,
        service_ident: format_ident!("{}Service", ident),
        client_ident: format_ident!("{}Client", ident),
        request_ident: format_ident!("{}Request", ident),
        response_ident: format_ident!("{}Response", ident),
        description_ident: format_ident!("{}_DESCRIPTION", screaming_snake(ident)),
        vis,
        attrs,
        methods,
    }
    .into_token_stream()
    .into()
}

// ---------------------------------------------------------------------------
// js::endpoint!
// ---------------------------------------------------------------------------

/// The parsed arguments of `js::endpoint!`.
struct EndpointArgs {
    service: Option<Path>,
    client: Option<Path>,
}

impl Parse for EndpointArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut service = None;
        let mut client = None;
        for entry in Punctuated::<EndpointArg, Token![,]>::parse_terminated(input)? {
            let (slot, path, key) = match entry {
                EndpointArg::Service(path) => (&mut service, path, "service"),
                EndpointArg::Client(path) => (&mut client, path, "client"),
            };
            if slot.replace(path).is_some() {
                return Err(syn::Error::new(
                    input.span(),
                    format!("`{key}` is given more than once"),
                ));
            }
        }
        if service.is_none() && client.is_none() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "a Javascript endpoint needs at least one of `service = ...` (the trait it \
                 implements) and `client = ...` (the trait it calls)",
            ));
        }
        Ok(Self { service, client })
    }
}

enum EndpointArg {
    Service(Path),
    Client(Path),
}

impl Parse for EndpointArg {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let key: Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        if key == "service" {
            Ok(EndpointArg::Service(input.parse()?))
        } else if key == "client" {
            Ok(EndpointArg::Client(input.parse()?))
        } else {
            Err(syn::Error::new(
                key.span(),
                "expected `service` or `client`",
            ))
        }
    }
}

/// Rewrite `some::path::FooService` (or `FooClient`) into `some::path::FOO_DESCRIPTION`.
fn description_path(path: &Path) -> syn::Result<Path> {
    let span = path.span();
    let mut path = path.clone();
    let last = path
        .segments
        .last_mut()
        .ok_or_else(|| syn::Error::new(span, "expected a generated Service or Client"))?;
    let name = last.ident.to_string();
    let trait_name = name
        .strip_suffix("Service")
        .or_else(|| name.strip_suffix("Client"))
        .filter(|trait_name| !trait_name.is_empty())
        .ok_or_else(|| {
            syn::Error::new(
                last.ident.span(),
                "expected a name generated by #[web_rpc::service], which ends in `Service` or \
                 `Client`",
            )
        })?;
    last.ident = format_ident!(
        "{}_DESCRIPTION",
        screaming_snake_str(trait_name),
        span = last.ident.span()
    );
    last.arguments = syn::PathArguments::None;
    Ok(path)
}

/// Render a Javascript endpoint and a `.d.ts` for the other end of a connection into two custom
/// sections of the wasm binary. See the [`web_rpc::js`](../web_rpc/js/index.html) module.
#[proc_macro]
pub fn endpoint(input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(input as EndpointArgs);

    let class_path = args.client.as_ref().or(args.service.as_ref()).unwrap();
    let class_name = class_path.segments.last().unwrap().ident.to_string();
    let snake = snake_case_str(&class_name);
    let screaming = screaming_snake_str(&class_name);

    let description = |path: Option<&Path>| match path.map(description_path) {
        Some(Ok(path)) => Ok(quote!(::core::option::Option::Some(#path))),
        Some(Err(error)) => Err(error.to_compile_error()),
        None => Ok(quote!(::core::option::Option::None)),
    };
    let service_description = match description(args.service.as_ref()) {
        Ok(tokens) => tokens,
        Err(error) => return error.into(),
    };
    let client_description = match description(args.client.as_ref()) {
        Ok(tokens) => tokens,
        Err(error) => return error.into(),
    };

    let endpoint_ident = format_ident!("__WEB_RPC_ENDPOINT_{screaming}");
    let js_length_ident = format_ident!("__WEB_RPC_ENDPOINT_{screaming}_JS_LENGTH");
    let js_ident = format_ident!("__WEB_RPC_ENDPOINT_{screaming}_JS");
    let dts_length_ident = format_ident!("__WEB_RPC_ENDPOINT_{screaming}_DTS_LENGTH");
    let dts_ident = format_ident!("__WEB_RPC_ENDPOINT_{screaming}_DTS");
    let guard_ident = format_ident!("__web_rpc_{snake}");
    let js_section = format!("__web_rpc_{snake}_js");
    let dts_section = format!("__web_rpc_{snake}_d_ts");

    quote! {
        #[doc(hidden)]
        const #endpoint_ident: web_rpc::js::Endpoint = web_rpc::js::Endpoint {
            class: #class_name,
            service: #service_description,
            client: #client_description,
        };
        #[doc(hidden)]
        const #js_length_ident: usize = web_rpc::js::render_js::<0>(&#endpoint_ident).length;
        #[doc(hidden)]
        #[allow(long_running_const_eval)]
        const #js_ident: [u8; #js_length_ident] =
            web_rpc::js::render_js::<#js_length_ident>(&#endpoint_ident).bytes;
        #[doc(hidden)]
        const #dts_length_ident: usize = web_rpc::js::render_dts::<0>(&#endpoint_ident).length;
        #[doc(hidden)]
        #[allow(long_running_const_eval)]
        const #dts_ident: [u8; #dts_length_ident] =
            web_rpc::js::render_dts::<#dts_length_ident>(&#endpoint_ident).bytes;

        const _: () = {
            #[used]
            #[link_section = #js_section]
            static JS: [u8; #js_length_ident] = #js_ident;
            #[used]
            #[link_section = #dts_section]
            static D_TS: [u8; #dts_length_ident] = #dts_ident;
            // Two endpoints with the same class name in one binary would concatenate into the
            // same custom section, so make that a duplicate symbol error instead.
            #[no_mangle]
            static #guard_ident: u8 = 0;
        };
    }
    .into()
}

// ---------------------------------------------------------------------------
// Name conversions
// ---------------------------------------------------------------------------

/// `add_numbers` becomes `AddNumbers`: the variant name of a method.
fn snake_to_camel(name: &str) -> String {
    let mut camel = String::with_capacity(name.len());
    let mut capitalize_next = true;
    for character in name.chars() {
        match character {
            '_' => capitalize_next = true,
            character if capitalize_next => {
                camel.extend(character.to_uppercase());
                capitalize_next = false;
            }
            character => camel.extend(character.to_lowercase()),
        }
    }
    camel
}

/// `add_numbers` becomes `addNumbers`: the wire name of a method or an argument.
fn to_lower_camel(name: &str) -> String {
    let camel = snake_to_camel(name);
    let mut characters = camel.chars();
    match characters.next() {
        Some(first) => first.to_lowercase().chain(characters).collect(),
        None => camel,
    }
}

/// `FooBar` becomes `FOO_BAR`: the prefix of the description const.
fn screaming_snake(ident: &Ident) -> String {
    screaming_snake_str(&ident.to_string())
}

fn screaming_snake_str(name: &str) -> String {
    snake_case_str(name).to_uppercase()
}

/// `FooBar` becomes `foo_bar`: the section name of an endpoint.
fn snake_case_str(name: &str) -> String {
    let mut snake = String::with_capacity(name.len() + 4);
    for (index, character) in name.chars().enumerate() {
        if character.is_uppercase() && index > 0 {
            snake.push('_');
        }
        snake.extend(character.to_lowercase());
    }
    snake
}
