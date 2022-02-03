use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use std::sync::Once;
use std::cell::RefCell;
use std::mem::MaybeUninit;

mod error;
mod utils;
mod render;
mod node_view;

struct SingletonReader {
    inner: RefCell<render::Render>,
}

fn singleton() -> &'static SingletonReader {
    // Create an uninitialized static
    static mut SINGLETON: MaybeUninit<SingletonReader> = MaybeUninit::uninit();
    static ONCE: Once = Once::new();

    unsafe {
        ONCE.call_once(|| {
            // Make it
            let singleton = SingletonReader {
                inner: RefCell::new(render::Render::new()),
            };
            // Store it to the static var, i.e. initialize it
            SINGLETON.write(singleton);
        });

        // Now we give out a shared reference to the data, which is safe to use
        // concurrently.
        SINGLETON.assume_init_ref()
    }
}

macro_rules! export_fn {
    ($fn_name:ident,String)=> {
        #[no_mangle]
        pub unsafe extern "C" fn $fn_name(input: *const c_char) -> *const c_char {
            let input = CStr::from_ptr(input);
            let in_str = input.to_str().unwrap();
        
            let res_str = singleton().inner.borrow_mut().$fn_name(in_str).unwrap();
            let res_str = CString::new(res_str).unwrap();

            res_str.into_raw()
        }
    };
    ($fn_name:ident,usize) => {
        #[no_mangle]
        pub unsafe extern "C" fn $fn_name(input: *const c_char) -> usize {
            let input = CStr::from_ptr(input);
            let in_str = input.to_str().unwrap();
        
            singleton().inner.borrow_mut().$fn_name(in_str).unwrap()
        }
    };
    ($fn_name:ident,()) => {
        #[no_mangle]
        pub unsafe extern "C" fn $fn_name(input: *const c_char) {
            let input = CStr::from_ptr(input);
            let in_str = input.to_str().unwrap();
        
            singleton().inner.borrow_mut().$fn_name(in_str).unwrap();
        }
    }
}

export_fn!(update_content, String);
export_fn!(update_metadata, ());
export_fn!(clear_all, ());
export_fn!(draw, usize);
export_fn!(set_folds, ());
