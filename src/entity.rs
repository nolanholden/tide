pub struct EntityId(u64);

pub const MAX_SAFE_INT_BITS: u64 = 53;

pub const MASK_PLAYER: u64 = 1 << (MAX_SAFE_INT_BITS - 1);
pub const MASK_ENEMY: u64 = 1 << (MAX_SAFE_INT_BITS - 2);
pub const MASK_PROJECTILE: u64 = 1 << (MAX_SAFE_INT_BITS - 3);
impl EntityId {
    pub fn is_player(id: EntityId) {
        id & MASK_PLAYER
    }
}
