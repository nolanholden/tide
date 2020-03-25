mod api_types;
mod config;
use api::bullet_info::lookup_bullet_info;
use api_types as api;

#[macro_use]
extern crate log;

use ncollide2d as nc;
use serde_json;
use ws;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time;

#[derive(Debug)]
struct GameMap {
    max_dimension: usize,
}

fn bullet_ray_scans_enemy(
    bullet: &api::BulletSnaphot,
    enemy: &api::Enemy,
    max_time_of_impact: f32,
) -> bool {
    use crate::nc::bounding_volume::BoundingVolume;
    use crate::nc::query::RayCast;
    use nc::math::Isometry;
    use nc::math::Point;

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

#[derive(Debug)]
struct GameUpdater {
    update_channel_rx: mpsc::Receiver<ChannelUpdate>,
    broadcaster: ws::Sender,
    state: api::GameState,
    map: GameMap,
}

impl GameUpdater {
    fn new(
        update_channel_rx: mpsc::Receiver<ChannelUpdate>,
        broadcaster: ws::Sender,
        map: GameMap,
    ) -> GameUpdater {
        GameUpdater {
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

    fn handle_player_update(
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
    fn progress_bullets(&mut self) -> Result<(), String> {
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

    fn broadcast_state(&mut self) -> Result<(), String> {
        let game_state_json = serde_json::ser::to_string(&self.state).unwrap();
        if let Err(e) = self.broadcaster.send(game_state_json) {
            Err(e.details.to_string())
        } else {
            Ok(())
        }
    }

    fn loop_until_cancelled<F: Fn() -> bool>(&mut self, cancelled: F) -> Result<(), String> {
        info!("game update daemon started.");

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
                .recv_timeout(*config::AWAIT_CLIENT_MSG_TIMEOUT())
            {
                Ok(ChannelUpdate { id, update }) => self.handle_player_update(id, update)?,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("update channel disconnected".to_owned());
                }
            };
        }

        info!("game updater daemon detected cancellation, terminating...");

        Ok(())
    }
    fn get_player(&mut self, id: &api::PlayerId) -> &mut api::Player {
        self.state.players.get_mut(id).unwrap()
    }
    fn handle_bullet_shot(&mut self, id: api::PlayerId, bullet: api::BulletSnaphot) {
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
                    enemy.health -= lookup_bullet_info(bullet.bullet_type).damage;
                    if enemy.health <= 0 {
                        enemy.status = api::EnemyStatus::Dead;
                    }
                }
            }
        }
    }

    fn try_connect_player(&mut self, id: api::PlayerId) -> Result<(), String> {
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
                    },
                );
            }
        }

        Ok(())
    }

    fn disconnect_player(&mut self, id: api::PlayerId) {
        if cfg!(feature = "ip-address-player-ids") {
            self.get_player(&id).connection_status = api::ConnectionStatus::Disconnected;
        } else {
            self.state.players.remove(&id);
        }
    }
}

struct ChannelUpdate {
    id: api::PlayerId,
    update: api::ClientUpdate,
}

// websockets game server
struct GameServer<'a> {
    out: ws::Sender,
    update_channel: mpsc::Sender<ChannelUpdate>,
    player_id_resolver: &'a PlayerIdResolver,
    player_id: Option<api::PlayerId>,
}

impl<'a> GameServer<'_> {
    fn new(
        out: ws::Sender,
        update_channel: mpsc::Sender<ChannelUpdate>,
        player_id_resolver: &'a PlayerIdResolver,
    ) -> GameServer {
        GameServer {
            out: out,
            update_channel: update_channel,
            player_id_resolver: player_id_resolver,
            player_id: None,
        }
    }
}

impl GameServer<'_> {
    fn send_out(&mut self, data: String) -> ws::Result<()> {
        debug!("server sending: [{}]", data);
        self.out.send(data)
    }
}

impl ws::Handler for GameServer<'_> {
    fn on_open(&mut self, shake: ws::Handshake) -> ws::Result<()> {
        let id: api::PlayerId = self.player_id_resolver.resolve_id(&shake)?;
        info!("Connection with [{}] now open", &id);
        self.player_id = Some(id.clone());
        self.update_channel
            .send(ChannelUpdate {
                id: id,
                update: api::ClientUpdate::PlayerConnected(()),
            })
            .unwrap();
        let message = format!(
            r#"{{"type": "YOUR_PLAYER_ID", "player_id": "{}"}}"#,
            self.player_id.as_ref().unwrap()
        );
        self.out.send(message)
    }

    fn on_message(&mut self, msg: ws::Message) -> ws::Result<()> {
        debug!("server got: [{}]", msg);
        let id = self.player_id.as_ref().unwrap();
        match msg {
            ws::Message::Text(json) => match serde_json::from_str(&json) {
                Ok(update) => {
                    self.update_channel
                        .send(ChannelUpdate {
                            id: id.clone(),
                            update,
                        })
                        .unwrap();
                }
                Err(error) => {
                    return self.send_out(format!(
                        "unrecognized message: [{}], error: [{:?}]",
                        json, error
                    ));
                }
            },
            _ => {
                return Err(ws::Error::new(
                    ws::ErrorKind::Internal,
                    "only accepting text strings",
                ));
            }
        }
        Ok(())
    }

    fn on_close(&mut self, code: ws::CloseCode, reason: &str) {
        info!("WebSocket closing for [{:?}], reason: [{}]", code, reason);
        self.update_channel
            .send(ChannelUpdate {
                id: self.player_id.as_ref().unwrap().clone(),
                update: api::ClientUpdate::PlayerDisconnected(()),
            })
            .unwrap();
        let message = format!(
            r#"{{"type": "PLAYER_DISCONNECTED", "player_id": "{}"}}"#,
            self.player_id.as_ref().unwrap()
        );
        self.out.broadcast(message).unwrap();
    }
}

/// usage:
///     let (cancelled, cancel) = make_atomic_canceller();
///     thread::spawn(move || {
///       while !cancelled() { /* ... */ }
///       println!("cancel() called");
///     });
///     // ...
///     cancel();
fn make_atomic_canceller() -> (impl Fn() -> bool, impl Fn() -> ()) {
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_receiver = cancel_flag.clone();
    let update_daemon_is_cancelled = move || cancel_flag_receiver.load(Ordering::Relaxed);
    let update_daemon_set_cancelled = move || cancel_flag.store(true, Ordering::Relaxed);
    (update_daemon_is_cancelled, update_daemon_set_cancelled)
}

#[cfg(feature = "ip-address-player-ids")]
struct PlayerIdResolver;
#[cfg(not(feature = "ip-address-player-ids"))]
struct PlayerIdResolver {
    next_player_id: Mutex<usize>,
}

#[cfg(feature = "ip-address-player-ids")]
impl PlayerIdResolver {
    fn new() {
        PlayerIdResolver {}
    }
    fn resolve_id(&mut self, handshake: &ws::Handshake) -> ws::Result<api::PlayerId> {
        Ok(handshake.remote_addr()?.unwrap())
    }
}
#[cfg(not(feature = "ip-address-player-ids"))]
impl PlayerIdResolver {
    fn new() -> PlayerIdResolver {
        PlayerIdResolver {
            next_player_id: Mutex::new(1),
        }
    }
    fn resolve_id(&self, _handshake: &ws::Handshake) -> ws::Result<api::PlayerId> {
        let current_id: Option<_>;
        {
            let mut id = self.next_player_id.lock().unwrap();
            current_id = Some(*id);
            *id += 1;
        }
        return Ok(current_id.unwrap().to_string());
    }
}

struct ServerFactory<'a> {
    update_channel: &'a mpsc::Sender<ChannelUpdate>,
    player_id_resolver: &'a PlayerIdResolver,
}
impl<'a> ws::Factory for ServerFactory<'a> {
    type Handler = GameServer<'a>;
    fn connection_made(&mut self, sender: ws::Sender) -> GameServer<'a> {
        GameServer::new(sender, self.update_channel.clone(), self.player_id_resolver)
    }
}

fn start_game_update_daemon(
    mut game: GameUpdater,
) -> Result<impl FnOnce() -> (), Box<dyn std::error::Error>> {
    let (update_daemon_is_cancelled, cancel_update_daemon) = make_atomic_canceller();
    let update_daemon = thread::Builder::new()
        .name("GameUpdateDaemon".to_owned())
        .spawn(move || game.loop_until_cancelled(update_daemon_is_cancelled))?;
    let terminate_daemon = move || {
        info!("requesting game update daemon thread to stop...");
        cancel_update_daemon();
        match update_daemon.join().unwrap() {
            Err(details) => error!("game update daemon failed, details: [{}]", details),
            Ok(_) => info!("game update daemon thread closed without error."),
        };
    };
    Ok(terminate_daemon)
}

fn parse_args() -> (String,) {
    let args: Vec<String> = std::env::args().collect();
    let socket_address = args[1].clone();
    (socket_address,)
}

fn set_up_websockets_servers<'a>(
    update_channel_tx: &'a mpsc::Sender<ChannelUpdate>,
    player_id_resolver: &'a PlayerIdResolver,
) -> (ws::WebSocket<ServerFactory<'a>>, ws::Sender) {
    let server_factory = ServerFactory {
        update_channel: &update_channel_tx,
        player_id_resolver: &player_id_resolver,
    };
    let socket = ws::Builder::new().build(server_factory).unwrap();
    let broadcaster = socket.broadcaster();
    (socket, broadcaster)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    config::init();

    let (socket_address,) = parse_args();
    info!("starting server, using address [{}]...", socket_address);

    // Create communication channel between websockets servers and
    // the update daemon.
    let (update_channel_tx, update_channel_rx) = mpsc::channel();

    // Configure websockets server(s).
    let mut resolver = PlayerIdResolver::new();
    let (socket, broadcaster) = set_up_websockets_servers(&update_channel_tx, &mut resolver);

    // Start update daemon.
    let map = GameMap { max_dimension: 100 };
    let game = GameUpdater::new(update_channel_rx, broadcaster, map);
    let terminate_update_daemon = start_game_update_daemon(game)?;

    // Start listening (on event loop).
    if let Err(error) = socket.listen(socket_address) {
        error!("failed to create websocket due to {:?}", error)
    }

    // If the websockets server quit for some reason, terminate the update daemon.
    terminate_update_daemon();

    info!("server closed.");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
