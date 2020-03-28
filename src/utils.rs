use crate::api_types as api;

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

pub trait SerialIdGenerator {
    const MASK: api::EntityId;

    fn get_next_id(&self) -> api::EntityId {
        let mut data = self.get_next_id_mutex().lock().unwrap();
        let id = *data;
        if (id & Self::MASK) != 0 {
            panic!("overflow of ids, id=[{}], MASK=[{}]", id, Self::MASK);
        }
        *data += 1;
        return id | Self::MASK;
    }
    fn new(first_id: api::EntityId) -> Self;
    fn get_next_id_mutex(&self) -> &Mutex<api::EntityId>;
}

/// u32 with first two bits: 00
pub struct PlayerIdGenerator {
    next_id: Mutex<api::EntityId>,
}
/// u32 with first two bits: 01
pub struct EnemyIdGenerator {
    next_id: Mutex<api::EntityId>,
}
/// u32 with first two bits: 1x (x is "don't care")
///
/// At 1,000 projectiles/seconds, this gives us:
/// 3^31 / (1,000 projectiles/second) ~= 20,000 years of ids
pub struct ProjectileIdGenerator {
    next_id: Mutex<api::EntityId>,
}

impl SerialIdGenerator for PlayerIdGenerator {
    const MASK: api::EntityId = 0b00 << (32 - 2);
    fn get_next_id_mutex(&self) -> &Mutex<api::EntityId> {
        &self.next_id
    }
    fn new(first_id: api::EntityId) -> PlayerIdGenerator {
        PlayerIdGenerator {
            next_id: Mutex::new(first_id),
        }
    }
}
impl SerialIdGenerator for EnemyIdGenerator {
    const MASK: api::EntityId = 0b01 << (32 - 2);
    fn get_next_id_mutex(&self) -> &Mutex<api::EntityId> {
        &self.next_id
    }
    fn new(first_id: api::EntityId) -> EnemyIdGenerator {
        EnemyIdGenerator {
            next_id: Mutex::new(first_id),
        }
    }
}
impl SerialIdGenerator for ProjectileIdGenerator {
    const MASK: api::EntityId = 0b10 << (32 - 2);
    fn get_next_id_mutex(&self) -> &Mutex<api::EntityId> {
        &self.next_id
    }
    fn new(first_id: api::EntityId) -> ProjectileIdGenerator {
        ProjectileIdGenerator {
            next_id: Mutex::new(first_id),
        }
    }
}
