//! WHAT A REDUCER READS (NC-9, O-12). Every reducer reads its args through [`get`], and a
//! test may record which keys a fold asked for: that is how the ICD's args are held to the
//! args a reducer reads, both ways, with no list of either kept anywhere but the ICD and
//! the reducers themselves. Outside a recording, [`get`] is `args.get(key)`.

use crate::coordinator::{ArgVal, Args};
use std::cell::RefCell;

thread_local! {
    static READ: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// `args.get(key)`, noted if this thread is recording.
pub fn get<'a>(args: &'a Args, key: &str) -> Option<&'a ArgVal> {
    READ.with(|r| {
        if let Some(keys) = r.borrow_mut().as_mut() {
            keys.push(key.to_string());
        }
    });
    args.get(key)
}

/// Run `f`, and the arg keys it read, in the order it read them.
pub fn recording<R>(f: impl FnOnce() -> R) -> (R, Vec<String>) {
    READ.with(|r| *r.borrow_mut() = Some(Vec::new()));
    let out = f();
    let keys = READ.with(|r| r.borrow_mut().take()).unwrap_or_default();
    (out, keys)
}
