mod api_types;
use api::bullet_info::lookup_bullet_info;
use api_types as api;

extern crate approx;
extern crate env_logger;
extern crate nalgebra as na;
extern crate ncollide2d as nc;
extern crate ws;

use serde_json;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time;
use std::time::Duration;

const AWAIT_CLIENT_MSG_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Debug)]
struct GameMap {
    max_dimension: usize,
}

#[derive(Debug)]
struct GameUpdater {
    state: api::GameState,
    map: GameMap,
    update_channel_rx: mpsc::Receiver<ChannelUpdate>,
    broadcaster: ws::Sender,
}

fn bullet_ray_scans_enemy(
    bullet: &api::BulletSnaphot,
    enemy: &api::Enemy,
    max_time_of_impact: f32,
) -> bool {
    use crate::nc::bounding_volume::BoundingVolume;
    use crate::nc::query::RayCast;
    use na::geometry::Point;
    use nc::math::Isometry;

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

impl GameUpdater {
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
        println!(" --> state: {:?}", self.state.players);
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
        println!("game update daemon started.");

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
                .recv_timeout(AWAIT_CLIENT_MSG_TIMEOUT)
            {
                Ok(ChannelUpdate { id, update }) => self.handle_player_update(id, update)?,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("update channel disconnected".to_owned());
                }
            };
        }

        println!("game updater daemon detected cancellation, terminating...");

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
struct GameServer<'a, F: FnOnce(&ws::Handshake) -> ws::Result<api::PlayerId>> {
    out: ws::Sender,
    update_channel: mpsc::Sender<ChannelUpdate>,
    get_player_id: &'a F,
    player_id: Option<api::PlayerId>,
}

impl<'a, F: FnOnce(&ws::Handshake) -> ws::Result<api::PlayerId>> GameServer<'_, F> {
    fn new(
        out: ws::Sender,
        update_channel: mpsc::Sender<ChannelUpdate>,
        get_player_id: &'a F,
    ) -> GameServer<F> {
        GameServer {
            out: out,
            update_channel: update_channel,
            get_player_id: get_player_id,
            player_id: None,
        }
    }
}

impl<F: FnOnce(&ws::Handshake) -> ws::Result<api::PlayerId>> GameServer<'_, F> {
    fn send_out(&mut self, data: String) -> ws::Result<()> {
        println!("server sending: [{}]", data);
        self.out.send(data)
    }
}

impl<F> ws::Handler for GameServer<'_, F>
where
    F: Fn(&ws::Handshake) -> ws::Result<api::PlayerId>,
{
    fn on_open(&mut self, shake: ws::Handshake) -> ws::Result<()> {
        let id: api::PlayerId = (self.get_player_id)(&shake)?;
        println!("Connection with [{}] now open", &id);
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
        println!("server got: [{}]", msg);
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
        println!("WebSocket closing for [{:?}], reason: [{}]", code, reason);
        println!("Shutting down server after first connection closes.");
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
fn resolve_player_id(handshake: &ws::Handshake) -> ws::Result<PlayerId> {
    let next_player_id = Mutex::new(1);
    if cfg!(feature = "ip-address-player-ids") {
        return Ok(handshake.remote_addr()?.unwrap());
    } else {
        let current_id: Option<_>;
        {
            let mut id = next_player_id.lock().unwrap();
            current_id = Some(*id);
            *id += 1;
        }
        return Ok(current_id.unwrap().to_string());
    }
}

#[cfg(feature = "ip-address-player-ids")]
fn make_player_id_resolver() -> impl Fn(&ws::Handshake) -> ws::Result<api::PlayerId> {
    |handshake| Ok(handshake.remote_addr()?.unwrap())
}
#[cfg(not(feature = "ip-address-player-ids"))]
fn make_player_id_resolver() -> impl Fn(&ws::Handshake) -> ws::Result<api::PlayerId> {
    let next_player_id = Mutex::new(1);
    return move |_handshake| {
        let current_id: Option<_>;
        {
            let mut id = next_player_id.lock().unwrap();
            current_id = Some(*id);
            *id += 1;
        }
        return Ok(current_id.unwrap().to_string());
    };
}

fn make_server_factory<'a, T>(
    update_channel: &'a mpsc::Sender<ChannelUpdate>,
    resolve_player_id: &'a T,
) -> impl FnMut(ws::Sender) -> GameServer<'a, T>
where
    T: FnOnce(&ws::Handshake) -> ws::Result<api::PlayerId>,
{
    move |sender| GameServer::new(sender, update_channel.clone(), resolve_player_id)
}

fn start_game_update_daemon(
    mut game: GameUpdater,
) -> Result<impl FnOnce() -> (), Box<dyn std::error::Error>> {
    let (update_daemon_is_cancelled, cancel_update_daemon) = make_atomic_canceller();
    let update_daemon = thread::Builder::new()
        .name("GameUpdateDaemon".to_owned())
        .spawn(move || game.loop_until_cancelled(update_daemon_is_cancelled))?;
    let terminate_daemon = move || {
        println!("requesting game update daemon thread to stop...");
        cancel_update_daemon();
        match update_daemon.join().unwrap() {
            Err(details) => println!("game update daemon failed, details: [{}]", details),
            Ok(_) => println!("game update daemon thread closed without error."),
        };
    };
    Ok(terminate_daemon)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Open interthread communication channel (between websockets servers and
    // the update daemon).
    let (update_channel_tx, update_channel_rx) = mpsc::channel();
    let args: Vec<String> = std::env::args().collect();
    let socket_address = &args[1];
    println!("starting server, using address [{}])...", socket_address);

    let player_id_resolver = make_player_id_resolver();
    let server_factory = make_server_factory(&update_channel_tx, &player_id_resolver);
    let socket = ws::Builder::new().build(server_factory).unwrap();
    let broadcaster = socket.broadcaster();
    // Start update daemon
    let game = GameUpdater {
        state: api::GameState {
            players: HashMap::new(),
            enemies: vec![],
            bullets: vec![],
        },
        update_channel_rx,
        map: GameMap { max_dimension: 100 },
        broadcaster: broadcaster,
    };
    let terminate_fn = start_game_update_daemon(game)?;

    // start listening (on event loop)
    if let Err(error) = socket.listen(socket_address) {
        println!("failed to create WebSocket due to {:?}", error)
    }

    // if the websockets server quit for some reason, terminate the update daemon
    terminate_fn();

    println!("server closed.");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
