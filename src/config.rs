#![allow(non_snake_case)]

use std::env;
use std::time::Duration;

/// must be called *synchronously* before accessing any environment variables
/// (access is not synchronized)
pub fn init() {
    env_logger::init();
    init_env_vars();
}

macro_rules! define_env_var {
    ($name:ident, $type:ty, $default_value:expr) => {
        /// define the value in its own module (for name deconfliction)
        mod $name {
            #[allow(unused_imports)]
            use ::std::time::Duration;
            pub static mut $name: $type = $default_value;
        }

        /// gets the static value
        pub fn $name() -> $type {
            unsafe { $name::$name }
        }
    };
}

macro_rules! init_env_var_impl {
    ($name:ident, $parse_closure:expr) => {{
        match env::var(stringify!($name)) {
            Ok(value) => {
                $name::$name = Duration::from_millis(value.parse().unwrap());
                info!("{}=[{:?}]", stringify!($name), &$name::$name);
            }
            Err(_) => {
                warn!("{} using default [{:?}]", stringify!($name), &$name::$name);
            }
        }
    }};
}

macro_rules! init_env_var {
    ($name:ident) => {{
        match env::var(stringify!($name)) {
            Ok(value) => {
                $name::$name = value.parse().unwrap();
                info!("{}=[{:?}]", stringify!($name), &$name::$name);
            }
            Err(_) => {
                warn!("{} using default [{:?}]", stringify!($name), &$name::$name);
            }
        }
    }};
}

define_env_var!(
    AWAIT_CLIENT_MSG_TIMEOUT_MS,
    Duration,
    Duration::from_millis(50)
);
define_env_var!(WEBSOCKETS_PINGPONG_INTERVAL_MS, u64, 4_500);

pub fn init_env_vars() {
    unsafe {
        init_env_var_impl!(AWAIT_CLIENT_MSG_TIMEOUT_MS, |s: String| {
            Duration::from_millis(s.parse().unwrap())
        });
        init_env_var!(WEBSOCKETS_PINGPONG_INTERVAL_MS);
    }
}
