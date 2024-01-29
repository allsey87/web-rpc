use std::collections::HashSet;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, ToTokens};
use syn::{
    braced,
    ext::IdentExt,
    parenthesized,
    parse::{Parse, ParseStream},
    parse_macro_input, parse_quote,
    spanned::Spanned,
    token::Comma,
    Attribute, FnArg, Ident, Pat, PatType, ReturnType, Token, Type,
    Visibility, punctuated::Punctuated, NestedMeta, Meta,
};

macro_rules! extend_errors {
    ($errors: ident, $e: expr) => {
        match $errors {
            Ok(_) => $errors = Err($e),
            Err(ref mut errors) => errors.extend($e),
        }
    };
}

struct Service {
    attrs: Vec<Attribute>,
    vis: Visibility,
    ident: Ident,
    rpcs: Vec<RpcMethod>,
}

struct RpcMethod {
    is_async: bool,
    attrs: Vec<Attribute>,
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
        let variants = rpcs.iter().zip(camel_case_idents.iter())
            .map(|(RpcMethod { args, post, .. }, camel_case_ident)| {
                let args_filtered = args.iter()
                    .filter(|arg| matches!(&*arg.pat, Pat::Ident(ident) if !post.contains(&ident.ident)));
                quote! {
                    #camel_case_ident { #( #args_filtered ),* }
                }
            });
        quote! {
            #[derive(web_rpc::serde::Serialize, web_rpc::serde::Deserialize)]
            #vis enum #request_ident {
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
        let variants = rpcs.iter().zip(camel_case_idents.iter())
            .map(|(RpcMethod { output, post, .. }, camel_case_ident)| match output {
                ReturnType::Type(_, ty) if !post.contains(&Ident::new("return", output.span())) => quote! {
                    #camel_case_ident ( #ty )
                },
                _ => quote! {
                    #camel_case_ident ( () )
                },
            });
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
        let rpc_fns = rpcs.iter()
            .map(|RpcMethod { attrs, args, ident, is_async, output, .. }| {
                let output = match output {
                    ReturnType::Type(_, ref ty) => ty,
                    ReturnType::Default => unit_type
                };
                let is_async = match is_async {
                    true => quote!(async),
                    false => quote!()
                };
                quote! {
                    #( #attrs )*
                    #is_async fn #ident(&self, #( #args ),*) -> #output;
                }
            });

        let forward_fns = rpcs.iter()
            .map(|RpcMethod { attrs, args, ident, is_async, output, .. }| {
                let output = match output {
                    ReturnType::Type(_, ref ty) => ty,
                    ReturnType::Default => unit_type
                };
                let do_await = match is_async {
                    true => quote!(.await),
                    false => quote!()
                };
                let is_async = match is_async {
                    true => quote!(async),
                    false => quote!()
                };
                let forward_args = args.iter().filter_map(|arg| match &*arg.pat {
                    Pat::Ident(ident) => Some(&ident.ident),
                    _ => None
                });
                quote! {
                    #( #attrs )*
                    #is_async fn #ident(&self, #( #args ),*) -> #output {
                        T::#ident(self, #( #forward_args ),*)#do_await
                    }
                }
            })
            .collect::<Vec<_>>();

        quote! {
            #( #attrs )*
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
            ..
        } = self;

        let rpc_fns = rpcs
            .iter()
            .zip(camel_case_idents.iter())
            .map(|(RpcMethod { attrs, args, transfer, post, ident, output, .. }, camel_case_ident)| {
                /* sort arguments based on post and transfer attributes */
                let serialize_arg_idents = args.iter()
                    .filter_map(|arg| match &*arg.pat {
                        Pat::Ident(ident) if !post.contains(&ident.ident) => Some(&ident.ident),
                        _ => None
                    });
                let post_arg_idents = args.iter()
                    .filter_map(|arg| match &*arg.pat {
                        Pat::Ident(ident) if post.contains(&ident.ident) => Some(&ident.ident),
                        _ => None
                    });
                let transfer_arg_idents = args.iter()
                    .filter_map(|arg| match &*arg.pat {
                        Pat::Ident(ident) if transfer.contains(&ident.ident) => Some(&ident.ident),
                        _ => None
                    });

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
                    let output = match output {
                        ReturnType::Type(_, ref ty) => ty,
                        _ => unit_output
                    };
                    quote! {
                        let (_, __post_response) = response;
                        web_rpc::wasm_bindgen::JsCast::dyn_into::<#output>(__post_response.shift())
                            .unwrap()
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
                        web_rpc::client::RequestFuture::new(
                            __response_future,
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
                        let __seq_id = self.seq_id.replace_with(|seq_id| seq_id.wrapping_add(1));
                        let __request = #request_ident::#camel_case_ident {
                            #( #serialize_arg_idents ),*
                        };
                        let __serialized = (self.request_serializer)(__seq_id, __request);
                        let __serialized = js_sys::Uint8Array::from(&__serialized[..]).buffer();
                        let __post: &[&wasm_bindgen::JsValue] =
                            &[__serialized.as_ref(), #( #post_arg_idents.as_ref() ),*];
                        let __post = web_rpc::js_sys::Array::from_iter(__post);
                        let __transfer: &[&wasm_bindgen::JsValue] =
                            &[__serialized.as_ref(), #( #transfer_arg_idents.as_ref() ),*];
                        let __transfer = web_rpc::js_sys::Array::from_iter(__transfer);
                        #maybe_register_callback
                        self.interface.post_message(&__post, &__transfer).unwrap();
                        #maybe_unpack_and_return_future
                    }
                }
            });

        quote! {
            #[derive(core::clone::Clone)]
            #vis struct #client_ident {
                callback_map: std::rc::Rc<
                    std::cell::RefCell<
                        web_rpc::client::CallbackMap<#response_ident>
                    >
                >,
                interface: std::rc::Rc<
                    dyn web_rpc::interface::Interface
                >,
                listener: std::rc::Rc<web_rpc::gloo_events::EventListener>,
                request_serializer: std::rc::Rc<
                    dyn std::ops::Fn(usize, #request_ident) -> std::vec::Vec<u8>
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
                type Request = #request_ident;
                type Response = #response_ident;
            }
            impl<I> From<web_rpc::client::Configuration<#request_ident, #response_ident, I>>
                for #client_ident where I: web_rpc::interface::Interface + 'static {
                fn from((callback_map, interface, listener, request_serializer, abort_sender):
                    web_rpc::client::Configuration<#request_ident, #response_ident, I>) -> Self {
                    Self {
                        callback_map,
                        interface,
                        listener,
                        request_serializer,
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

        let handlers = rpcs.iter()
            .zip(camel_case_idents.iter())
            .map(|(RpcMethod { is_async, ident, args, transfer, post, output, .. }, camel_case_ident)| {
                let serialize_arg_idents = args.iter()
                    .filter_map(|arg| match &*arg.pat {
                        Pat::Ident(ident) if !post.contains(&ident.ident) => Some(&ident.ident),
                        _ => None
                    });
                let extract_js_args = args.iter()
                    .filter_map(|arg| match &*arg.pat {
                        Pat::Ident(ident) if post.contains(&ident.ident) => {
                            let arg_pat = &arg.pat;
                            let arg_ty = &arg.ty;
                            Some(quote! {
                                let #arg_pat = web_rpc::wasm_bindgen::JsCast::dyn_into::<#arg_ty>(__js_args.shift())
                                    .unwrap();
                            })
                        },
                        _ => None
                    });
                let return_ident = Ident::new("return", output.span());
                let return_response = match (post.contains(&return_ident), transfer.contains(&return_ident)) {
                    (false, _) => quote! {
                        let __post = web_rpc::js_sys::Array::new();
                        let __transfer = web_rpc::js_sys::Array::new();
                        (Self::Response::#camel_case_ident(__response), __post, __transfer)
                    },
                    (true, false) => quote! {
                        let __post = web_rpc::js_sys::Array::of1(__response.as_ref());
                        let __transfer = web_rpc::js_sys::Array::new();
                        (Self::Response::#camel_case_ident(()), __post, __transfer)
                    },
                    (true, true) => quote! {
                        let __post = web_rpc::js_sys::Array::of1(__response.as_ref());
                        let __transfer = web_rpc::js_sys::Array::of1(__response.as_ref());
                        (Self::Response::#camel_case_ident(()), __post, __transfer)
                    }
                };
                let args = args.iter().filter_map(|arg| match &*arg.pat {
                    Pat::Ident(ident) => Some(&ident.ident),
                    _ => None
                });
                match is_async {
                    true => quote! {
                        Self::Request::#camel_case_ident { #( #serialize_arg_idents ),* } => {
                            #( #extract_js_args )*
                            let __task =
                                web_rpc::futures_util::FutureExt::fuse(self.server_impl.#ident(#( #args ),*));
                            web_rpc::pin_utils::pin_mut!(__task);
                            web_rpc::futures_util::select! {
                                _ = __abort_rx => None,
                                __response = __task => Some({
                                    #return_response
                                })
                            }
                        }
                    },
                    false => quote! {
                        Self::Request::#camel_case_ident { #( #serialize_arg_idents ),* } => {
                            #( #extract_js_args )*
                            let __response = self.server_impl.#ident(#( #args ),*);
                            Some({
                                #return_response
                            })
                        }
                    }
                }
            });

        quote! {
            #vis struct #service_ident<T> {
                server_impl: T
            }
            impl<T: #trait_ident> web_rpc::service::Service for #service_ident<T> {
                type Request = #request_ident;
                type Response = #response_ident;
                async fn execute(
                    &self,
                    __seq_id: usize,
                    mut __abort_rx: web_rpc::futures_channel::oneshot::Receiver<()>,
                    __request: Self::Request,
                    __js_args: web_rpc::js_sys::Array
                ) -> (usize, Option<(Self::Response, web_rpc::js_sys::Array, web_rpc::js_sys::Array)>) {
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
        let (post_attrs, attrs): (Vec<_>, Vec<_>) = attrs.into_iter()
            .partition(|attr| attr.path.segments.last()
                .is_some_and(|last_segment| last_segment.ident == "post"));
        let mut transfer: HashSet<Ident> = HashSet::new();
        let mut post: HashSet<Ident> = HashSet::new();
        for post_attr in post_attrs {
            let parsed_args =
                post_attr.parse_args_with(Punctuated::<NestedMeta, Token![,]>::parse_terminated)?;
            for parsed_arg in parsed_args {
                match &parsed_arg {
                    NestedMeta::Meta(meta) => match meta {
                        Meta::Path(path) => if let Some(segment) = path.segments.last() {
                            post.insert(segment.ident.clone());
                        },
                        Meta::List(list) => match list.path.segments.last() {
                            Some(last_segment) if last_segment.ident == "transfer" => {
                                if list.nested.len() != 1 {
                                    extend_errors!(
                                        errors,
                                        syn::Error::new(parsed_arg.span(), "Syntax error in post attribute")
                                    );
                                }
                                match list.nested.first() {
                                    Some(NestedMeta::Meta(Meta::Path(path))) => match path.segments.last() {
                                        Some(segment) => {
                                            post.insert(segment.ident.clone());
                                            transfer.insert(segment.ident.clone());
                                        },
                                        _ => extend_errors!(
                                            errors,
                                            syn::Error::new(parsed_arg.span(), "Syntax error in post attribute")
                                        )
                                    }
                                    _ => extend_errors!(
                                        errors,
                                        syn::Error::new(parsed_arg.span(), "Syntax error in post attribute")
                                    )
                                }
                            }
                            _ => extend_errors!(
                                errors,
                                syn::Error::new(parsed_arg.span(), "Syntax error in post attribute")
                            )
                        }
                        _ => extend_errors!(
                            errors,
                            syn::Error::new(parsed_arg.span(), "Syntax error in post attribute")
                        )
                    },
                    _ => extend_errors!(
                        errors,
                        syn::Error::new(parsed_arg.span(), "Syntax error in post attribute")
                    )
                }
            }
        }
        
        let is_async = input.parse::<Token![async]>().is_ok();
        input.parse::<Token![fn]>()?;
        let ident = input.parse()?;
        let content;
        parenthesized!(content in input);
        let mut args = Vec::new();
        for arg in content.parse_terminated::<FnArg, Comma>(FnArg::parse)? {
            match arg {
                FnArg::Typed(captured) => {
                    match &*captured.pat {
                        Pat::Ident(_) => args.push(captured),
                        _ => {
                            extend_errors!(
                                errors,
                                syn::Error::new(captured.pat.span(), "patterns are not allowed in RPC arguments")
                            )
                        }
                    }
                }
                FnArg::Receiver(_) => {
                    extend_errors!(
                        errors,
                        syn::Error::new(arg.span(), "receivers are not allowed in RPC arguments")
                    );
                }
            }
        }
        errors?;
        let output = input.parse()?;
        input.parse::<Token![;]>()?;

        Ok(Self {
            is_async,
            attrs,
            ident,
            args,
            post,
            transfer,
            output,
        })
    }
}

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
