mod helper;

use proc_macro::TokenStream;
use proc_macro_error::*;
use punctuated::Punctuated;
use quote::{format_ident, quote, ToTokens};
use spanned::Spanned;
use syn::*;

// TODO generics
#[proc_macro_error]
#[proc_macro]
pub fn rpc_define(trait_body: TokenStream) -> TokenStream {
    let unit = Type::Tuple(TypeTuple {
        paren_token: token::Paren::default(),
        elems: Punctuated::default(),
    });

    let mut trait_body = parse_macro_input!(trait_body as ItemTrait);
    let functions = trait_body
        .items
        .iter()
        .filter_map(|item| match item {
            syn::TraitItem::Method(x) => Some(x),
            item => {
                emit_error!(
                    item,
                    "#[rpc_define] trait cannot have any item other than function"
                );
                None
            }
        })
        .collect::<Vec<_>>();
    let vis = &trait_body.vis;
    let mut root: Path = parse_quote!(::tiny_rpc::rpc::re_export);

    trait_body.attrs = trait_body
        .attrs
        .into_iter()
        .filter(|attr| {
            if attr.path.is_ident("rpc") {
                let span = attr.span();
                if let Some((flags, mut props)) = helper::parse_meta(attr) {
                    if let Some((ident, lit)) = props.remove("root") {
                        if let Lit::Str(s) = lit {
                            root = s.parse().unwrap_or_abort();
                        } else {
                            abort!(ident.span(), "`root` require a string")
                        }
                    }

                    flags
                        .into_iter()
                        .for_each(|(_, f)| emit_warning!(f, "unused flag"));
                    props
                        .into_iter()
                        .for_each(|(_, (f, _))| emit_warning!(f, "unused property"));
                } else {
                    emit_error!(
                        span,
                        "Invalid syntax for #[rpc] helper trait";
                        usage = "#[rpc(name1 = literal_val1, name2)]";
                    );
                }
                false
            } else {
                true
            }
        })
        .collect();
    let serde_path = format!("{}::serde", root.to_token_stream());
    let serde_path = LitStr::new(serde_path.as_str(), root.span());

    let rpc_ident = trait_body.ident;
    trait_body.ident = format_ident!("{}ServerImpl", rpc_ident);
    let impl_ident = &trait_body.ident;
    let req_ident = format_ident!("{}Request", &rpc_ident);
    let rsp_ident = format_ident!("{}Response", &rpc_ident);
    let api_server_ident = format_ident!("{}Server", &rpc_ident);
    let api_stub_ident = format_ident!("{}Stub", &rpc_ident);
    let fn_name: Vec<_> = functions.iter().map(|func| &func.sig.ident).collect();
    let stub_arg_name = functions.iter().map(|func| {
        func.sig
            .inputs
            .iter()
            .filter_map(|arg| match arg {
                FnArg::Receiver(_) => None,
                FnArg::Typed(pat) => match *pat.pat {
                    Pat::Ident(ref i) => Some(i),
                    ref pat => {
                        abort!(pat, "Argument must be ident pattern"; help = "mut ident: Type");
                    }
                },
            })
            .collect::<Vec<_>>()
    });
    let fn_arg_ty = functions
        .iter()
        .map(|func| {
            func.sig
                .inputs
                .iter()
                .filter_map(|arg| match arg {
                    FnArg::Receiver(_) => None,
                    FnArg::Typed(pat) => Some(&pat.ty),
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let fn_arg_expr = functions.iter().map(|func| {
        func.sig
            .inputs
            .iter()
            .scan(0usize, |counter, arg| match arg {
                FnArg::Receiver(_) => Some(quote! { &self.0 }),
                FnArg::Typed(_) => {
                    let i = Index::from(*counter);
                    *counter += 1;
                    Some(quote! { _args.#i })
                }
            })
            .collect::<Vec<_>>()
    });
    let fn_asyncness = functions.iter().map(|func| match func.sig.asyncness {
        Some(_) => {
            quote! { .await }
        }
        None => {
            quote! {}
        }
    });
    let fn_ret_ty = functions.iter().map(|func| match func.sig.output {
        ReturnType::Default => unit.clone(),
        ReturnType::Type(_, ref ty) => *ty.clone(),
    });
    let stub_sig = functions.iter().map(|func| {
        let mut sig = func.sig.clone();
        sig.asyncness = Some(Default::default());
        sig.inputs = std::iter::once(parse_quote!(&mut self))
            .chain(sig.inputs.into_iter().filter(|arg| match arg {
                FnArg::Typed(_) => true,
                FnArg::Receiver(_) => false,
            }))
            .collect();
        let o = match sig.output {
            ReturnType::Default => unit.clone(),
            ReturnType::Type(_, ty) => *ty,
        };
        sig.output =
            parse2(quote! { -> ::std::result::Result<#o, #root::Error> }).expect("hardcode parse");
        sig
    });

    let ret = quote! {
        pub enum #rpc_ident {}

        impl #root::Rpc for #rpc_ident {
            type Request = #req_ident;
            type Response = #rsp_ident;
        }

        #trait_body

        #[derive(#root::Serialize, #root::Deserialize)]
        #[serde(crate = #serde_path)]
        #[allow(non_camel_case_types)]
        pub enum #req_ident {
            #(#fn_name((#(#fn_arg_ty,)*)),)*
        }

        #[derive(#root::Serialize, #root::Deserialize)]
        #[serde(crate = #serde_path)]
        #[allow(non_camel_case_types)]
        pub enum #rsp_ident {
            __server_error,
            #(#fn_name(#fn_ret_ty),)*
        }

        #vis struct #api_server_ident<T: #impl_ident + Sync>(T);

        impl<T: #impl_ident + Sync> #api_server_ident<T> {
            pub fn new(server_impl: T) -> Self {
                Self(server_impl)
            }
        }

        impl<T, I, O> #root::RpcServerStub<#rpc_ident, I, O> for #api_server_ident<T>
        where
            T: #impl_ident + Send + Sync + 'static,
            I: #root::RpcFrame<#req_ident>,
            O: #root::RpcFrame<#rsp_ident>,
        {
            fn make_response(
                self: #root::Arc<Self>,
                req: I,
            ) -> #root::Pin<#root::Box<dyn #root::Future<Output = Option<O>> + Send>> {
                #root::Box::pin(async move {
                    let id = I::get_id(&req);
                    let req = match I::get_data(req) {
                        Ok(req) => req,
                        Err(e) => {
                            #root::log::error!("failed to get request: {} {}", id, e);
                            return O::from_parts(id, #rsp_ident::__server_error)
                                .map_err(|e| #root::log::error!("failed to return server error: {} {}", id, e))
                                .ok();
                        }
                    };
                    let rsp = match req {
                        #(
                            #req_ident::#fn_name(_args) => {
                                #rsp_ident::#fn_name(
                                    T::#fn_name(#(#fn_arg_expr)*) #fn_asyncness
                                )
                            }
                        )*
                    };
                    O::from_parts(id, rsp)
                        .map_err(|e| #root::log::error!("failed to response: {} {}", id, e))
                        .ok()
                })
            }
        }

        #[derive(Debug)]
        #vis struct #api_stub_ident<'a, I, O>(
            #root::RpcClient<'a, #rpc_ident, I, O>,
            #root::Arc<#root::AtomicU64>
        )
        where
            I: #root::RpcFrame<#rsp_ident>,
            O: #root::RpcFrame<#req_ident>;

        impl<'a, I, O> #api_stub_ident<'a, I, O>
        where
            I: #root::RpcFrame<#rsp_ident>,
            O: #root::RpcFrame<#req_ident>,
        {
            pub fn new<T, U>(recv: T, send: U) -> Self
            where
                T: #root::Stream<Item = #root::Result<I>> + Unpin + Send + 'static,
                U: #root::Sink<O, Error = #root::Error> + Unpin + Send + 'static,
            {
                Self(
                    #root::RpcClient::new(recv, send),
                    #root::Arc::new(#root::AtomicU64::new(5)),
                )
            }

            pub fn new_with_driver<T, U>(recv: T, send: U) -> (impl #root::Future<Output = ()> + 'a, Self)
            where
                T: #root::Stream<Item = #root::Result<I>> + Unpin + 'a,
                U: #root::Sink<O, Error = #root::Error> + Unpin + 'a,
            {
                let (driver, client) = #root::RpcClient::new_with_driver(recv, send);
                (driver, Self(client, #root::Arc::new(#root::AtomicU64::new(5))))
            }

            #(
                pub #stub_sig {
                    let id = #root::RequestId(
                        self.1.fetch_add(1, #root::Ordering::SeqCst)
                    );
                    let req = O::from_parts(
                        id,
                        #req_ident::#fn_name((#(#stub_arg_name,)*)),
                    )?;
                    let rsp = self.0.make_request(req).await?;
                    match I::get_data(rsp)? {
                        #rsp_ident::#fn_name(r) => Ok(r),
                        #rsp_ident::__server_error => Err(#root::Error::ServerError(id)),
                        _ => Err(#root::Error::ResponseMismatch(id)),
                    }
                }
            )*
        }

        impl<'a, I, O> Clone for #api_stub_ident<'a, I, O>
        where
            I: #root::RpcFrame<#rsp_ident>,
            O: #root::RpcFrame<#req_ident>,
        {
            #[inline]
            fn clone(&self) -> Self {
                Self(self.0.clone(), self.1.clone())
            }
        }
    }
    .into();
    if option_env!("RUST_TRACE_MACROS").is_some() {
        println!("{}", ret);
    }
    ret
}
