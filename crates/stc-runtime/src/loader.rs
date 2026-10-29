//! Platform loader for host ABI 2 modules.
//!
//! The loader maps a shared library, resolves the fixed module entry points from
//! GOT Artifacts §7.3, and copies the descriptor blob and function-address table
//! out of module memory. Everything the host needs stays valid after
//! [`LoadedModule`] is dropped, so retiring and unloading a generation never
//! invalidates host-side data.
//!
//! Load failures are non-fatal: the caller keeps the active generation and
//! reports the module diagnostic. Modules are trusted native code in v1
//! (Architecture §8), so calling an entry point is safe only in the sense that
//! the artifact format was validated; the code itself has the host's
//! privileges.
//!
//! Linux modules load with `RTLD_NOW | RTLD_LOCAL`: every import resolves
//! before the first entry point runs, and the module's exports stay out of the
//! global symbol scope, so a retired generation's symbols can never satisfy a
//! later module's unresolved imports.

use std::ffi::c_void;
use std::path::Path;

use stc_got::error::{Error, Result, codes};

use crate::host_api::HostApi;

/// Descriptor accessor: returns the descriptor bytes and writes the length.
pub type DescriptorFn = unsafe extern "C" fn(*mut u64) -> *const u8;
/// Function-table accessor: returns code addresses and writes the count.
pub type FunctionsFn = unsafe extern "C" fn(*mut u32) -> *const *const c_void;
/// Attach entry point.
pub type AttachFn = unsafe extern "C" fn(*const HostApi, *const *mut u8, u32, u64) -> i32;
/// Prepare entry point.
pub type PrepareFn = unsafe extern "C" fn(u32, u64, *const u8, u32) -> i32;
/// Cycle entry point.
pub type CycleFn = unsafe extern "C" fn(u64) -> i32;
/// Shutdown entry point.
pub type ShutdownFn = unsafe extern "C" fn(u32) -> i32;
/// Detach entry point.
pub type DetachFn = unsafe extern "C" fn(u64);

/// Entry-point names from GOT Artifacts §7.3.
pub mod symbol {
    /// Descriptor accessor.
    pub const DESCRIPTOR: &str = "stc_module_descriptor";
    /// Function-address accessor.
    pub const FUNCTIONS: &str = "stc_module_functions";
    /// Attach entry point.
    pub const ATTACH: &str = "stc_module_attach";
    /// Prepare entry point.
    pub const PREPARE: &str = "stc_module_prepare";
    /// Cycle entry point.
    pub const CYCLE: &str = "stc_module_cycle";
    /// Shutdown entry point.
    pub const SHUTDOWN: &str = "stc_module_shutdown";
    /// Detach entry point.
    pub const DETACH: &str = "stc_module_detach";
}

/// Largest descriptor blob a host accepts.
pub const MAX_DESCRIPTOR_BYTES: u64 = 16 * 1024 * 1024;

/// Largest function-address table a host accepts.
pub const MAX_FUNCTIONS: u32 = 1_000_000;

/// A loaded shared library with its ABI 2 metadata and entry points.
///
/// Dropping the value unloads the library. A host generation keeps one alive
/// until the generation is retired and unloaded.
pub struct LoadedModule {
    _library: Library,
    descriptor: Vec<u8>,
    functions: Vec<*const c_void>,
    attach: AttachFn,
    prepare: PrepareFn,
    cycle: CycleFn,
    shutdown: ShutdownFn,
    detach: DetachFn,
}

// The platform library handle and copied code addresses are process-wide values.
// The host serializes all entry-point calls; moving a loaded module between
// threads does not permit concurrent execution.
unsafe impl Send for LoadedModule {}

impl LoadedModule {
    /// Loads a module and copies its descriptor and function table.
    ///
    /// No entry point other than the two metadata accessors is called here.
    ///
    /// # Errors
    ///
    /// Rejects a missing library or symbol, a null or oversized descriptor, a
    /// malformed function table, or an entry-point count mismatch.
    pub fn load(path: &Path) -> Result<Self> {
        let library = Library::open(path)?;
        let descriptor_symbol = library.symbol(symbol::DESCRIPTOR)?;
        let functions_symbol = library.symbol(symbol::FUNCTIONS)?;
        let attach = library.entry::<AttachFn>(symbol::ATTACH)?;
        let prepare = library.entry::<PrepareFn>(symbol::PREPARE)?;
        let cycle = library.entry::<CycleFn>(symbol::CYCLE)?;
        let shutdown = library.entry::<ShutdownFn>(symbol::SHUTDOWN)?;
        let detach = library.entry::<DetachFn>(symbol::DETACH)?;

        let descriptor_fn: DescriptorFn = unsafe { std::mem::transmute_copy(&descriptor_symbol) };
        let functions_fn: FunctionsFn = unsafe { std::mem::transmute_copy(&functions_symbol) };

        let mut size = 0_u64;
        // SAFETY: the symbol has the descriptor accessor signature from
        // GOT Artifacts §7.3 and is called before attach with a valid out
        // pointer. Modules are trusted native code in v1.
        let descriptor_pointer = unsafe { descriptor_fn(&mut size) };
        if descriptor_pointer.is_null() {
            return Err(module_error("descriptor accessor returned a null pointer"));
        }
        if size == 0 || size > MAX_DESCRIPTOR_BYTES {
            return Err(module_error(format!(
                "descriptor length {size} is not between 1 and {MAX_DESCRIPTOR_BYTES}"
            )));
        }
        let size_usize = usize::try_from(size)
            .map_err(|_| module_error("descriptor length exceeds the platform limit"))?;
        // SAFETY: the module contract says the pointer is valid for `size`
        // bytes until unload; the bytes are copied immediately.
        let descriptor =
            unsafe { std::slice::from_raw_parts(descriptor_pointer, size_usize) }.to_vec();

        let mut count = 0_u32;
        // SAFETY: the symbol has the function-table accessor signature and is
        // called with a valid out pointer.
        let table = unsafe { functions_fn(&mut count) };
        if count > MAX_FUNCTIONS {
            return Err(module_error(format!(
                "function count {count} exceeds the host limit {MAX_FUNCTIONS}"
            )));
        }
        if count > 0 && table.is_null() {
            return Err(module_error("function table is null with a nonzero count"));
        }
        let mut functions = Vec::with_capacity(count as usize);
        for index in 0..count {
            // SAFETY: the module contract says the table has `count` entries.
            let address = unsafe { *table.add(index as usize) };
            if address.is_null() {
                return Err(module_error(format!(
                    "function table entry {index} is null"
                )));
            }
            functions.push(address);
        }

        Ok(Self {
            _library: library,
            descriptor,
            functions,
            attach,
            prepare,
            cycle,
            shutdown,
            detach,
        })
    }

    /// Returns the copied descriptor blob.
    #[must_use]
    pub fn descriptor_bytes(&self) -> &[u8] {
        &self.descriptor
    }

    /// Returns the copied function-address table in descriptor order.
    #[must_use]
    pub fn function_addresses(&self) -> &[*const c_void] {
        &self.functions
    }

    /// Resolves an exported module symbol by name.
    ///
    /// This serves the embedding host's export registry. The address is valid
    /// while this module remains loaded.
    #[must_use]
    pub fn export_address(&self, name: &str) -> Option<*const c_void> {
        platform::symbol(&self._library.handle, name).ok()
    }

    /// Calls `stc_module_attach`.
    ///
    /// # Safety
    ///
    /// `api` must point to a valid host API table and `slots` must point to a
    /// generation-specific array of `slot_count` cell addresses.
    #[must_use]
    pub unsafe fn attach(
        &self,
        api: *const HostApi,
        slots: *const *mut u8,
        slot_count: u32,
        generation: u64,
    ) -> i32 {
        // SAFETY: the caller contract above supplies valid table and slot
        // pointers, and the module is called by the thread that owns the
        // generation.
        unsafe { (self.attach)(api, slots, slot_count, generation) }
    }

    /// Calls `stc_module_prepare`.
    ///
    /// # Safety
    ///
    /// `fresh_root_mask` must point to a readable `mask_size`-byte mask, or be
    /// null with a zero size.
    #[must_use]
    pub unsafe fn prepare(
        &self,
        mode: u32,
        generation: u64,
        fresh_root_mask: *const u8,
        mask_size: u32,
    ) -> i32 {
        // SAFETY: the caller contract above guarantees the mask range, and
        // the mask stays alive for the duration of the call.
        unsafe { (self.prepare)(mode, generation, fresh_root_mask, mask_size) }
    }

    /// Calls `stc_module_cycle`.
    #[must_use]
    pub fn cycle(&self, generation: u64) -> i32 {
        // SAFETY: the host serializes cycle calls for this generation.
        unsafe { (self.cycle)(generation) }
    }

    /// Calls `stc_module_shutdown`.
    #[must_use]
    pub fn shutdown(&self, mode: u32) -> i32 {
        // SAFETY: the host serializes shutdown calls for this generation.
        unsafe { (self.shutdown)(mode) }
    }

    /// Calls `stc_module_detach`.
    pub fn detach(&self, generation: u64) {
        // SAFETY: the host calls detach at most once per attached generation.
        unsafe { (self.detach)(generation) };
    }
}

fn module_error(message: impl Into<String>) -> Error {
    Error::new(codes::MODULE_INVALID, message)
}

#[cfg(windows)]
type NativeHandle = *mut c_void;

#[cfg(target_os = "linux")]
type NativeHandle = *mut c_void;

#[cfg(not(any(windows, target_os = "linux")))]
type NativeHandle = ();

/// An owned platform library handle.
struct Library {
    handle: NativeHandle,
}

impl Library {
    fn open(path: &Path) -> Result<Self> {
        platform::open(path).map(|handle| Self { handle })
    }

    fn symbol(&self, name: &str) -> Result<*const c_void> {
        platform::symbol(&self.handle, name)
    }

    fn entry<T: Copy>(&self, name: &str) -> Result<T> {
        let address = self.symbol(name)?;
        debug_assert_eq!(
            std::mem::size_of::<T>(),
            std::mem::size_of::<*const c_void>()
        );
        // SAFETY: platform function symbols are function pointers, and
        // `debug_assert_eq!` documents that `T` has pointer size. The address
        // is non-null and names the expected entry point.
        Ok(unsafe { std::mem::transmute_copy(&address) })
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        platform::close(self.handle);
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::{CString, c_void};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use stc_got::error::{Error, Result, codes};

    use super::NativeHandle;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(path: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
    }

    pub(super) fn open(path: &Path) -> Result<NativeHandle> {
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        // SAFETY: `wide` is a NUL-terminated UTF-16 path.
        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        if handle.is_null() {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!(
                    "cannot load module `{}`: {}",
                    path.display(),
                    std::io::Error::last_os_error()
                ),
            ));
        }
        Ok(handle)
    }

    pub(super) fn symbol(handle: &NativeHandle, name: &str) -> Result<*const c_void> {
        let name = CString::new(name)
            .map_err(|_| Error::new(codes::MODULE_INVALID, "symbol name contains a NUL byte"))?;
        // SAFETY: the handle is live and the name is NUL-terminated.
        let address = unsafe { GetProcAddress(*handle, name.as_ptr().cast()) };
        if address.is_null() {
            return Err(Error::new(
                codes::MODULE_INVALID,
                format!("module does not export `{}`", name.to_string_lossy()),
            ));
        }
        Ok(address.cast_const())
    }

    pub(super) fn close(handle: NativeHandle) {
        // SAFETY: the handle came from `LoadLibraryW` and is released once.
        unsafe {
            FreeLibrary(handle);
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    use stc_got::error::{Error, Result, codes};

    use super::NativeHandle;

    const RTLD_NOW: c_int = 2;
    // The platform default on Linux GNU hosts; passing it explicitly keeps
    // module exports out of the global scope (see the module documentation).
    const RTLD_LOCAL: c_int = 0;

    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> c_int;
        fn dlerror() -> *const c_char;
    }

    fn last_error() -> String {
        // SAFETY: `dlerror` returns a NUL-terminated string or null. The
        // pointer is only read while the calling thread owns the error slot.
        unsafe {
            let pointer = dlerror();
            if pointer.is_null() {
                "unknown dynamic-loader error".into()
            } else {
                CStr::from_ptr(pointer).to_string_lossy().into_owned()
            }
        }
    }

    pub(super) fn open(path: &Path) -> Result<NativeHandle> {
        let name = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            Error::new(
                codes::MODULE_INVALID,
                format!("module path `{}` contains a NUL byte", path.display()),
            )
        })?;
        // SAFETY: clear any stale error, then pass a valid path to the loader.
        unsafe {
            let _ = dlerror();
            let handle = dlopen(name.as_ptr(), RTLD_NOW | RTLD_LOCAL);
            if handle.is_null() {
                return Err(Error::new(
                    codes::MODULE_INVALID,
                    format!("cannot load module `{}`: {}", path.display(), last_error()),
                ));
            }
            Ok(handle)
        }
    }

    pub(super) fn symbol(handle: &NativeHandle, name: &str) -> Result<*const c_void> {
        let name = CString::new(name)
            .map_err(|_| Error::new(codes::MODULE_INVALID, "symbol name contains a NUL byte"))?;
        // SAFETY: the handle is live and the name is NUL-terminated.
        unsafe {
            let address = dlsym(*handle, name.as_ptr());
            if address.is_null() {
                return Err(Error::new(
                    codes::MODULE_INVALID,
                    format!("module does not export `{}`", name.to_string_lossy()),
                ));
            }
            Ok(address.cast_const())
        }
    }

    pub(super) fn close(handle: NativeHandle) {
        // SAFETY: the handle came from `dlopen` and is released once.
        unsafe {
            dlclose(handle);
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use std::path::Path;

    use stc_got::error::{Error, Result, codes};

    use super::NativeHandle;

    pub(super) fn open(_path: &Path) -> Result<NativeHandle> {
        Err(Error::new(
            codes::MODULE_TARGET,
            "dynamic module loading is not implemented for this host",
        ))
    }

    pub(super) fn symbol(_handle: &NativeHandle, _name: &str) -> Result<*const std::ffi::c_void> {
        Err(Error::new(
            codes::MODULE_TARGET,
            "dynamic module loading is not implemented for this host",
        ))
    }

    pub(super) fn close(_handle: NativeHandle) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_module_is_a_module_error() {
        let path = std::env::temp_dir().join("stc-runtime-no-such-module.dll");
        let error = match LoadedModule::load(&path) {
            Ok(_) => panic!("a missing module must not load"),
            Err(error) => error,
        };
        assert_eq!(error.code, codes::MODULE_INVALID);
    }

    #[test]
    fn entry_point_names_match_the_abi() {
        assert_eq!(symbol::ATTACH, "stc_module_attach");
        assert_eq!(symbol::PREPARE, "stc_module_prepare");
        assert_eq!(symbol::CYCLE, "stc_module_cycle");
        assert_eq!(symbol::SHUTDOWN, "stc_module_shutdown");
        assert_eq!(symbol::DETACH, "stc_module_detach");
    }
}
