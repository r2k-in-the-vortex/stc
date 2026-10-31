//! Embedding host run surface (`stc_host_run`) integration tests.
//!
//! These use one compiled no-op module and drive the process-global embedding
//! host, so all interactions live in one test.

mod common;

use std::ffi::CString;

use stc_runtime::{
    StcHostOptions, load_mode, stc_host_configure_io, stc_host_configure_retain,
    stc_host_load_module, stc_host_run, stc_host_shutdown,
};

#[test]
fn embedding_host_run_honors_options_and_stop_conditions() {
    let artifacts = common::artifacts_native(common::SIMPLE, "embedding-run");
    let Some(fixture) =
        common::Fixture::build("embedding-run", artifacts, common::Behavior::default())
    else {
        return;
    };
    let module = CString::new(fixture.module_path.to_string_lossy().as_bytes())
        .expect("module path is NUL-free");

    // SAFETY: the module path is a live NUL-terminated string.
    let loaded = unsafe { stc_host_load_module(module.as_ptr(), load_mode::COLD) };
    assert_eq!(loaded, 0, "embedding host cold install");

    // Cycle count from the options struct.
    let mut options = StcHostOptions {
        cycles: 2,
        ..StcHostOptions::default()
    };
    // SAFETY: the options struct contains only null pointers here.
    assert_eq!(unsafe { stc_host_run(&options) }, 0);

    // A pre-set stop flag interrupts before the first scan.
    let stop = 1_i32;
    options.cycles = 0;
    options.stop = &stop;
    // SAFETY: the stop flag lives for the duration of the call.
    assert_eq!(unsafe { stc_host_run(&options) }, 130);
    options.stop = std::ptr::null();

    // The control endpoint is reserved for phase 8.
    let endpoint = CString::new("pipe://stc").expect("endpoint is NUL-free");
    options.control_endpoint = endpoint.as_ptr();
    // SAFETY: the endpoint is a live NUL-terminated string.
    assert_eq!(unsafe { stc_host_run(&options) }, 2);
    options.control_endpoint = std::ptr::null();

    // A module path loads cold and runs; the host stays loaded afterwards.
    options.module_path = module.as_ptr();
    options.cycles = 1;
    // SAFETY: the module path is a live NUL-terminated string.
    assert_eq!(unsafe { stc_host_run(&options) }, 0);

    // Struct/ABI validation.
    options.struct_size = 4;
    // SAFETY: the options struct is still readable.
    assert_eq!(unsafe { stc_host_run(&options) }, 2);
    options.struct_size = 0;
    options.abi_version = 99;
    // SAFETY: the options struct is still readable.
    assert_eq!(unsafe { stc_host_run(&options) }, 2);
    // Backend and retained-state configuration exports validate their input.
    // SAFETY: the file pointers are null and no host generation is running.
    assert_eq!(unsafe { stc_host_configure_io(1, std::ptr::null()) }, 0);
    // Numeric backend IDs follow the CLI spelling: 0 null, 1 memory, 2 file,
    // 3 ethercat.
    assert_eq!(unsafe { stc_host_configure_io(3, std::ptr::null()) }, 0);
    assert_eq!(unsafe { stc_host_configure_io(9, std::ptr::null()) }, 2);
    assert_eq!(
        unsafe { stc_host_configure_retain(std::ptr::null(), 0, 0) },
        0
    );
    assert_eq!(
        unsafe { stc_host_configure_retain(std::ptr::null(), 9, 0) },
        2
    );

    // SAFETY: no other test shares this process's embedding host.
    assert_eq!(stc_host_shutdown(), 0);
}
