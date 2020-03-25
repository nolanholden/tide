#![allow(non_snake_case)]

use std::env;
use ::std::time::Duration;

/// must be called *synchronously* before accessing any environment variables
pub fn init() {
    env_logger::init();
    init_env_vars();
}

macro_rules! define_env_var {
    ($name:ident, $type:ty, $default_value:expr) => {
        pub mod $name {
            use ::std::time::Duration;
            pub static mut $name: $type = $default_value;
        }
        pub fn $name() -> &'static $type {
            unsafe { &$name::$name }
        }
    };
}

macro_rules! init_env_var {
    ($name:ident, $parse_expr:expr) => {{
        match env::var(stringify!($name)) {
            Ok(value) => {
                $name::$name = ($parse_expr)(value);
            }
            Err(_) => {
                warn!(
                    "using default for {}: [{:?}]",
                    stringify!($name),
                    &$name::$name
                );
            }
        }
    }};
}

define_env_var!(
    AWAIT_CLIENT_MSG_TIMEOUT,
    Duration,
    Duration::from_millis(50)
);

pub fn init_env_vars() {
    unsafe {
        init_env_var!(AWAIT_CLIENT_MSG_TIMEOUT, |s: String| Duration::from_millis(
            s.parse().unwrap()
        ));
    }
}
