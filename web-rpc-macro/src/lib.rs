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
    Attribute, FnArg, Ident, Lifetime, Meta, NestedMeta, Pat, PatType, ReturnType, Token, Type,
    Visibility,
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

/// Describes how a posted type is wrapped.
enum PostWrapperKind<'a> {
    /// Bare JS type, e.g., `JsString`
    Bare,
    /// `Option<JsType>`
    Option { inner_ty: &'a Type },
    /// `Result<JsType, E>`
    Result { ok_ty: &'a Type, err_ty: &'a Type },
}

fn post_wrapper_kind(ty: &Type) -> PostWrapperKind<'_> {
    if let Some(inner) = option_inner_type(ty) {
        PostWrapperKind::Option { inner_ty: inner }
    } else if let Some((ok_ty, err_ty)) = result_inner_types(ty) {
        PostWrapperKind::Result { ok_ty, err_ty }
    } else {
        PostWrapperKind::Bare
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
    transfer: HashSet<Ident>,
    post: HashSet<Ident>,
    output: ReturnType,
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
    has_borrowed_args: bool,
    has_streaming_methods: bool,
}

impl<'a> ServiceGenerator<'a> {
    fn enum_request(&self) -> TokenStream2 {
        let &Self {
            vis,
            request_ident,
            camel_case_idents,
            rpcs,
            has_borrowed_args,
            ..
        } = self;
        let lifetime = if has_borrowed_args {
            quote!(<'a>)
        } else {
            quote!()
        };
        let variants = rpcs.iter().zip(camel_case_idents.iter()).map(
            |(RpcMethod { args, post, .. }, camel_case_ident)| {
                let fields = args.iter().filter_map(|arg| {
                    let is_post =
                        matches!(&*arg.pat, Pat::Ident(ident) if post.contains(&ident.ident));
                    if is_post {
                        // For wrapped post args, include a discriminant field
                        let pat = &arg.pat;
                        match post_wrapper_kind(&arg.ty) {
                            PostWrapperKind::Option { .. } => Some(quote! { #pat: bool }),
                            PostWrapperKind::Result { .. } => Some(quote! { #pat: bool }),
                            PostWrapperKind::Bare => None, // bare post args excluded
                        }
                    } else {
                        // Non-post args included as-is (with lifetime fix for borrowed)
                        Some(if has_borrowed_args {
                            if let Type::Reference(type_ref) = &*arg.ty {
                                let mut type_ref = type_ref.clone();
                                type_ref.lifetime =
                                    Some(Lifetime::new("'a", type_ref.and_token.span()));
                                let pat = &arg.pat;
                                quote! { #pat: #type_ref }
                            } else {
                                quote! { #arg }
                            }
                        } else {
                            quote! { #arg }
                        })
                    }
                });
                quote! {
                    #camel_case_ident { #( #fields ),* }
                }
            },
        );
        quote! {
            #[derive(web_rpc::serde::Serialize, web_rpc::serde::Deserialize)]
            #vis enum #request_ident #lifetime {
                #( #variants ),*
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
            |(RpcMethod { output, post, .. }, camel_case_ident)| match output {
                ReturnType::Type(_, ty) if !post.contains(&Ident::new("return", output.span())) => {
                    // For streaming methods, use the inner item type
                    if let Some(item_ty) = stream_item_type(ty) {
                        quote! {
                            #camel_case_ident ( #item_ty )
                        }
                    } else {
                        quote! {
                            #camel_case_ident ( #ty )
                        }
                    }
                }
                ReturnType::Type(_, ty) => {
                    // post(return) — determine the effective type (stream item or raw return)
                    let effective_ty = stream_item_type(ty).unwrap_or(ty);
                    match post_wrapper_kind(effective_ty) {
                        PostWrapperKind::Bare => quote! {
                            #camel_case_ident ( () )
                        },
                        PostWrapperKind::Option { .. } => quote! {
                            #camel_case_ident ( bool )
                        },
                        PostWrapperKind::Result { .. } => quote! {
                            #camel_case_ident ( bool )
                        },
                    }
                }
                _ => quote! {
                    #camel_case_ident ( () )
                },
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
            .map(|(RpcMethod { attrs, args, transfer, post, ident, output, .. }, camel_case_ident)| {
                // Build request struct fields — non-post args use ident directly,
                // wrapped post args use a discriminant value
                let request_struct_fields: Vec<_> = args.iter()
                    .filter_map(|arg| match &*arg.pat {
                        Pat::Ident(pat_ident) => {
                            let id = &pat_ident.ident;
                            if post.contains(id) {
                                match post_wrapper_kind(&arg.ty) {
                                    PostWrapperKind::Bare => None,
                                    PostWrapperKind::Option { .. } => {
                                        Some(quote! { #id: #id.is_some() })
                                    }
                                    PostWrapperKind::Result { .. } => {
                                        Some(quote! { #id: #id.is_ok() })
                                    }
                                }
                            } else {
                                Some(quote! { #id })
                            }
                        }
                        _ => None
                    })
                    .collect();

                // Build JS post/transfer array pushes — bare post args always push,
                // wrapped post args conditionally push
                let post_pushes: Vec<_> = args.iter()
                    .filter_map(|arg| match &*arg.pat {
                        Pat::Ident(pat_ident) if post.contains(&pat_ident.ident) => {
                            let id = &pat_ident.ident;
                            let is_transfer = transfer.contains(&pat_ident.ident);
                            match post_wrapper_kind(&arg.ty) {
                                PostWrapperKind::Bare => {
                                    let transfer_push = if is_transfer {
                                        quote! { __transfer.push(#id.as_ref()); }
                                    } else {
                                        quote! {}
                                    };
                                    Some(quote! {
                                        __post.push(#id.as_ref());
                                        #transfer_push
                                    })
                                }
                                PostWrapperKind::Option { .. } => {
                                    let transfer_push = if is_transfer {
                                        quote! { __transfer.push(__val.as_ref()); }
                                    } else {
                                        quote! {}
                                    };
                                    Some(quote! {
                                        if let Some(ref __val) = #id {
                                            __post.push(__val.as_ref());
                                            #transfer_push
                                        }
                                    })
                                }
                                PostWrapperKind::Result { .. } => {
                                    let transfer_push = if is_transfer {
                                        quote! { __transfer.push(__val.as_ref()); }
                                    } else {
                                        quote! {}
                                    };
                                    Some(quote! {
                                        match #id {
                                            Ok(ref __val) => {
                                                __post.push(__val.as_ref());
                                                #transfer_push
                                            }
                                            Err(ref __val) => {
                                                __post.push(__val.as_ref());
                                                #transfer_push
                                            }
                                        }
                                    })
                                }
                            }
                        }
                        _ => None
                    })
                    .collect();

                // Static transfer args (bare post args that are also transfer)
                let bare_transfer_arg_idents: Vec<_> = args.iter()
                    .filter_map(|arg| match &*arg.pat {
                        Pat::Ident(pat_ident) if transfer.contains(&pat_ident.ident)
                            && matches!(post_wrapper_kind(&arg.ty), PostWrapperKind::Bare) =>
                        {
                            Some(&pat_ident.ident)
                        }
                        _ => None
                    })
                    .collect();

                let has_wrapped_post_args = args.iter().any(|arg| {
                    matches!(&*arg.pat, Pat::Ident(pat_ident)
                        if post.contains(&pat_ident.ident)
                        && !matches!(post_wrapper_kind(&arg.ty), PostWrapperKind::Bare))
                });

                // Generate the send_request code
                let send_request = if has_wrapped_post_args {
                    // Use incremental push for post/transfer arrays
                    quote! {
                        let __seq_id = self.seq_id.replace_with(|seq_id| seq_id.wrapping_add(1));
                        let __request = #request_ident::#camel_case_ident {
                            #( #request_struct_fields ),*
                        };
                        let __header = web_rpc::MessageHeader::Request(__seq_id);
                        let __header_bytes = web_rpc::bincode::serialize(&__header).unwrap();
                        let __header_buffer = web_rpc::js_sys::Uint8Array::from(&__header_bytes[..]).buffer();
                        let __payload_bytes = web_rpc::bincode::serialize(&__request).unwrap();
                        let __payload_buffer = web_rpc::js_sys::Uint8Array::from(&__payload_bytes[..]).buffer();
                        let __post = web_rpc::js_sys::Array::new();
                        let __transfer = web_rpc::js_sys::Array::new();
                        __post.push(__header_buffer.as_ref());
                        __post.push(__payload_buffer.as_ref());
                        __transfer.push(__header_buffer.as_ref());
                        __transfer.push(__payload_buffer.as_ref());
                        #( #post_pushes )*
                        self.port.post_message(&__post, &__transfer).unwrap();
                    }
                } else {
                    // Original slice-literal approach for bare post args
                    let bare_post_arg_idents: Vec<_> = args.iter()
                        .filter_map(|arg| match &*arg.pat {
                            Pat::Ident(pat_ident) if post.contains(&pat_ident.ident) => Some(&pat_ident.ident),
                            _ => None
                        })
                        .collect();
                    quote! {
                        let __seq_id = self.seq_id.replace_with(|seq_id| seq_id.wrapping_add(1));
                        let __request = #request_ident::#camel_case_ident {
                            #( #request_struct_fields ),*
                        };
                        let __header = web_rpc::MessageHeader::Request(__seq_id);
                        let __header_bytes = web_rpc::bincode::serialize(&__header).unwrap();
                        let __header_buffer = web_rpc::js_sys::Uint8Array::from(&__header_bytes[..]).buffer();
                        let __payload_bytes = web_rpc::bincode::serialize(&__request).unwrap();
                        let __payload_buffer = web_rpc::js_sys::Uint8Array::from(&__payload_bytes[..]).buffer();
                        let __post: &[&web_rpc::wasm_bindgen::JsValue] =
                            &[__header_buffer.as_ref(), __payload_buffer.as_ref(), #( #bare_post_arg_idents.as_ref() ),*];
                        let __post = web_rpc::js_sys::Array::from_iter(__post);
                        let __transfer: &[&web_rpc::wasm_bindgen::JsValue] =
                            &[__header_buffer.as_ref(), __payload_buffer.as_ref(), #( #bare_transfer_arg_idents.as_ref() ),*];
                        let __transfer = web_rpc::js_sys::Array::from_iter(__transfer);
                        self.port.post_message(&__post, &__transfer).unwrap();
                    }
                };

                // Check if this is a streaming method
                let is_streaming = matches!(output, ReturnType::Type(_, ref ty) if stream_item_type(ty).is_some());

                if is_streaming {
                    let item_ty = match output {
                        ReturnType::Type(_, ref ty) => stream_item_type(ty).unwrap(),
                        _ => unreachable!(),
                    };

                    let unpack_stream_item = if post.contains(&Ident::new("return", output.span())) {
                        match post_wrapper_kind(item_ty) {
                            PostWrapperKind::Bare => quote! {
                                |(_response, __post_array)| {
                                    web_rpc::wasm_bindgen::JsCast::dyn_into::<#item_ty>(__post_array.shift())
                                        .unwrap()
                                }
                            },
                            PostWrapperKind::Option { inner_ty } => quote! {
                                |(__response, __post_array)| {
                                    let #response_ident::#camel_case_ident(__has_value) = __response else {
                                        panic!("received incorrect response variant")
                                    };
                                    if __has_value {
                                        Some(web_rpc::wasm_bindgen::JsCast::dyn_into::<#inner_ty>(__post_array.shift())
                                            .unwrap())
                                    } else {
                                        None
                                    }
                                }
                            },
                            PostWrapperKind::Result { ok_ty, err_ty } => quote! {
                                |(__response, __post_array)| {
                                    let #response_ident::#camel_case_ident(__is_ok) = __response else {
                                        panic!("received incorrect response variant")
                                    };
                                    if __is_ok {
                                        Ok(web_rpc::wasm_bindgen::JsCast::dyn_into::<#ok_ty>(__post_array.shift())
                                            .unwrap())
                                    } else {
                                        Err(web_rpc::wasm_bindgen::JsCast::dyn_into::<#err_ty>(__post_array.shift())
                                            .unwrap())
                                    }
                                }
                            },
                        }
                    } else {
                        quote! {
                            |(__response, _post_array)| {
                                let #response_ident::#camel_case_ident(__inner) = __response else {
                                    panic!("received incorrect response variant")
                                };
                                __inner
                            }
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
                    // Non-streaming (original logic)
                    let return_type = match output {
                        ReturnType::Type(_, ref ty) => quote! {
                            web_rpc::client::RequestFuture<#ty>
                        },
                        _ => quote!(())
                    };
                    let maybe_register_callback = match output {
                        ReturnType::Type(_, _) => quote! {
                            let (__response_tx, __response_rx) =
                                web_rpc::futures_channel::oneshot::channel();
                            self.callback_map.borrow_mut().insert(__seq_id, __response_tx);
                        },
                        _ => Default::default()
                    };

                    let unpack_response = if post.contains(&Ident::new("return", output.span())) {
                        let unit_output: &Type = &parse_quote!(());
                        let ret_ty = match output {
                            ReturnType::Type(_, ref ty) => ty,
                            _ => unit_output
                        };
                        match post_wrapper_kind(ret_ty) {
                            PostWrapperKind::Bare => quote! {
                                let (_, __post_response) = response;
                                web_rpc::wasm_bindgen::JsCast::dyn_into::<#ret_ty>(__post_response.shift())
                                    .unwrap()
                            },
                            PostWrapperKind::Option { inner_ty } => quote! {
                                let (__serialize_response, __post_response) = response;
                                let #response_ident::#camel_case_ident(__has_value) = __serialize_response else {
                                    panic!("received incorrect response variant")
                                };
                                if __has_value {
                                    Some(web_rpc::wasm_bindgen::JsCast::dyn_into::<#inner_ty>(__post_response.shift())
                                        .unwrap())
                                } else {
                                    None
                                }
                            },
                            PostWrapperKind::Result { ok_ty, err_ty } => quote! {
                                let (__serialize_response, __post_response) = response;
                                let #response_ident::#camel_case_ident(__is_ok) = __serialize_response else {
                                    panic!("received incorrect response variant")
                                };
                                if __is_ok {
                                    Ok(web_rpc::wasm_bindgen::JsCast::dyn_into::<#ok_ty>(__post_response.shift())
                                        .unwrap())
                                } else {
                                    Err(web_rpc::wasm_bindgen::JsCast::dyn_into::<#err_ty>(__post_response.shift())
                                        .unwrap())
                                }
                            },
                        }
                    } else {
                        quote! {
                            let (__serialize_response, _) = response;
                            let #response_ident::#camel_case_ident(__inner) = __serialize_response else {
                                panic!("received incorrect response variant")
                            };
                            __inner
                        }
                    };

                    let maybe_unpack_and_return_future = match output {
                        ReturnType::Type(_, _) => quote! {
                            let __response_future = web_rpc::futures_util::FutureExt::map(
                                __response_rx,
                                |response| {
                                    let response = response.unwrap();
                                    #unpack_response
                                }
                            );
                            let __abort_sender = self.abort_sender.clone();
                            let __dispatcher = self.dispatcher.clone();
                            web_rpc::client::RequestFuture::new(
                                __response_future,
                                __dispatcher,
                                std::boxed::Box::new(move || (__abort_sender)(__seq_id)))
                        },
                        _ => Default::default()
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
            quote! {
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
            has_borrowed_args,
            ..
        } = self;

        let request_type = if has_borrowed_args {
            quote! { #request_ident<'_> }
        } else {
            quote! { #request_ident }
        };

        let handlers = rpcs.iter()
            .zip(camel_case_idents.iter())
            .map(|(RpcMethod { is_async, ident, args, transfer, post, output, .. }, camel_case_ident)| {
                // Collect idents for request enum destructuring — non-post args
                // plus wrapped post args (which are now bool/Result<(), E> discriminants)
                let destructure_arg_idents: Vec<_> = args.iter()
                    .filter_map(|arg| match &*arg.pat {
                        Pat::Ident(pat_ident) => {
                            let id = &pat_ident.ident;
                            if post.contains(id) {
                                // Only include if wrapped (bare post args are not in the enum)
                                match post_wrapper_kind(&arg.ty) {
                                    PostWrapperKind::Bare => None,
                                    _ => Some(id),
                                }
                            } else {
                                Some(id)
                            }
                        }
                        _ => None
                    })
                    .collect();
                let extract_js_args = args.iter()
                    .filter_map(|arg| match &*arg.pat {
                        Pat::Ident(pat_ident) if post.contains(&pat_ident.ident) => {
                            let arg_pat = &arg.pat;
                            let arg_ty = &arg.ty;
                            match post_wrapper_kind(arg_ty) {
                                PostWrapperKind::Bare => {
                                    // Original logic for bare post args
                                    if let Type::Reference(type_ref) = &**arg_ty {
                                        let inner_ty = &type_ref.elem;
                                        let tmp_ident = format_ident!("__tmp_{}", pat_ident.ident);
                                        Some(quote! {
                                            let #tmp_ident = __js_args.shift();
                                            let #arg_pat: #arg_ty = web_rpc::wasm_bindgen::JsCast::dyn_ref::<#inner_ty>(&#tmp_ident)
                                                .unwrap();
                                        })
                                    } else {
                                        Some(quote! {
                                            let #arg_pat = web_rpc::wasm_bindgen::JsCast::dyn_into::<#arg_ty>(__js_args.shift())
                                                .unwrap();
                                        })
                                    }
                                }
                                PostWrapperKind::Option { inner_ty } => {
                                    // `arg_pat` is already bound as bool from destructuring
                                    // Shadow it with the reconstructed Option
                                    Some(quote! {
                                        let #arg_pat: #arg_ty = if #arg_pat {
                                            Some(web_rpc::wasm_bindgen::JsCast::dyn_into::<#inner_ty>(__js_args.shift())
                                                .unwrap())
                                        } else {
                                            None
                                        };
                                    })
                                }
                                PostWrapperKind::Result { ok_ty, .. } => {
                                    // `arg_pat` is already bound as Result<(), E> from destructuring
                                    // Shadow it with the reconstructed Result
                                    Some(quote! {
                                        let #arg_pat: #arg_ty = match #arg_pat {
                                            Ok(()) => Ok(web_rpc::wasm_bindgen::JsCast::dyn_into::<#ok_ty>(__js_args.shift())
                                                .unwrap()),
                                            Err(__e) => Err(__e),
                                        };
                                    })
                                }
                            }
                        },
                        _ => None
                    });

                // Check if this is a streaming method
                let is_streaming = matches!(output, ReturnType::Type(_, ref ty) if stream_item_type(ty).is_some());

                if is_streaming {
                    let call_args = args.iter().filter_map(|arg| match &*arg.pat {
                        Pat::Ident(ident) => Some(&ident.ident),
                        _ => None
                    });
                    let return_ident = Ident::new("return", output.span());
                    let is_post_return = post.contains(&return_ident);
                    let is_transfer_return = transfer.contains(&return_ident);
                    let item_ty = match output {
                        ReturnType::Type(_, ref ty) => stream_item_type(ty).unwrap(),
                        _ => unreachable!(),
                    };
                    let wrap_item = if !is_post_return {
                        quote! {
                            let __response = #response_ident::#camel_case_ident(__item);
                            let __post = web_rpc::js_sys::Array::new();
                            let __transfer = web_rpc::js_sys::Array::new();
                        }
                    } else {
                        let wrapper_kind = post_wrapper_kind(item_ty);
                        match wrapper_kind {
                            PostWrapperKind::Bare => {
                                let transfer_code = if is_transfer_return {
                                    quote! { web_rpc::js_sys::Array::of1(__item.as_ref()) }
                                } else {
                                    quote! { web_rpc::js_sys::Array::new() }
                                };
                                quote! {
                                    let __response = #response_ident::#camel_case_ident(());
                                    let __post = web_rpc::js_sys::Array::of1(__item.as_ref());
                                    let __transfer = #transfer_code;
                                }
                            }
                            PostWrapperKind::Option { .. } => {
                                let transfer_push = if is_transfer_return {
                                    quote! { __transfer.push(__val.as_ref()); }
                                } else {
                                    quote! {}
                                };
                                quote! {
                                    let (__response, __post, __transfer) = match __item {
                                        Some(ref __val) => {
                                            let __post = web_rpc::js_sys::Array::of1(__val.as_ref());
                                            let __transfer = web_rpc::js_sys::Array::new();
                                            #transfer_push
                                            (#response_ident::#camel_case_ident(true), __post, __transfer)
                                        }
                                        None => {
                                            (#response_ident::#camel_case_ident(false),
                                             web_rpc::js_sys::Array::new(),
                                             web_rpc::js_sys::Array::new())
                                        }
                                    };
                                }
                            }
                            PostWrapperKind::Result { .. } => {
                                let transfer_push = if is_transfer_return {
                                    quote! { __transfer.push(__val.as_ref()); }
                                } else {
                                    quote! {}
                                };
                                quote! {
                                    let (__response, __post, __transfer) = match __item {
                                        Ok(ref __val) => {
                                            let __post = web_rpc::js_sys::Array::of1(__val.as_ref());
                                            let __transfer = web_rpc::js_sys::Array::new();
                                            #transfer_push
                                            (#response_ident::#camel_case_ident(true), __post, __transfer)
                                        }
                                        Err(ref __val) => {
                                            let __post = web_rpc::js_sys::Array::of1(__val.as_ref());
                                            let __transfer = web_rpc::js_sys::Array::new();
                                            (#response_ident::#camel_case_ident(false), __post, __transfer)
                                        }
                                    };
                                }
                            }
                        }
                    };
                    // Build the forwarding closure (reused for both async/sync)
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
                            #request_ident::#camel_case_ident { #( #destructure_arg_idents ),* } => {
                                #( #extract_js_args )*
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
                            #request_ident::#camel_case_ident { #( #destructure_arg_idents ),* } => {
                                #( #extract_js_args )*
                                let mut __user_rx = self.server_impl.#ident(#( #call_args ),*);
                                #fwd_body
                            }
                        },
                    }
                } else {
                    // Non-streaming (original logic, but wrapped in ExecuteResult::Response)
                    let return_ident = Ident::new("return", output.span());
                    let is_post_return = post.contains(&return_ident);
                    let is_transfer_return = transfer.contains(&return_ident);
                    let ret_ty = match output {
                        ReturnType::Type(_, ref ty) => Some(ty.as_ref()),
                        _ => None,
                    };
                    let return_response = if !is_post_return {
                        quote! {
                            let __post = web_rpc::js_sys::Array::new();
                            let __transfer = web_rpc::js_sys::Array::new();
                            (#response_ident::#camel_case_ident(__response), __post, __transfer)
                        }
                    } else {
                        let wrapper_kind = ret_ty.map(|ty| post_wrapper_kind(ty));
                        match wrapper_kind {
                            Some(PostWrapperKind::Option { .. }) => {
                                let transfer_push = if is_transfer_return {
                                    quote! { __transfer.push(__val.as_ref()); }
                                } else {
                                    quote! {}
                                };
                                quote! {
                                    match __response {
                                        Some(ref __val) => {
                                            let __post = web_rpc::js_sys::Array::of1(__val.as_ref());
                                            let __transfer = web_rpc::js_sys::Array::new();
                                            #transfer_push
                                            (#response_ident::#camel_case_ident(true), __post, __transfer)
                                        }
                                        None => {
                                            (#response_ident::#camel_case_ident(false),
                                             web_rpc::js_sys::Array::new(),
                                             web_rpc::js_sys::Array::new())
                                        }
                                    }
                                }
                            }
                            Some(PostWrapperKind::Result { .. }) => {
                                let transfer_push = if is_transfer_return {
                                    quote! { __transfer.push(__val.as_ref()); }
                                } else {
                                    quote! {}
                                };
                                quote! {
                                    match __response {
                                        Ok(ref __val) => {
                                            let __post = web_rpc::js_sys::Array::of1(__val.as_ref());
                                            let __transfer = web_rpc::js_sys::Array::new();
                                            #transfer_push
                                            (#response_ident::#camel_case_ident(true), __post, __transfer)
                                        }
                                        Err(ref __val) => {
                                            let __post = web_rpc::js_sys::Array::of1(__val.as_ref());
                                            let __transfer = web_rpc::js_sys::Array::new();
                                            (#response_ident::#camel_case_ident(false), __post, __transfer)
                                        }
                                    }
                                }
                            }
                            _ => {
                                // Bare post return
                                let transfer_code = if is_transfer_return {
                                    quote! { web_rpc::js_sys::Array::of1(__response.as_ref()) }
                                } else {
                                    quote! { web_rpc::js_sys::Array::new() }
                                };
                                quote! {
                                    let __post = web_rpc::js_sys::Array::of1(__response.as_ref());
                                    let __transfer = #transfer_code;
                                    (#response_ident::#camel_case_ident(()), __post, __transfer)
                                }
                            }
                        }
                    };
                    let call_args = args.iter().filter_map(|arg| match &*arg.pat {
                        Pat::Ident(ident) => Some(&ident.ident),
                        _ => None
                    });
                    match is_async {
                        Some(_) => quote! {
                            #request_ident::#camel_case_ident { #( #destructure_arg_idents ),* } => {
                                #( #extract_js_args )*
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
                            #request_ident::#camel_case_ident { #( #destructure_arg_idents ),* } => {
                                #( #extract_js_args )*
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

impl Parse for RpcMethod {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut errors = Ok(());
        let attrs = input.call(Attribute::parse_outer)?;
        let (post_attrs, attrs): (Vec<_>, Vec<_>) = attrs.into_iter().partition(|attr| {
            attr.path
                .segments
                .last()
                .is_some_and(|last_segment| last_segment.ident == "post")
        });
        let mut transfer: HashSet<Ident> = HashSet::new();
        let mut post: HashSet<Ident> = HashSet::new();
        for post_attr in post_attrs {
            let parsed_args =
                post_attr.parse_args_with(Punctuated::<NestedMeta, Token![,]>::parse_terminated)?;
            for parsed_arg in parsed_args {
                match &parsed_arg {
                    NestedMeta::Meta(meta) => match meta {
                        Meta::Path(path) => {
                            if let Some(segment) = path.segments.last() {
                                post.insert(segment.ident.clone());
                            }
                        }
                        Meta::List(list) => match list.path.segments.last() {
                            Some(last_segment) if last_segment.ident == "transfer" => {
                                if list.nested.len() != 1 {
                                    extend_errors!(
                                        errors,
                                        syn::Error::new(
                                            parsed_arg.span(),
                                            "Syntax error in post attribute"
                                        )
                                    );
                                }
                                match list.nested.first() {
                                    Some(NestedMeta::Meta(Meta::Path(path))) => {
                                        match path.segments.last() {
                                            Some(segment) => {
                                                post.insert(segment.ident.clone());
                                                transfer.insert(segment.ident.clone());
                                            }
                                            _ => extend_errors!(
                                                errors,
                                                syn::Error::new(
                                                    parsed_arg.span(),
                                                    "Syntax error in post attribute"
                                                )
                                            ),
                                        }
                                    }
                                    _ => extend_errors!(
                                        errors,
                                        syn::Error::new(
                                            parsed_arg.span(),
                                            "Syntax error in post attribute"
                                        )
                                    ),
                                }
                            }
                            _ => extend_errors!(
                                errors,
                                syn::Error::new(
                                    parsed_arg.span(),
                                    "Syntax error in post attribute"
                                )
                            ),
                        },
                        _ => extend_errors!(
                            errors,
                            syn::Error::new(parsed_arg.span(), "Syntax error in post attribute")
                        ),
                    },
                    _ => extend_errors!(
                        errors,
                        syn::Error::new(parsed_arg.span(), "Syntax error in post attribute")
                    ),
                }
            }
        }

        let is_async = input.parse::<Token![async]>().ok();
        input.parse::<Token![fn]>()?;
        let ident: Ident = input.parse()?;
        let content;
        parenthesized!(content in input);
        let mut receiver: Option<syn::Receiver> = None;
        let mut args = Vec::new();
        for arg in content.parse_terminated::<FnArg, Comma>(FnArg::parse)? {
            match arg {
                FnArg::Typed(captured) => match &*captured.pat {
                    Pat::Ident(_) => args.push(captured),
                    _ => {
                        extend_errors!(
                            errors,
                            syn::Error::new(
                                captured.pat.span(),
                                "patterns are not allowed in RPC arguments"
                            )
                        )
                    }
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

        let arg_names: HashSet<_> = args
            .iter()
            .filter_map(|arg| match &*arg.pat {
                Pat::Ident(pat_ident) => Some(pat_ident.ident.clone()),
                _ => None,
            })
            .collect();
        let return_ident = Ident::new("return", output.span());
        for ident in &post {
            if *ident != return_ident && !arg_names.contains(ident) {
                extend_errors!(
                    errors,
                    syn::Error::new(
                        ident.span(),
                        format!("`{}` does not match any parameter", ident)
                    )
                );
            }
        }
        for ident in &transfer {
            if *ident != return_ident && !post.contains(ident) {
                extend_errors!(
                    errors,
                    syn::Error::new(
                        ident.span(),
                        format!("`{}` is marked as transfer but not as post", ident)
                    )
                );
            }
        }
        errors?;

        Ok(Self {
            is_async,
            attrs,
            receiver,
            ident,
            args,
            post,
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

    let has_borrowed_args = rpcs.iter().any(|rpc| {
        rpc.args.iter().any(|arg| {
            matches!(&*arg.pat, Pat::Ident(pat_ident) if !rpc.post.contains(&pat_ident.ident))
                && matches!(&*arg.ty, Type::Reference(_))
        })
    });

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
        has_borrowed_args,
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
