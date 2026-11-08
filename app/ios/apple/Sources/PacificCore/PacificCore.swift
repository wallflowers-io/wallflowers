// PacificCore — the idiomatic surface the SwiftUI app imports.
//
// It re-exports the UniFFI-generated bindings (the `Core` handle, `ObjectRow`,
// `FfiError`) so views never touch raw FFI symbols. Keep app-facing conveniences
// here as the API grows; the generated glue stays hidden behind this one import.
@_exported import UniFFI
