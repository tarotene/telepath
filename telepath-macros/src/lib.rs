//! Proc-macro crate for Telepath.
//!
//! Provides the `#[command]` attribute macro that generates a type-erased shim
//! function and a `CommandMetadata` const from a plain Rust function definition.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use syn::{parse_macro_input, FnArg, ItemFn, Pat, ReturnType, Type, TypeReference};
use telepath_wire::cmd_id::derive_cmd_id as compute_cmd_id;

fn seen_cmd_ids() -> &'static Mutex<HashMap<u16, String>> {
    static SEEN: OnceLock<Mutex<HashMap<u16, String>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Marks a function as a Telepath RPC command.
///
/// # What it generates
///
/// For every annotated function the macro emits five additional items:
///
/// 1. **`fn __telepath_shim_<name>(input: &[u8], output: &mut [u8], resources: &ResourceRegistry) -> Result<usize, DispatchError>`** —
///    deserializes `input` via postcard, resolves `#[resource]`-annotated arguments from
///    `resources`, calls the original function, and serializes the result into `output`.
/// 2. **`fn __telepath_args_schema_<name>(out: &mut [u8]) -> Result<usize, ()>`** —
///    writes a postcard-encoded `postcard_schema::schema::NamedType` for the argument tuple
///    into `out` and returns the byte count.
/// 3. **`fn __telepath_ret_schema_<name>(out: &mut [u8]) -> Result<usize, ()>`** —
///    same for the return type.
/// 4. **`pub const __TELEPATH_CMD_<NAME>: CommandMetadata`** — a `CommandMetadata` const whose
///    `id` is derived deterministically from the function's signature via
///    `derive_cmd_id` at build time.
/// 5. **`#[linkme] static __TELEPATH_REG_<NAME>`** — registers the metadata in
///    [`telepath_server::TELEPATH_COMMANDS`] at link time.
///
/// The original function body is preserved unchanged so it remains directly callable.
///
/// # Requirements on the calling crate
///
/// The calling crate must declare the following direct dependencies:
/// - `telepath-server` — provides `CommandMetadata`, `DispatchError`, and re-exports
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
/// use telepath_server::{command, CommandMetadata};
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

    // Wire arguments: deserialized from the postcard request payload.
    let mut wire_idents = Vec::new();
    let mut wire_types: Vec<Box<Type>> = Vec::new();
    let mut wire_type_strs = Vec::new();

    // Resource arguments: injected from the ResourceRegistry.
    struct ResourceArg {
        ident: syn::Ident,
        inner_ty: Box<Type>,
        is_mut: bool,
    }
    let mut resource_args: Vec<ResourceArg> = Vec::new();

    // All argument idents in declaration order, for calling the original function.
    let mut all_arg_idents: Vec<syn::Ident> = Vec::new();

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

                let is_resource = pat_type.attrs.iter().any(|a| a.path().is_ident("resource"));

                if is_resource {
                    let Type::Reference(TypeReference {
                        elem, mutability, ..
                    }) = pat_type.ty.as_ref()
                    else {
                        return Err(syn::Error::new_spanned(
                            &pat_type.ty,
                            "#[resource] arguments must be &T or &mut T",
                        ));
                    };

                    // Best-effort compile-time uniqueness check via token-string comparison.
                    // Type aliases or differently-spelled paths for the same concrete type
                    // may slip through; `ResourceRegistry::insert` panics at runtime as a
                    // fallback in those cases.
                    let inner_str = quote! { #elem }.to_string();
                    for existing in &resource_args {
                        let existing_ty = &existing.inner_ty;
                        let existing_str = quote! { #existing_ty }.to_string();
                        if existing_str == inner_str {
                            return Err(syn::Error::new_spanned(
                                &pat_type.ty,
                                "duplicate #[resource] type; each resource type may appear at most once",
                            ));
                        }
                    }

                    resource_args.push(ResourceArg {
                        ident: ident.clone(),
                        inner_ty: elem.clone(),
                        is_mut: mutability.is_some(),
                    });
                    all_arg_idents.push(ident);
                } else {
                    if let Type::Reference(r) = pat_type.ty.as_ref() {
                        return Err(syn::Error::new_spanned(
                            r,
                            "#[command] does not support reference arguments \
                             (use #[resource] for injected references)",
                        ));
                    }
                    let ty = &*pat_type.ty;
                    wire_type_strs.push(quote! { #ty }.to_string());
                    wire_idents.push(ident.clone());
                    wire_types.push(pat_type.ty.clone());
                    all_arg_idents.push(ident);
                }
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

    // --- Build arg_names_str ---
    // Comma-joined wire argument names for runtime introspection (e.g. "a,b").
    // Resource arguments are excluded — they are server-side only.
    let arg_names_str: String = wire_idents
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");

    // --- Build args_type_str ---
    // Canonical tuple format of wire arguments matching Rust syntax: "()" for 0-arg,
    // "(T,)" for 1-arg, "(T1, T2)" for 2-arg. Resource arguments are excluded.

    let args_type_str = if wire_type_strs.is_empty() {
        "()".to_string()
    } else if wire_type_strs.len() == 1 {
        format!("({},)", wire_type_strs[0])
    } else {
        format!("({})", wire_type_strs.join(", "))
    };

    // --- Duplicate cmd_id detection ---
    //
    // Compute the cmd_id at macro-expansion time so we can:
    // 1. Check for same-crate collisions via an in-process registry → compile_error!
    // 2. Emit a link-time guard symbol (export_name keyed on the hex id) that causes
    //    a "multiple definition" linker error when two commands from different crates
    //    happen to share the same id in the final binary.

    let cmd_id_value = compute_cmd_id(&fn_name_str, &args_type_str, &ret_type_str);

    {
        let mut seen = seen_cmd_ids().lock().unwrap();
        if let Some(existing) = seen.get(&cmd_id_value) {
            return Err(syn::Error::new_spanned(
                fn_ident,
                format!(
                    "#[command] cmd_id collision: `{}` and `{}` both map to 0x{:04X}. \
                     Rename one of the commands to avoid the collision.",
                    fn_name_str, existing, cmd_id_value
                ),
            ));
        }
        seen.insert(cmd_id_value, fn_name_str.clone());
    }

    let collision_export = format!("__telepath_cmd_id_{:04X}", cmd_id_value);
    let guard_ident = format_ident!("__TELEPATH_CMDID_GUARD_{}", fn_name_str.to_uppercase());

    // --- Generated identifiers ---

    let shim_ident = format_ident!("__telepath_shim_{}", fn_name_str);
    let args_schema_ident = format_ident!("__telepath_args_schema_{}", fn_name_str);
    let ret_schema_ident = format_ident!("__telepath_ret_schema_{}", fn_name_str);
    let static_ident = format_ident!("__TELEPATH_CMD_{}", fn_name_str.to_uppercase());
    let reg_ident = format_ident!("__TELEPATH_REG_{}", fn_name_str.to_uppercase());

    // --- Compute args tuple type and ret type tokens for schema writers ---
    // Only wire arguments participate in schemas and CmdID derivation.

    let args_schema_type = if wire_types.is_empty() {
        quote! { () }
    } else if wire_types.len() == 1 {
        let t = &*wire_types[0];
        quote! { (#t,) }
    } else {
        quote! { (#(#wire_types),*) }
    };

    let ret_schema_type = match &func.sig.output {
        ReturnType::Default => quote! { () },
        ReturnType::Type(_, ty) => quote! { #ty },
    };

    // --- Build shim body ---

    // Wire-arg deserialization
    let wire_deser = if wire_idents.is_empty() {
        quote! {
            if !input.is_empty() {
                return ::core::result::Result::Err(
                    ::telepath_server::DispatchError::DeserializeError
                );
            }
        }
    } else {
        let wire_tuple_type = if wire_types.len() == 1 {
            let t = &*wire_types[0];
            quote! { (#t,) }
        } else {
            quote! { (#(#wire_types),*) }
        };
        let wire_pat = if wire_idents.len() == 1 {
            let id = &wire_idents[0];
            quote! { (#id,) }
        } else {
            quote! { (#(#wire_idents),*) }
        };
        quote! {
            let #wire_pat: #wire_tuple_type = match ::postcard::from_bytes(input) {
                Ok(v) => v,
                Err(_) => return ::core::result::Result::Err(
                    ::telepath_server::DispatchError::DeserializeError
                ),
            };
        }
    };

    // Resource lookups
    let resource_lookups: Vec<_> = resource_args
        .iter()
        .map(|ra| {
            let ident = &ra.ident;
            let inner_ty = &ra.inner_ty;
            if ra.is_mut {
                quote! {
                    let #ident: &mut #inner_ty = unsafe {
                        &mut *__resources.get_ptr::<#inner_ty>()
                            .ok_or(::telepath_server::DispatchError::ResourceUnavailable)?
                    };
                }
            } else {
                quote! {
                    let #ident: &#inner_ty = unsafe {
                        &*__resources.get_ptr::<#inner_ty>()
                            .ok_or(::telepath_server::DispatchError::ResourceUnavailable)?
                    };
                }
            }
        })
        .collect();

    // Call arguments in declaration order
    let call_args: Vec<_> = all_arg_idents
        .iter()
        .map(|ident| quote! { #ident })
        .collect();

    let shim_body = quote! {
        #wire_deser
        #(#resource_lookups)*
        let __ret = #fn_ident(#(#call_args),*);
        match ::postcard::to_slice(&__ret, output) {
            Ok(s) => ::core::result::Result::Ok(s.len()),
            Err(_) => ::core::result::Result::Err(
                ::telepath_server::DispatchError::SerializeError
            ),
        }
    };

    // Strip #[resource] attributes from the original function so that
    // it compiles as a normal function with reference parameters.
    let mut clean_func = func.clone();
    for fn_arg in &mut clean_func.sig.inputs {
        if let FnArg::Typed(pat_type) = fn_arg {
            pat_type.attrs.retain(|a| !a.path().is_ident("resource"));
        }
    }

    Ok(quote! {
        #clean_func

        #[allow(non_snake_case)]
        fn #shim_ident(
            input: &[u8],
            output: &mut [u8],
            __resources: &::telepath_server::ResourceRegistry,
        ) -> ::core::result::Result<usize, ::telepath_server::DispatchError> {
            #shim_body
        }

        #[allow(non_snake_case)]
        fn #args_schema_ident(out: &mut [u8]) -> ::core::result::Result<usize, ()> {
            ::postcard::to_slice(
                <#args_schema_type as ::telepath_server::__postcard_schema::Schema>::SCHEMA,
                out,
            )
            .map(|s| s.len())
            .map_err(|_| ())
        }

        #[allow(non_snake_case)]
        fn #ret_schema_ident(out: &mut [u8]) -> ::core::result::Result<usize, ()> {
            ::postcard::to_slice(
                <#ret_schema_type as ::telepath_server::__postcard_schema::Schema>::SCHEMA,
                out,
            )
            .map(|s| s.len())
            .map_err(|_| ())
        }

        pub const #static_ident: ::telepath_server::CommandMetadata =
            ::telepath_server::CommandMetadata {
                name: #fn_name_str,
                id: ::telepath_server::__derive_cmd_id(
                    #fn_name_str,
                    #args_type_str,
                    #ret_type_str,
                ),
                invoke: #shim_ident,
                args_schema: #args_schema_ident,
                ret_schema: #ret_schema_ident,
                arg_names: #arg_names_str,
            };

        #[allow(non_upper_case_globals, non_snake_case)]
        #[::telepath_server::__linkme::distributed_slice(::telepath_server::TELEPATH_COMMANDS)]
        #[linkme(crate = ::telepath_server::__linkme)]
        static #reg_ident: ::telepath_server::CommandMetadata = #static_ident;

        // Link-time duplicate cmd_id guard.
        //
        // If two #[command] functions in the same binary (possibly from different
        // crates) share the same cmd_id, the linker will emit a "multiple
        // definition" error for `__telepath_cmd_id_XXXX`, stopping the build
        // before the firmware is ever flashed.
        //
        // The in-process check above already catches same-crate collisions as a
        // nicer compile_error!; this symbol is the defense-in-depth for
        // incremental builds and cross-crate collisions.
        #[doc(hidden)]
        #[allow(non_upper_case_globals, dead_code)]
        #[used]
        #[export_name = #collision_export]
        pub static #guard_ident: u8 = 0;

    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serializes all tests that touch the global seen_cmd_ids() registry.
    static TEST_GUARD: Mutex<()> = Mutex::new(());

    fn parse_fn(src: &str) -> ItemFn {
        syn::parse_str(src).unwrap()
    }

    #[test]
    fn same_crate_collision_is_rejected() {
        let _g = TEST_GUARD.lock().unwrap();
        seen_cmd_ids().lock().unwrap().clear();
        // cmd_446() -> u32 and cmd_470() -> u32 both map to 0x43AE (verified by brute force).
        assert!(expand_command(parse_fn("fn cmd_446() -> u32 { 0 }")).is_ok());
        let err = expand_command(parse_fn("fn cmd_470() -> u32 { 0 }"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("cmd_id collision"),
            "expected collision error, got: {err}"
        );
        assert!(
            err.contains("0x43AE"),
            "expected hex id 0x43AE in error, got: {err}"
        );
        assert!(
            err.contains("cmd_446") && err.contains("cmd_470"),
            "expected both command names in error, got: {err}"
        );
        seen_cmd_ids().lock().unwrap().clear();
    }

    #[test]
    fn guard_symbol_has_correct_export_name() {
        let _g = TEST_GUARD.lock().unwrap();
        seen_cmd_ids().lock().unwrap().clear();
        let ts = expand_command(parse_fn("fn cmd_446() -> u32 { 0 }"))
            .unwrap()
            .to_string();
        // Guard static export_name encodes the cmd_id as uppercase hex.
        assert!(
            ts.contains("__telepath_cmd_id_43AE"),
            "guard symbol export_name not found in generated code: {ts}"
        );
        seen_cmd_ids().lock().unwrap().clear();
    }

    #[test]
    fn distinct_commands_do_not_collide() {
        let _g = TEST_GUARD.lock().unwrap();
        seen_cmd_ids().lock().unwrap().clear();
        assert!(expand_command(parse_fn("fn ping() -> u32 { 0 }")).is_ok());
        assert!(expand_command(parse_fn("fn echo(x: u32) -> u32 { x }")).is_ok());
        seen_cmd_ids().lock().unwrap().clear();
    }
}
