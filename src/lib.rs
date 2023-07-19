// Copyright 2014-2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Scoped thread-local storage
//!
//! This module provides the ability to generate *scoped* thread-local
//! variables. In this sense, scoped indicates that thread local storage
//! actually stores a reference to a value, and this reference is only placed
//! in storage for a scoped amount of time.
//!
//! There are no restrictions on what types can be placed into a scoped
//! variable, but all scoped variables are initialized to the equivalent of
//! null. Scoped thread local storage is useful when a value is present for a known
//! period of time and it is not required to relinquish ownership of the
//! contents.
//!
//! # Examples
//!
//! ```
//! #[macro_use]
//! extern crate scoped_tls;
//!
//! scoped_thread_local!(static FOO: u32);
//!
//! # fn main() {
//! // Initially each scoped slot is empty.
//! assert!(!FOO.is_set());
//!
//! // When inserting a value, the value is only in place for the duration
//! // of the closure specified.
//! FOO.set(&1, || {
//!     FOO.with(|slot| {
//!         assert_eq!(*slot, 1);
//!     });
//! });
//! # }
//! ```

#![deny(missing_docs, warnings)]

use std::{cell::Cell, marker, ptr::NonNull, thread::LocalKey};

/// The macro. See the module level documentation for the description and examples.
#[macro_export]
macro_rules! scoped_thread_local {
    ($(#[$attrs:meta])* $vis:vis static $name:ident: $ty:ty) => (
        $(#[$attrs])*
        $vis static $name: $crate::ScopedKey<$ty> = unsafe {
            ::std::thread_local!(static FOO: ::std::cell::Cell<Option<::std::ptr::NonNull<$ty>>> = const {
                ::std::cell::Cell::new(None)
            });
            // Safety: nothing else can access FOO since it's hidden in its own scope
            $crate::ScopedKey::new(&FOO)
        };
    )
}

/// Type representing a thread local storage key corresponding to a reference
/// to the type parameter `T`.
///
/// Keys are statically allocated and can contain a reference to an instance of
/// type `T` scoped to a particular lifetime. Keys provides two methods, `set`
/// and `with`, both of which currently use closures to control the scope of
/// their contents.
pub struct ScopedKey<T: ?Sized + 'static> {
    inner: &'static LocalKey<Cell<Option<NonNull<T>>>>,
    _marker: marker::PhantomData<T>,
}

unsafe impl<T: ?Sized> Sync for ScopedKey<T> {}

impl<T: ?Sized + 'static> ScopedKey<T> {
    #[doc(hidden)]
    /// # Safety
    /// `inner` must only be accessed through `ScopedKey`'s API
    pub const unsafe fn new(inner: &'static LocalKey<Cell<Option<NonNull<T>>>>) -> Self {
        Self {
            inner,
            _marker: marker::PhantomData,
        }
    }

    /// Inserts a value into this scoped thread local storage slot for a
    /// duration of a closure.
    ///
    /// While `f` is running, the value `t` will be returned by `get` unless
    /// this function is called recursively inside of `f`.
    ///
    /// Upon return, this function will restore the previous value, if any
    /// was available.
    ///
    /// # Examples
    ///
    /// ```
    /// #[macro_use]
    /// extern crate scoped_tls;
    ///
    /// scoped_thread_local!(static FOO: u32);
    ///
    /// # fn main() {
    /// FOO.set(&100, || {
    ///     let val = FOO.with(|v| *v);
    ///     assert_eq!(val, 100);
    ///
    ///     // set can be called recursively
    ///     FOO.set(&101, || {
    ///         // ...
    ///     });
    ///
    ///     // Recursive calls restore the previous value.
    ///     let val = FOO.with(|v| *v);
    ///     assert_eq!(val, 100);
    /// });
    /// # }
    /// ```
    pub fn set<F, R>(&'static self, t: &T, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        struct Reset<T: ?Sized + 'static> {
            key: &'static LocalKey<Cell<Option<NonNull<T>>>>,
            val: Option<NonNull<T>>,
        }
        impl<T: ?Sized + 'static> Drop for Reset<T> {
            fn drop(&mut self) {
                self.key.with(|c| c.set(self.val));
            }
        }
        let prev = self.inner.with(|c| {
            let prev = c.get();
            c.set(Some(t.into()));
            prev
        });
        let _reset = Reset {
            key: self.inner,
            val: prev,
        };
        f()
    }

    /// Gets a value out of this scoped variable.
    ///
    /// This function takes a closure which receives the value of this
    /// variable.
    ///
    /// # Panics
    ///
    /// This function will panic if `set` has not previously been called.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// #[macro_use]
    /// extern crate scoped_tls;
    ///
    /// scoped_thread_local!(static FOO: u32);
    ///
    /// # fn main() {
    /// FOO.with(|slot| {
    ///     // work with `slot`
    /// # drop(slot);
    /// });
    /// # }
    /// ```
    pub fn with<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        let val = self
            .inner
            .with(|c| c.get())
            .expect("cannot access a scoped thread local variable without calling `set` first");
        unsafe { f(val.as_ref()) }
    }

    /// Test whether this TLS key has been `set` for the current thread.
    pub fn is_set(&'static self) -> bool {
        self.inner.with(|c| c.get().is_some())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        sync::mpsc::{channel, Sender},
        thread,
    };

    scoped_thread_local!(static FOO: u32);

    #[test]
    fn smoke() {
        scoped_thread_local!(static BAR: u32);

        assert!(!BAR.is_set());
        BAR.set(&1, || {
            assert!(BAR.is_set());
            BAR.with(|slot| {
                assert_eq!(*slot, 1);
            });
        });
        assert!(!BAR.is_set());
    }

    #[test]
    fn cell_allowed() {
        scoped_thread_local!(static BAR: Cell<u32>);

        BAR.set(&Cell::new(1), || {
            BAR.with(|slot| {
                assert_eq!(slot.get(), 1);
            });
        });
    }

    #[test]
    fn scope_item_allowed() {
        assert!(!FOO.is_set());
        FOO.set(&1, || {
            assert!(FOO.is_set());
            FOO.with(|slot| {
                assert_eq!(*slot, 1);
            });
        });
        assert!(!FOO.is_set());
    }

    #[test]
    fn panic_resets() {
        struct Check(Sender<u32>);
        impl Drop for Check {
            fn drop(&mut self) {
                FOO.with(|r| {
                    self.0.send(*r).unwrap();
                })
            }
        }

        let (tx, rx) = channel();
        let t = thread::spawn(|| {
            FOO.set(&1, || {
                let _r = Check(tx);

                FOO.set(&2, || panic!());
            });
        });

        assert_eq!(rx.recv().unwrap(), 1);
        assert!(t.join().is_err());
    }

    #[test]
    fn attrs_allowed() {
        scoped_thread_local!(
            /// Docs
            static BAZ: u32
        );

        scoped_thread_local!(
            #[allow(non_upper_case_globals)]
            static quux: u32
        );

        let _ = BAZ;
        let _ = quux;
    }

    #[test]
    fn unsized_tls() {
        scoped_thread_local!(static DYN: dyn std::fmt::Display);

        DYN.set(&42 as _, || {
            DYN.with(|n| assert_eq!(n.to_string(), "42"))
        });

        scoped_thread_local!(static SLICE: [i32]);

        SLICE.set(&[1, 2, 3][..], || {
            SLICE.with(|s| assert_eq!(s, &[1, 2, 3][..]))
        });
    }
}
