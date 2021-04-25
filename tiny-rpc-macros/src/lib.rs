use std::borrow::Cow;

use proc_macro::TokenStream;
use proc_macro_error::*;
use quote::{format_ident, quote, ToTokens};
use syn::{spanned::Spanned, *};

#[proc_macro_error]
#[proc_macro_attribute]
pub fn rpc_trait(_args: TokenStream, trait_body: TokenStream) -> TokenStream {
    rpc_define(trait_body)
}

// TODO generics
#[proc_macro_error]
#[proc_macro]
pub fn rpc_define(trait_body: TokenStream) -> TokenStream {
    let root: Path = parse_quote! { ::tiny_rpc::rpc::re_export }; // TODO extract from meta

    let trait_body = parse_macro_input!(trait_body as ItemTrait);
    let ident = &trait_body.ident;
    let func_list = gen_func_list(&trait_body);
    let (req_rsp_body, req_ident, rsp_ident) =
        gen_req_rsp(&root, &trait_body.vis, ident, &func_list);
    let server_body = gen_server(&root, ident, &req_ident, &rsp_ident, &func_list);
    let client_body = gen_client(&root, ident, &req_ident, &rsp_ident, &func_list);

    let ret = quote! {
        #trait_body
        #req_rsp_body
        #server_body
        #client_body
    };
    if option_env!("RUST_TRACE_MACROS").is_some() {
        println!("{}", ret);
    }
    ret.into()
}

/// Check if `arg` is receiver we want, i.e., `&self`, `&'a self`, `self: &Self` and `self: &'a Self`
fn is_ref_receiver(arg: Option<&FnArg>) -> bool {
    let arg = match arg {
        Some(arg) => arg,
        None => return false,
    };

    match arg {
        FnArg::Receiver(receiver) => receiver.reference.is_some() && receiver.mutability.is_none(), // `&self`, not `self` or `&mut self`
        FnArg::Typed(PatType { pat, ty, .. }) => {
            matches!(pat.as_ref(), Pat::Ident(ident) if ident.ident == "self") // `self: T`
                && matches!(
                    ty.as_ref(),
                    Type::Reference(TypeReference{ mutability, elem, ..})
                        if mutability.is_none() && matches!(elem.as_ref(), Type::Path(path) if path.qself.is_none() && path.path.is_ident("Self")) // `self: T` where T is a reference to `Self`
                )
        }
    }
}

/// Generate a list of trait method.
///
/// This function emit error and generate dummy for following cases:
///  - A trait method which has a default implementation.
///  - The first input is not `&self` or its equivalent.
///  - An input is given in pattern, e.g., `(x, y): (f32, f32)`.
fn gen_func_list(trait_body: &ItemTrait) -> Vec<Cow<'_, TraitItemMethod>> {
    let ref_receiver: FnArg = parse_quote!(&self); // const

    trait_body
        .items
        .iter()
        .filter_map(|item| match item {
            TraitItem::Method(method) => {
                // check signatures and prepare dummy for bad method

                let mut method = Cow::Borrowed(method);
                if method.default.is_some() {
                    emit_error!(
                        method.default,
                        "trait method can't have default implementation"
                    );

                    // create dummy by remove default and append semicolon(`;`)
                    let mut dummy = method.into_owned();
                    dummy.semi_token = Some(Token![;](dummy.default.span()));
                    dummy.default = None;
                    method = Cow::Owned(dummy);
                }

                if !is_ref_receiver(method.sig.inputs.first()) {
                    emit_error!(method, "trait method must have `&self` receiver");

                    // create dummy by replace bad receiver, append one if none is provided
                    let mut dummy = method.into_owned();
                    match dummy.sig.inputs.first() {
                        Some(FnArg::Receiver(_)) => {
                            // e.g., `&mut self`
                            *(dummy
                                .sig
                                .inputs
                                .first_mut()
                                .expect("infallible: non-mutable use before")) =
                                ref_receiver.clone();
                        }
                        Some(FnArg::Typed(PatType { pat, .. })) => match &**pat {
                            Pat::Ident(PatIdent { ident, .. }) if ident == "self" => {
                                // e.g., `self: Box<Self>`
                                *(dummy
                                    .sig
                                    .inputs
                                    .first_mut()
                                    .expect("infallible: non-mutable use before")) =
                                    ref_receiver.clone();
                            }
                            _ => {
                                // no receiver, just argument
                                dummy.sig.inputs.insert(0, ref_receiver.clone());
                            }
                        },
                        None => {
                            // no receiver and argument
                            dummy.sig.inputs.insert(0, ref_receiver.clone());
                        }
                    }
                    method = Cow::Owned(dummy);
                }

                for i in 0..(method.sig.inputs.len()) {
                    if let FnArg::Typed(PatType { ref pat, .. }) = method.sig.inputs[i] {
                        match pat.as_ref() {
                            Pat::Ident(_) => {}
                            other => {
                                emit_error!(other, "trait method cannot use pattern as argument");

                                let dummy_ident = format_ident!("__dummy_{:x}", {
                                    use std::hash::{Hash, Hasher};

                                    let mut h =
                                        std::collections::hash_map::DefaultHasher::default();
                                    other.hash(&mut h);
                                    h.finish()
                                });

                                let new_pat = Box::new(Pat::Ident(PatIdent {
                                    ident: dummy_ident,
                                    attrs: Default::default(),
                                    by_ref: None,
                                    mutability: None,
                                    subpat: None,
                                }));

                                let mut dummy = method.into_owned();
                                match dummy.sig.inputs[i] {
                                    FnArg::Typed(PatType { ref mut pat, .. }) => *pat = new_pat,
                                    _ => unreachable!(),
                                }
                                method = Cow::Owned(dummy);
                            }
                        }
                    }
                }

                Some(method)
            }
            item => {
                emit_error!(
                    item,
                    "#[rpc_define] trait cannot have any item other than function"
                );
                None
            }
        })
        .collect::<Vec<_>>()
}

fn gen_req_rsp<'a>(
    root: &Path,
    vis: &Visibility,
    ident: &Ident,
    func_list: &[Cow<'a, TraitItemMethod>],
) -> (proc_macro2::TokenStream, Ident, Ident) {
    let unit_type = parse_quote!(()); // const

    let req_ident = format_ident!("{}Request", ident);
    let rsp_ident = format_ident!("{}Response", ident);
    let serde_path = format!("{}::serde", root.to_token_stream());
    let serde_path = LitStr::new(serde_path.as_str(), root.span());

    let func_ident = func_list
        .iter()
        .map(|method| &method.sig.ident)
        .collect::<Vec<_>>();
    let input_type = func_list.iter().map(|method| {
        method
            .sig
            .inputs
            .iter()
            .skip(1) // Skip the receiver, which must exist.
            .map(|input| match input {
                FnArg::Typed(PatType { ty, .. }) => ty,
                FnArg::Receiver(_) => unreachable!(),
            })
            .collect::<Vec<_>>()
    });
    let output_type = func_list.iter().map(|method| match method.sig.output {
        ReturnType::Default => &unit_type,
        ReturnType::Type(_, ref ty) => ty.as_ref(),
    });

    let req_rsp = quote! {
        #[derive(#root::Serialize, #root::Deserialize)]
        #[serde(crate = #serde_path)]
        #[serde(deny_unknown_fields)]
        #[allow(non_camel_case_types)]
        #vis enum #req_ident<'req> {
            #( #func_ident ( ( #(#input_type,)* ) ), )*
            ___tiny_rpc_marker((#root::Never, #root::PhantomData<&'req ()>))
        }

        #[derive(#root::Serialize, #root::Deserialize)]
        #[serde(crate = #serde_path)]
        #[serde(deny_unknown_fields)]
        #[allow(non_camel_case_types)]
        #vis enum #rsp_ident {
            #( #func_ident ( #output_type ), )*
        }
    };

    (req_rsp, req_ident, rsp_ident)
}

fn gen_server<'a>(
    root: &Path,
    ident: &Ident,
    req_ident: &Ident,
    rsp_ident: &Ident,
    func_list: &[Cow<'a, TraitItemMethod>],
) -> proc_macro2::TokenStream {
    let null_stream = quote! {}; // const
    let keyword_await = quote! { .await }; // const

    let server_ident = format_ident!("{}Server", ident);
    let func_ident = func_list
        .iter()
        .map(|method| &method.sig.ident)
        .collect::<Vec<_>>();
    let input_ident = func_list
        .iter()
        .map(|method| {
            method
                .sig
                .inputs
                .iter()
                .filter_map(|input| match input {
                    FnArg::Receiver(_) => None, // NOTE all receivers are now `&self`
                    FnArg::Typed(PatType { pat, .. }) => match &**pat {
                        Pat::Ident(ident) => Some(&ident.ident),
                        _ => unreachable!(),
                    },
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let await_if_async = func_list.iter().map(|method| {
        method
            .sig
            .asyncness
            .map_or(&null_stream, |_| &keyword_await)
    });

    quote! {
        pub struct #server_ident<T: #ident + #root::Send + #root::Sync + 'static>(#root::Arc<T>);

        impl<T: #ident + #root::Send + #root::Sync + 'static> #server_ident<T> {
            pub fn serve(server_impl: T, transport: #root::Transport) -> #root::BoxStream<'static, #root::BoxFuture<'static, ()>> {
                Self::__internal_serve(Self(#root::Arc::new(server_impl)), transport)
            }

            pub fn serve_arc(server_impl: #root::Arc<T>, transport: #root::Transport) -> #root::BoxStream<'static, #root::BoxFuture<'static, ()>> {
                Self::__internal_serve(Self(server_impl), transport)
            }

            fn __internal_serve(self, transport: #root::Transport) -> #root::BoxStream<'static, #root::BoxFuture<'static, ()>> {
                #root::Server::serve(self, transport)
            }
        }

        impl<T: #ident + #root::Send + #root::Sync + 'static> #root::Clone for #server_ident<T> {
            fn clone(&self) -> Self {
                Self(#root::Clone::clone(&self.0))
            }
        }

        impl<T: #ident + #root::Send + #root::Sync + 'static> #root::Server for #server_ident<T> {
            fn make_response(
                self,
                req: #root::RpcFrame,
            ) -> #root::BoxFuture<'static, #root::Result<#root::RpcFrame>> {
                #root::FutureExt::boxed(
                    async move {
                        let id = req.id()?;
                        let req = req.data()?;
                        let rsp = match req {
                            #(
                                #req_ident::#func_ident( ( #(#input_ident,)* ) ) => {
                                    #rsp_ident::#func_ident(self.0.#func_ident(#(#input_ident),*) #await_if_async)
                                }
                            )*
                            #req_ident::___tiny_rpc_marker(_) => #root::unreachable!(),
                        };
                        let rsp = #root::RpcFrame::new(id, rsp)?;
                        Ok(rsp)
                    }
                )
            }
        }
    }
}

fn gen_client<'a>(
    root: &Path,
    ident: &Ident,
    req_ident: &Ident,
    rsp_ident: &Ident,
    func_list: &[Cow<'a, TraitItemMethod>],
) -> proc_macro2::TokenStream {
    let unit_type: Type = parse_quote!(()); // const

    let client_ident = format_ident!("{}Client", ident);
    let signature = func_list
        .iter()
        .cloned()
        .map(|method| {
            let method = method.into_owned();
            let span = method.span();
            let mut sig = method.sig;

            if sig.asyncness.is_none() {
                sig.asyncness = Some(Token![async](span));
            }

            let ty = match sig.output {
                ReturnType::Type(_, ty) => *ty,
                ReturnType::Default => unit_type.clone(),
            };
            sig.output = parse_quote! { -> #root::Result<#ty> };
            sig
        })
        .collect::<Vec<_>>();
    let arg_ident = signature.iter().map(|sig| {
        sig.inputs
            .iter()
            .filter_map(|arg| match arg {
                FnArg::Receiver(_) => None,
                FnArg::Typed(PatType { pat, .. }) => match &**pat {
                    Pat::Ident(ident) => Some(&ident.ident),
                    _ => unreachable!(),
                },
            })
            .collect::<Vec<_>>()
    });
    let func_ident = signature.iter().map(|sig| &sig.ident);

    quote! {
        #[derive(Clone)]
        pub struct #client_ident(#root::IdGenerator, #root::ClientDriverHandle);

        impl #root::Client for #client_ident {
            fn from_handle(handle: #root::ClientDriverHandle) -> Self {
                Self(#root::IdGenerator::new(), handle)
            }

            fn handle(&self) -> &#root::ClientDriverHandle {
                &self.1
            }
        }

        impl #client_ident {
            pub fn new(transport: #root::Transport) -> (Self, #root::BoxFuture<'static, ()>) {
                #root::Client::new(transport)
            }

            #(
                pub #signature {
                    let args = ( #(#arg_ident,)* );
                    let id = self.0.next();
                    let span = info_span!(#root::stringify!(#func_ident), %id);

                    #root::Instrument::instrument(
                        async move {
                            let req = #req_ident::#func_ident(args);
                            let req = #root::RpcFrame::new(id, req)?;
                            let rsp = <Self as #root::Client>::make_request(self, req).await?;
                            let rsp = rsp.data()?;
                            match rsp {
                                #rsp_ident::#func_ident(ret) => Ok(ret),
                                _ => Err(#root::Into::into(#root::ProtocolError::ResponseMismatch(id))),
                            }
                        },
                        span,
                    )
                    .await
                }
            )*
        }
    }
}

#[test]
fn test_is_ref_receiver() {
    let ref_receiver: &[FnArg] = &[
        parse_quote!(self),
        parse_quote!(&self),
        parse_quote!(&'a self),
        parse_quote!(&mut self),
        parse_quote!(&'a mut self),
        parse_quote!(self: Self),
        parse_quote!(self: &Self),
        parse_quote!(self: &'a Self),
        parse_quote!(self: &mut Self),
        parse_quote!(self: &'a mut Self),
    ];
    let answer = &[
        false, true, true, false, false, false, true, true, false, false,
    ];

    assert_eq!(is_ref_receiver(None), false);
    for (t, a) in ref_receiver.into_iter().zip(answer) {
        assert_eq!(is_ref_receiver(Some(t)), *a);
    }
}
