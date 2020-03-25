use crate::api_types as api;

use ws;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::SystemTime;

pub fn unix_time() -> Duration {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
}

/// 2020, March 25, 00:00:00 GMT
///
pub const CUSTOM_EPOCH_OFFSET: Duration = Duration::from_secs(1_585_094_400);
lazy_static! {
    pub static ref CUSTOM_EPOCH: SystemTime = SystemTime::UNIX_EPOCH + CUSTOM_EPOCH_OFFSET;
}
pub fn custom_time() -> Duration {
    SystemTime::now().duration_since(*CUSTOM_EPOCH).unwrap()
}

/// 2^64 nanoseconds gives us 584.554531 years, or ~2604 CE from 2020 Mar 25
#[allow(non_camel_case_types)]
type custom_nanos_t = u64;
pub fn custom_time_ns() -> custom_nanos_t {
    custom_time().as_nanos() as custom_nanos_t
}

/// usage:
///     let (cancelled, cancel) = make_atomic_canceller();
///     thread::spawn(move || {
///       while !cancelled() { /* ... */ }
///       println!("cancel() called");
///     });
///     // ...
///     cancel();
///
/// Note: make sure Ordering suits your needs (below)
pub fn make_atomic_canceller() -> (impl Fn() -> bool, impl Fn() -> ()) {
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_receiver = cancel_flag.clone();
    let cancelled = move || cancel_flag_receiver.load(Ordering::Relaxed);
    let cancel = move || cancel_flag.store(true, Ordering::Relaxed);
    (cancelled, cancel)
}

#[cfg(feature = "ip-address-player-ids")]
pub struct PlayerIdResolver;
#[cfg(not(feature = "ip-address-player-ids"))]
pub struct PlayerIdResolver {
    next_player_id: Mutex<usize>,
}

#[cfg(feature = "ip-address-player-ids")]
impl PlayerIdResolver {
    pub fn new() {
        PlayerIdResolver {}
    }
    pub fn resolve_id(&mut self, handshake: &ws::Handshake) -> ws::Result<api::PlayerId> {
        Ok(handshake.remote_addr()?.unwrap())
    }
}
#[cfg(not(feature = "ip-address-player-ids"))]
impl PlayerIdResolver {
    pub fn new() -> PlayerIdResolver {
        PlayerIdResolver {
            next_player_id: Mutex::new(1),
        }
    }
    pub fn resolve_id(&self, _handshake: &ws::Handshake) -> ws::Result<api::PlayerId> {
        let current_id: Option<_>;
        {
            let mut id = self.next_player_id.lock().unwrap();
            current_id = Some(*id);
            *id += 1;
        }
        return Ok(current_id.unwrap().to_string());
    }
}
