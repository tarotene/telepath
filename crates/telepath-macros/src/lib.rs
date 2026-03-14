//! Proc-macro crate for Telepath.
//!
//! Currently provides a passthrough stub for `#[command]`. Future versions will
//! generate:
//! - A type-erased shim function (`__telepath_shim_<name>`)
//! - A `CommandMetadata` static registered via `linkme` distributed slice
//! - Argument / result struct definitions for postcard de/serialization

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

/// Marks a function as a Telepath RPC command.
///
/// # Current behavior (stub)
///
/// The function is passed through unchanged. No code generation occurs yet.
///
/// # Planned behavior
///
/// When fully implemented this macro will:
/// 1. Derive `Serialize`/`Deserialize` argument and result wrapper structs.
/// 2. Generate a type-erased shim: `fn __telepath_shim_<name>(&[u8], &mut [u8]) -> Result<usize, DispatchError>`.
/// 3. Register a `CommandMetadata` static in the `.telepath_commands` linker section
///    via `linkme::distributed_slice`.
///
/// # Example
///
/// ```rust,ignore
/// use telepath_macros::command;
///
/// #[command]
/// fn set_led(id: u8, brightness: u16) -> Result<(), AppError> {
///     // hardware control
///     Ok(())
/// }
/// ```
#[proc_macro_attribute]
pub fn command(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    // Passthrough: emit the original function unchanged.
    // TODO: generate shim, metadata static, and linkme registration.
    quote! { #input }.into()
}
