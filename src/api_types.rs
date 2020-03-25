use serde;
use serde::{Deserialize, Serialize};

use std::collections::HashMap;

/* This is the single data structure used for websockets messages (in json)

usage:
    {"type": "POSITION_UPDATE",
        "xy": [1.0, 2.0], "timeMs": 1232435 }

    {"type": "BULLET_SHOT",
        "bulletType": "HIT_SCAN0",
        "origin": { "xy": [1.1, 2.1], "timeMs": 1232435 },
        "velocity": [0.707, 0.707] }

    // ...

messages you'll receive:

- your assigned id: (sent immediately upon connection)
    {"type": "YOUR_PLAYER_ID", "player_id": "1"}

- game state: (sent periodically)
    {"type": "GAME_STATE", ... }

- player disconnected: (immediately upon any player's disconnection)
    {"type": "PLAYER_DISCONNECTED", "player_id": "2"}

*/
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClientUpdate {
    PositionUpdate(PositionStamped),
    BulletShot(BulletSnaphot),
    /// manual (server side) messages; client should not have access to these
    #[serde(skip)]
    PlayerConnected(()),
    #[serde(skip)]
    PlayerDisconnected(()),
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PositionStamped {
    pub xy: Vec2,
    pub time_ms: u64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BulletSnaphot {
    pub bullet_type: BulletType,
    pub origin: PositionStamped, // i.e. starts at the end of the gun barrel
    pub velocity: Vec2, // treated as a unit vector (direction), velocity is given by BulletInfo
}

pub type Vec2 = nalgebra::Vector2<f32>;

pub type Health = isize;

#[derive(Copy, Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BulletType {
    HitScan0 = 0,
    Projectile0 = 1,
    // ...
}

pub mod bullet_info {
    #[derive(Debug)]
    pub struct BulletInfo {
        pub speed: Option<f32>, // if None, this is hitscan (infinite speed)
        pub damage: super::Health,
        /// number of enemies bullet will pass through, None indicates infinite
        /// most bullets will likely be 1
        pub num_penetrations: Option<isize>,
    }
    static BULLET_INFOS: &'static [BulletInfo] = &[
        BulletInfo {
            speed: None,
            damage: 1,
            num_penetrations: Some(1),
        },
        BulletInfo {
            speed: Some(2.0),
            damage: 10,
            num_penetrations: Some(1),
        },
    ];
    pub fn lookup_bullet_info(bullet_type: super::BulletType) -> &'static BulletInfo {
        &BULLET_INFOS[bullet_type as usize]
    }
}

///---------------------------///
/// Messages sent to clients: ///
///---------------------------///

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GameState {
    pub players: HashMap<PlayerId, Player>,
    pub enemies: Vec<Enemy>,
    pub bullets: Vec<PlayerBullet>,
}

pub type PlayerId = String;

#[derive(Debug, Serialize)]
pub struct Player {
    pub position: PositionStamped,
    pub connection_status: ConnectionStatus,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerBullet {
    pub player_id: PlayerId,
    pub bullet: BulletSnaphot,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectionStatus {
    _Unspecified,
    Connected,
    Disconnected,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Enemy {
    pub enemy_id: isize,
    pub position: PositionStamped,
    pub health: Health,
    pub status: EnemyStatus,
}
#[derive(Serialize, Debug)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EnemyStatus {
    _Unspecified = 0,
    _Alive = 1,
    Dead = 2,
}
