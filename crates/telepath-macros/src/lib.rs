//! Proc-macro crate for Telepath.
//!
//! Provides the `#[command]` attribute macro that generates a type-erased shim
//! function and a `CommandMetadata` const from a plain Rust function definition.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, FnArg, ItemFn, Pat, ReturnType, Type};

/// Marks a function as a Telepath RPC command.
///
/// # What it generates
///
/// For every annotated function the macro emits five additional items:
///
/// 1. **`fn __telepath_shim_<name>(input: &[u8], output: &mut [u8]) -> Result<usize, DispatchError>`** —
///    deserializes `input` via postcard, calls the original function, and serializes the result
///    into `output`.
/// 2. **`fn __telepath_args_schema_<name>(out: &mut [u8]) -> Result<usize, ()>`** —
///    writes a postcard-encoded `postcard_schema::schema::NamedType` for the argument tuple
///    into `out` and returns the byte count.
/// 3. **`fn __telepath_ret_schema_<name>(out: &mut [u8]) -> Result<usize, ()>`** —
///    same for the return type.
/// 4. **`pub const __TELEPATH_CMD_<NAME>: CommandMetadata`** — a `CommandMetadata` const whose
///    `id` is derived deterministically from the function's signature via
///    `derive_cmd_id` at build time.
/// 5. **`#[linkme] static __TELEPATH_REG_<NAME>`** — registers the metadata in
///    [`telepath_firmware::TELEPATH_COMMANDS`] at link time.
///
/// The original function body is preserved unchanged so it remains directly callable.
///
/// # Requirements on the calling crate
///
/// The calling crate must declare the following direct dependencies:
/// - `telepath-firmware` — provides `CommandMetadata`, `DispatchError`, and re-exports
///   `postcard_schema` and `linkme` for use in generated code.
/// - `postcard` — used in the generated shim for (de)serialization
///
/// All argument types and the return type must implement
/// `postcard_schema::Schema`. Built-in primitives (`u8`, `u32`, `()`,
/// standard tuples, etc.) already implement it. For user-defined types,
/// add `#[derive(postcard_schema::Schema)]`.
///
/// # Restrictions
///
/// The macro rejects functions that are:
/// - `async fn` (RPC dispatch is synchronous)
/// - `unsafe fn`
/// - Generic (`<T>` / `where` clauses)
/// - Methods (`self` receiver)
/// - Functions with reference arguments or reference return types
/// - Functions with pattern-destructured arguments
///
/// # Example
///
/// ```rust,ignore
/// use telepath_firmware::{command, CommandMetadata};
///
/// #[command]
/// fn ping() -> u32 {
///     0xDEAD_BEEF
/// }
///
/// static COMMANDS: [CommandMetadata; 1] = [__TELEPATH_CMD_PING];
/// ```
#[proc_macro_attribute]
pub fn command(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    match expand_command(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_command(func: ItemFn) -> syn::Result<proc_macro2::TokenStream> {
    let fn_ident = &func.sig.ident;
    let fn_name_str = fn_ident.to_string();

    // --- Validation ---

    if let Some(tok) = &func.sig.asyncness {
        return Err(syn::Error::new_spanned(
            tok,
            "#[command] does not support async fn",
        ));
    }
    if let Some(tok) = &func.sig.unsafety {
        return Err(syn::Error::new_spanned(
            tok,
            "#[command] does not support unsafe fn",
        ));
    }
    if !func.sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &func.sig.generics,
            "#[command] does not support generic functions",
        ));
    }
    if let Some(wc) = &func.sig.generics.where_clause {
        return Err(syn::Error::new_spanned(
            wc,
            "#[command] does not support where clauses",
        ));
    }

    // --- Parse arguments ---

    let mut arg_idents = Vec::new();
    let mut arg_types: Vec<Box<Type>> = Vec::new();
    let mut arg_type_strs = Vec::new();

    for fn_arg in &func.sig.inputs {
        match fn_arg {
            FnArg::Receiver(recv) => {
                return Err(syn::Error::new_spanned(
                    recv,
                    "#[command] cannot be applied to methods",
                ));
            }
            FnArg::Typed(pat_type) => {
                let ident = match pat_type.pat.as_ref() {
                    Pat::Ident(pi) => pi.ident.clone(),
                    other => {
                        return Err(syn::Error::new_spanned(
                            other,
                            "#[command] requires simple named arguments (patterns not supported)",
                        ));
                    }
                };
                if let Type::Reference(r) = pat_type.ty.as_ref() {
                    return Err(syn::Error::new_spanned(
                        r,
                        "#[command] does not support reference arguments",
                    ));
                }
                let ty = &*pat_type.ty;
                arg_type_strs.push(quote! { #ty }.to_string());
                arg_idents.push(ident);
                arg_types.push(pat_type.ty.clone());
            }
        }
    }

    // --- Parse return type ---

    let ret_type_str = match &func.sig.output {
        ReturnType::Default => "()".to_string(),
        ReturnType::Type(_, ty) => {
            if let Type::Reference(r) = ty.as_ref() {
                return Err(syn::Error::new_spanned(
                    r,
                    "#[command] does not support reference return types",
                ));
            }
            quote! { #ty }.to_string()
        }
    };

    // --- Build args_type_str ---
    // Canonical tuple format matching Rust syntax: "()" for 0-arg, "(T,)" for 1-arg,
    // "(T1, T2)" for 2-arg. Must match the tuple type used for postcard deserialization.

    let args_type_str = if arg_type_strs.is_empty() {
        "()".to_string()
    } else if arg_type_strs.len() == 1 {
        format!("({},)", arg_type_strs[0])
    } else {
        format!("({})", arg_type_strs.join(", "))
    };

    // --- Generated identifiers ---

    let shim_ident = format_ident!("__telepath_shim_{}", fn_name_str);
    let args_schema_ident = format_ident!("__telepath_args_schema_{}", fn_name_str);
    let ret_schema_ident = format_ident!("__telepath_ret_schema_{}", fn_name_str);
    let static_ident = format_ident!("__TELEPATH_CMD_{}", fn_name_str.to_uppercase());
    let reg_ident = format_ident!("__TELEPATH_REG_{}", fn_name_str.to_uppercase());

    // --- Compute args tuple type and ret type tokens for schema writers ---

    let args_schema_type = if arg_types.is_empty() {
        quote! { () }
    } else if arg_types.len() == 1 {
        let t = &*arg_types[0];
        quote! { (#t,) }
    } else {
        quote! { (#(#arg_types),*) }
    };

    let ret_schema_type = match &func.sig.output {
        ReturnType::Default => quote! { () },
        ReturnType::Type(_, ty) => quote! { #ty },
    };

    // --- Build shim body ---

    let shim_body = if arg_idents.is_empty() {
        quote! {
            if !input.is_empty() {
                return ::core::result::Result::Err(
                    ::telepath_firmware::DispatchError::DeserializeError
                );
            }
            let __ret = #fn_ident();
            match ::postcard::to_slice(&__ret, output) {
                Ok(s) => ::core::result::Result::Ok(s.len()),
                Err(_) => ::core::result::Result::Err(
                    ::telepath_firmware::DispatchError::SerializeError
                ),
            }
        }
    } else {
        // Tuple type for deserialization: (T,) for 1-element, (T1, T2) for multi.
        let args_tuple_type = if arg_types.len() == 1 {
            let t = &*arg_types[0];
            quote! { (#t,) }
        } else {
            quote! { (#(#arg_types),*) }
        };
        // Destructuring pattern mirroring the tuple type.
        let destructure_pat = if arg_idents.len() == 1 {
            let id = &arg_idents[0];
            quote! { (#id,) }
        } else {
            quote! { (#(#arg_idents),*) }
        };
        quote! {
            let #destructure_pat: #args_tuple_type = match ::postcard::from_bytes(input) {
                Ok(v) => v,
                Err(_) => return ::core::result::Result::Err(
                    ::telepath_firmware::DispatchError::DeserializeError
                ),
            };
            let __ret = #fn_ident(#(#arg_idents),*);
            match ::postcard::to_slice(&__ret, output) {
                Ok(s) => ::core::result::Result::Ok(s.len()),
                Err(_) => ::core::result::Result::Err(
                    ::telepath_firmware::DispatchError::SerializeError
                ),
            }
        }
    };

    Ok(quote! {
        #func

        #[allow(non_snake_case)]
        fn #shim_ident(
            input: &[u8],
            output: &mut [u8],
        ) -> ::core::result::Result<usize, ::telepath_firmware::DispatchError> {
            #shim_body
        }

        #[allow(non_snake_case)]
        fn #args_schema_ident(out: &mut [u8]) -> ::core::result::Result<usize, ()> {
            ::postcard::to_slice(
                <#args_schema_type as ::telepath_firmware::__postcard_schema::Schema>::SCHEMA,
                out,
            )
            .map(|s| s.len())
            .map_err(|_| ())
        }

        #[allow(non_snake_case)]
        fn #ret_schema_ident(out: &mut [u8]) -> ::core::result::Result<usize, ()> {
            ::postcard::to_slice(
                <#ret_schema_type as ::telepath_firmware::__postcard_schema::Schema>::SCHEMA,
                out,
            )
            .map(|s| s.len())
            .map_err(|_| ())
        }

        pub const #static_ident: ::telepath_firmware::CommandMetadata =
            ::telepath_firmware::CommandMetadata {
                name: #fn_name_str,
                id: ::telepath_firmware::__derive_cmd_id(
                    #fn_name_str,
                    #args_type_str,
                    #ret_type_str,
                ),
                invoke: #shim_ident,
                args_schema: #args_schema_ident,
                ret_schema: #ret_schema_ident,
            };

        #[allow(non_upper_case_globals, non_snake_case)]
        #[::telepath_firmware::__linkme::distributed_slice(::telepath_firmware::TELEPATH_COMMANDS)]
        #[linkme(crate = ::telepath_firmware::__linkme)]
        static #reg_ident: ::telepath_firmware::CommandMetadata = #static_ident;

    })
}
