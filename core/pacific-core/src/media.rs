//! MEDIA — re-exported from the dependency-free `pacific-media` crate.
//!
//! The types live there rather than here so the same validation and the same
//! codec can be compiled into the browser demos. Every existing
//! `crate::media::MediaRef` path keeps resolving through this re-export.

pub use pacific_media::media::*;
pub use pacific_media::{bc1, pyramid, ArgVal as MediaArgVal, Args as MediaArgs};
