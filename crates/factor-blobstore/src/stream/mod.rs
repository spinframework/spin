//! Adapts the WASI streams to allow closing without resource mapping.
//!
//! The solution that the Wasmtime/WASI folks advice is to map your child
//! resources to a custom type, and have the parent "close child" function
//! get the child resource and call a suitable function to termimate it.
//! Unfortunately, that requires (as far as I know) the binding expression
//! to know about the custom type.  And since we do all our binding in
//! `spin-world`, which cannot depend on factor crates because it would make
//! things circular, we need to work around it by implementing a close
//! side channel on our own OutputStream implementation.
//! And that is what this module does.

mod write_stream;

pub use write_stream::AsyncWriteStream;
