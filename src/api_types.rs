use serde;
use serde::{Deserialize, Serialize};

use std::collections::HashMap;

/* This is the single data structure used for websockets messages (in json)

usage:
    {"type": "POSITION_UPDATE",
        "xy": [1.0, 2.0], "timeMs": 1232435 }

    {"type": "PROJECTILE_CREATED",
        "projectileType": "HIT_SCAN0",
        "origin": { "xy": [1.1, 2.1], "timeMs": 1232435 },
        "vel": [0.707, 0.707] }

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
    ProjectileCreated(ProjectileSnaphot),
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
pub struct ProjectileSnaphot {
    pub projectile_type: ProjectileType,
    pub origin: PositionStamped, // i.e. starts at the end of the gun barrel
    pub vel: Vec2, // treated as a unit vector (direction), vel is given by ProjectileInfo
}

pub type Vec2 = nalgebra::Vector2<f32>;

pub type Health = isize;

#[derive(Copy, Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectileType {
    HitScan0 = 0,
    Projectile0 = 1,
    // ...
}

pub mod projectile_info {
    #[derive(Debug)]
    pub struct ProjectileInfo {
        pub speed: Option<f32>, // if None, this is hitscan (infinite speed)
        pub damage: super::Health,
        /// number of enemies projectile will pass through, None indicates infinite
        /// most projectiles will likely be 1
        pub num_penetrations: Option<isize>,
    }
    static PROJECTILE_INFOS: &'static [ProjectileInfo] = &[
        ProjectileInfo {
            speed: None,
            damage: 1,
            num_penetrations: Some(1),
        },
        ProjectileInfo {
            speed: Some(2.0),
            damage: 10,
            num_penetrations: Some(1),
        },
    ];
    pub fn lookup_projectile_info(
        projectile_type: super::ProjectileType,
    ) -> &'static ProjectileInfo {
        &PROJECTILE_INFOS[projectile_type as usize]
    }
}

///---------------------------///
/// Messages sent to clients: ///
///---------------------------///

#[derive(Serialize, Debug)]
#[serde(tag = "type")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServerUpdate {
    YourPlayerId(PlayerIdMessage),
    PlayerDisconnected(PlayerIdMessage),
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerIdMessage {
    pub player_id: PlayerId,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GameState {
    pub players: HashMap<PlayerId, Player>,
    pub enemies: Vec<Enemy>,
    pub projectiles: Vec<PlayerProjectile>,
}

pub type PlayerId = String;

#[derive(Debug)]
pub enum AuthorizationStatus {
    #[allow(dead_code)]
    Unspecified = 0,
    GoodStanding = 1,
    #[allow(dead_code)]
    FoulPlayDetected = 2, // TODO: control for malicious clients
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Player {
    pub position: PositionStamped,
    pub connection_status: ConnectionStatus,

    #[serde(skip)]
    pub authr_status: AuthorizationStatus,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerProjectile {
    pub player_id: PlayerId,
    pub projectile: ProjectileSnaphot,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectionStatus {
    #[allow(dead_code)]
    Unspecified,
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
    #[allow(dead_code)]
    Unspecified = 0,
    #[allow(dead_code)]
    Alive = 1, // TODO: spawn enemies
    Dead = 2,
}
