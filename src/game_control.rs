use crate::api_types;
use crate::config;
use crate::geography::GameMap;
use crate::intercomm::ChannelUpdate;
use crate::utils;
use api::bullet_info::lookup_bullet_info;
use api_types as api;

use ncollide2d as nc;

use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time;

pub fn start_game_controller_thread(
    mut game: GameController,
) -> Result<impl FnOnce() -> (), Box<dyn std::error::Error>> {
    let (game_controller_is_cancelled, cancel_game_controller) = utils::make_atomic_canceller();
    let game_controller = thread::Builder::new()
        .name("GameController".to_owned())
        .spawn(move || game.loop_until_cancelled(game_controller_is_cancelled))?;
    let terminate_game_controller = move || {
        info!("requesting game controller thread to stop...");
        cancel_game_controller();
        match game_controller.join().unwrap() {
            Err(details) => error!("game controller failed, details: [{}]", details),
            Ok(_) => info!("game controller thread closed without error."),
        };
    };
    Ok(terminate_game_controller)
}

#[derive(Debug)]
pub struct GameController {
    update_channel_rx: mpsc::Receiver<ChannelUpdate>,
    broadcaster: ws::Sender,
    state: api::GameState,
    map: GameMap,
}

impl GameController {
    pub fn new(
        update_channel_rx: mpsc::Receiver<ChannelUpdate>,
        broadcaster: ws::Sender,
        map: GameMap,
    ) -> GameController {
        GameController {
            update_channel_rx,
            broadcaster: broadcaster,
            map: map,
            state: api::GameState {
                players: HashMap::new(),
                enemies: vec![],
                bullets: vec![],
            },
        }
    }

    pub fn handle_player_update(
        &mut self,
        id: api::PlayerId,
        update: api::ClientUpdate,
    ) -> Result<(), String> {
        match update {
            api::ClientUpdate::PlayerConnected(()) => self.try_connect_player(id)?,
            api::ClientUpdate::PlayerDisconnected(()) => self.disconnect_player(id),
            api::ClientUpdate::PositionUpdate(position) => self.get_player(&id).position = position,
            api::ClientUpdate::BulletShot(shot) => self.handle_bullet_shot(id, shot),
        };
        debug!(" --> state: {:?}", self.state.players);
        Ok(())
    }

    pub fn progress_bullets(&mut self) -> Result<(), String> {
        for player_bullet in self.state.bullets.iter_mut() {
            let bullet: &mut api::BulletSnaphot = &mut player_bullet.bullet;
            let now_ms = time::SystemTime::now()
                .duration_since(time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            let delta_ms = now_ms - bullet.origin.time_ms;
            let delta_secs = delta_ms as f32 / 1000.0;
            bullet.origin.time_ms = now_ms;
            // TODO: should we check if bullet would scan a target during the jump?
            //    or just use ncollide2d? (see bullet_ray_scans_enemy())
            bullet.origin.xy += bullet.velocity * (delta_secs);
        }
        Ok(())
    }

    pub fn broadcast_state(&mut self) -> Result<(), String> {
        let game_state_json = serde_json::ser::to_string(&self.state).unwrap();
        if let Err(e) = self.broadcaster.send(game_state_json) {
            Err(e.details.to_string())
        } else {
            Ok(())
        }
    }

    pub fn loop_until_cancelled<F: Fn() -> bool>(&mut self, cancelled: F) -> Result<(), String> {
        info!("game controller started.");

        // TODO: can we give branch prediction compiler hint here? (in rust)
        while !cancelled() {
            self.progress_bullets()?;
            self.broadcast_state()?;
            // We'll wait as long as the full timeout for any client messages.
            // Thus, the timeout is the worst-case granularity of internal updates.
            // If we get client message *more* frequently, we expect to see
            // higher granularity to compensate.
            match self
                .update_channel_rx
                .recv_timeout(*config::AWAIT_CLIENT_MSG_TIMEOUT_MS())
            {
                Ok(ChannelUpdate { id, update }) => self.handle_player_update(id, update)?,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("update channel disconnected".to_owned());
                }
            };
        }

        info!("game updater game_controller detected cancellation, terminating...");

        Ok(())
    }
    pub fn get_player(&mut self, id: &api::PlayerId) -> &mut api::Player {
        self.state.players.get_mut(id).unwrap()
    }
    pub fn handle_bullet_shot(&mut self, id: api::PlayerId, bullet: api::BulletSnaphot) {
        let bullet_info = lookup_bullet_info(bullet.bullet_type);
        match bullet_info.speed {
            Some(speed) => {
                // TODO: how do we handle possibility of crossing
                // over enemies which should have been hit (due to
                // insufficient granularity)
                self.state.bullets.push(api::PlayerBullet {
                    player_id: id,
                    bullet: api::BulletSnaphot {
                        bullet_type: bullet.bullet_type,
                        origin: bullet.origin,
                        velocity: bullet.velocity.normalize().scale(speed),
                    },
                });
            }
            None => {
                let max_time_of_impact = self.map.max_dimension as f32;
                let (mut in_bullet_path, _): (Vec<&mut api::Enemy>, Vec<&mut api::Enemy>) =
                    self.state.enemies.iter_mut().partition(|ref enemy| {
                        bullet_ray_scans_enemy(&bullet, enemy, max_time_of_impact)
                    });
                for enemy in in_bullet_path.iter_mut() {
                    enemy.health -= bullet_info.damage;
                    if enemy.health <= 0 {
                        enemy.status = api::EnemyStatus::Dead;
                    }
                }
            }
        }
    }

    pub fn try_connect_player(&mut self, id: api::PlayerId) -> Result<(), String> {
        match self.state.players.get_mut(&id) {
            // if we had this player before, reconnect them
            Some(player) => match player.connection_status {
                api::ConnectionStatus::Disconnected => {
                    player.connection_status = api::ConnectionStatus::Connected
                }
                _ => {
                    return Err(format!(
                        "cannot have duplicate client addresses, got [{}]",
                        &id
                    ));
                }
            },
            // if never seen this player, add them
            None => {
                self.state.players.insert(
                    id,
                    api::Player {
                        position: api::PositionStamped {
                            xy: api::Vec2::new(0.0, 0.0),
                            time_ms: 0,
                        },
                        connection_status: api::ConnectionStatus::Connected,
                        authr_status: api::AuthorizationState::GoodStanding,
                    },
                );
            }
        }

        Ok(())
    }

    pub fn disconnect_player(&mut self, id: api::PlayerId) {
        if cfg!(feature = "ip-address-player-ids") {
            self.get_player(&id).connection_status = api::ConnectionStatus::Disconnected;
        } else {
            self.state.players.remove(&id);
        }
    }
}

pub fn bullet_ray_scans_enemy(
    bullet: &api::BulletSnaphot,
    enemy: &api::Enemy,
    max_time_of_impact: f32,
) -> bool {
    use nc::bounding_volume::BoundingVolume;
    use nc::math::Isometry;
    use nc::math::Point;
    use nc::query::RayCast;

    let origin = Point::from(enemy.position.xy);
    let enemy_hit_boundary = nc::bounding_volume::aabb::AABB::new(origin, origin).loosened(0.5);

    let bullet_ray =
        nc::query::Ray::new(Point::from(bullet.origin.xy), bullet.velocity.normalize());

    // TODO: this currently shoots through walls, fix that
    // TODO: implement max bullet penetration
    enemy_hit_boundary.intersects_ray(
        &Isometry::identity(),
        &bullet_ray,
        max_time_of_impact, // e.g. scalar scaling of vector
    )
}
