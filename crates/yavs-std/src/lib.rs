//! Native semantics of `std.standard`, `std.textio` and `std.env` value
//! conversion for yavs.
//!
//! The IEEE `std` packages have declarations only, so yavs implements the
//! bodies natively. This crate holds the IR-independent part: pure Rust
//! functions over plain Rust values (`i64`, `f64`, Latin-1 `[u8]`, type
//! descriptions) that the runtime calls. Nothing here panics on bad input;
//! every failure is a structured error for `yavs-diag` to render.
//!
//! - [`image`] covers `'IMAGE`, `TO_STRING`, `TO_HSTRING`/`TO_OSTRING`/`TO_BSTRING`
//! - [`value`] covers `'VALUE`
//! - [`printf`] covers C-style real formatting (`TO_STRING(real, format)`)
//! - [`textio`] covers READ/WRITE and friends on a line buffer
//! - [`file`] covers TEXT files (READLINE, WRITELINE, ENDFILE, 2019 file ops)
//! - [`env`] covers STOP/FINISH, RESOLUTION_LIMIT and the 2019 env additions
//! - [`arith`] covers overflow-checked integer and TIME arithmetic

pub mod arith;
pub mod env;
pub mod error;
pub mod file;
pub mod image;
pub mod printf;
pub mod textio;
pub mod types;
pub mod value;
