use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use std::sync::Once;
use std::cell::RefCell;
use std::mem::MaybeUninit;

use miniserde::{json, Serialize};

mod error;
mod utils;
mod render;
mod content;
mod node_view;

use error::{Error, Result};

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

pub fn result_to_cstring<T: ToString>(res: Result<T>) -> CString {
    let inner = match res {
        Ok(inn) => format!("{{ \"ok\": {} }}", inn.to_string()),
        Err(err) => format!("{{ \"err\": \"{}\" }}", err.to_string()),
    };

    CString::new(inner).unwrap()
}

macro_rules! export_fn {
    ($fn_name:ident,String)=> {
        #[no_mangle]
        pub unsafe extern "C" fn $fn_name(input: *const c_char) -> *const c_char {
            let input = CStr::from_ptr(input);
            let in_str = input.to_str().unwrap();
        
            let res = singleton().inner.borrow_mut().$fn_name(in_str);
            let res_str = result_to_cstring(res);

            res_str.into_raw()
        }
    };
    ($fn_name:ident,usize) => {
        #[no_mangle]
        pub unsafe extern "C" fn $fn_name(input: *const c_char) -> usize {
            let input = CStr::from_ptr(input);
            let in_str = input.to_str().unwrap();
        
            match singleton().inner.borrow_mut().$fn_name(in_str)
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
export_fn!(draw, String);
export_fn!(set_folds, ());
