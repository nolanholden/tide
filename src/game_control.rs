use crate::api_types;
use crate::config;
use crate::geography::GameMap;
use crate::intercomm::ChannelUpdate;
use crate::utils;
use api::projectile_info::lookup_projectile_info;
use api::Vec2;
use api_types as api;

use ncollide2d as nc;

use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use specs::prelude::*;
use specs::Component;
// use specs::{
//     Builder, Component, DispatcherBuilder, Entities, Join, Read, ReadStorage, System, VecStorage,
//     World, WorldExt, WriteStorage,
// };

#[derive(Debug, Component)]
#[storage(VecStorage)]
struct Position(Vec2);

#[derive(Debug, Component)]
#[storage(VecStorage)]
struct Velocity(Vec2);

#[derive(Default)]
struct Receiver(Option<std::sync::Mutex<std::sync::mpsc::Receiver<ChannelUpdate>>>);

#[derive(Default)]
struct TimeDeltaSecs(f64);

struct UpdatePosition;

#[derive(Debug, Component)]
#[storage(VecStorage)]
struct Colliding(bool);

impl<'a> System<'a> for UpdatePosition {
    type SystemData = (
        ReadStorage<'a, ProjectileEntity>,
        ReadStorage<'a, Velocity>,
        WriteStorage<'a, Position>,
        Read<'a, TimeDeltaSecs>,
    );
    fn run(&mut self, (projectiles, vel, mut pos, delta): Self::SystemData) {
        for (_, pos, vel) in (&projectiles, &mut pos, &vel).join() {
            pos.0[0] = (vel.0[0] as f64 * delta.0) as f32;
            pos.0[1] = (vel.0[1] as f64 * delta.0) as f32;
        }
    }
}

#[derive(Debug, Component)]
#[storage(VecStorage)]
struct PlayerEntity {
    player_id: api::PlayerId,
    connection_status: api::ConnectionStatus,
}

#[derive(Debug, Component)]
#[storage(VecStorage)]
struct ProjectileEntity {
    proj_type: api::ProjectileType,
}

use specs::shrev::ReaderId;

struct ClientUpdateHandler;
impl ClientUpdateHandler {
    fn handle_player_connection_updates<'a>(
        &self,
        id: api::PlayerId,
        update: api::ClientUpdate,
        players: WriteStorage<'a, PlayerEntity>,
        entities: Entities<'a>,
        updater: Read<'a, LazyUpdate>,
    ) {
        let add_new_player = || {
            let new_player = entities.create();
            updater.insert(
                new_player,
                PlayerEntity {
                    player_id: id,
                    connection_status: api::ConnectionStatus::Connected,
                },
            );
            updater.insert(new_player, Position(Vec2::new(0.0, 0.0)));
            updater.insert(new_player, Velocity(Vec2::new(0.0, 0.0)));
        };
        match update {
            api::ClientUpdate::PlayerConnected(()) => {
                match (&players).join().find(|p| p.player_id == id) {
                    Some(player) => {
                        if player.connection_status != api::ConnectionStatus::Disconnected {
                            panic!("trying to reconnect to an existing id")
                        }
                        player.connection_status = api::ConnectionStatus::Connected;
                    }
                    None => {
                        add_new_player();
                    }
                }
            }
            api::ClientUpdate::PlayerDisconnected(()) => {
                match (&players).join().find(|p| p.player_id == id) {
                    Some(player) => {
                        if player.connection_status == api::ConnectionStatus::Disconnected {
                            panic!("trying to disconnect an already-disconnected player");
                        }
                        player.connection_status = api::ConnectionStatus::Disconnected;
                    }
                    None => panic!("trying to disconnect a nonexistent player"),
                };
            }
            _ => panic!("invalid type of update"),
        };
    }
}

impl<'a> System<'a> for ClientUpdateHandler {
    type SystemData = (
        Read<'a, Receiver>,
        Entities<'a>,
        Read<'a, LazyUpdate>,
        WriteStorage<'a, PlayerEntity>,
        WriteStorage<'a, Position>,
        WriteStorage<'a, Velocity>,
        WriteStorage<'a, ProjectileEntity>,
    );
    fn run(
        &mut self,
        (receiver,  entities, updater, players, positions, velocities, projectiles): Self::SystemData,
    ) {
        let channel = receiver.0.as_ref().unwrap().lock().unwrap();
        loop {
            let ChannelUpdate { id, update } = match channel.try_recv() {
                Ok(x) => x,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("client update channel disconnected")
                }
            };
            match update {
                api::ClientUpdate::PlayerConnected(())
                | api::ClientUpdate::PlayerDisconnected(()) => {
                    self.handle_player_connection_updates(id, update, players, entities, updater)
                }
                api::ClientUpdate::PositionUpdate(update) => {
                    (&players, &mut positions, &mut velocities).join().for_each(
                        move |(player, pos, vel)| {
                            if player.player_id == id {
                                pos.0 = update.xy;
                                vel.0 = update.vel;
                            }
                        },
                    );
                }
                api::ClientUpdate::ProjectileCreated(proj) => {
                    // TODO: how do we handle possibility of crossing
                    // over enemies which should have been hit (due to
                    // insufficient granularity)
                    let e = entities.create();
                    updater.insert(
                        e,
                        ProjectileEntity {
                            proj_type: proj.projectile_type,
                        },
                    );
                    updater.insert(e, Position(Vec2::new(0.0, 0.0)));
                    updater.insert(e, Velocity(Vec2::new(0.0, 0.0)));
                }
            };
        }
    }
}

// #[derive(Default)]
// struct Sys {
//     reader: Option<ReaderId<ChannelUpdate>>,
// }

// impl<'a> System<'a> for Sys {
//     type SystemData = Read<'a, cbc::Receiver<ChannelUpdate>>;

//     fn run(&mut self, events: Self::SystemData) {
//         for ChannelUpdate { id, update } in events.read(&mut self.reader.as_mut().unwrap()) {
//             match update {
//                 api::ClientUpdate::PlayerConnected(()) => self.try_connect_player(id)?,
//                 api::ClientUpdate::PlayerDisconnected(()) => self.disconnect_player(id),
//                 api::ClientUpdate::PositionUpdate(position) => {
//                     self.get_player(&id).position = position
//                 }
//                 api::ClientUpdate::ProjectileCreated(proj) => {
//                     self.handle_projectile_created(id, proj)
//                 }
//             };
//         }
//     }

//     fn setup(&mut self, world: &mut World) {
//         Self::SystemData::setup(world);
//         self.reader = Some(
//             world
//                 .fetch_mut::<EventChannel<ChannelUpdate>>()
//                 .register_reader(),
//         );
//     }
// }

#[derive(Default)]
pub struct ComponentChangeObserver {
    pub dirty: BitSet,
    pub reader_id: Option<ReaderId<ComponentEvent>>,
}

#[derive(Debug)]
pub struct HitScanProjectile;

impl Component for HitScanProjectile {
    type Storage = FlaggedStorage<Self, DenseVecStorage<Self>>;
}

impl<'a> System<'a> for ComponentChangeObserver {
    type SystemData = (ReadStorage<'a, HitScanProjectile>,);

    fn run(&mut self, (data,): Self::SystemData) {
        self.dirty.clear();

        let events = data.channel().read(self.reader_id.as_mut().unwrap());

        // Note that we could use separate bitsets here, we only use one to
        // simplify the example
        for event in events {
            match event {
                ComponentEvent::Modified(id) | ComponentEvent::Inserted(id) => {
                    self.dirty.add(*id);
                }
                // We don't need to take this event into account since
                // removed components will be filtered out by the join;
                // if you want to, you can use `self.dirty.remove(*id);`
                // so the bit set only contains IDs that still exist
                ComponentEvent::Removed(_) => (),
            }
        }

        // for (d, other, _) in (&data, &mut some_other_data, &self.dirty).join() {
        //     // Mutate `other` based on the update data in `d`
        // }
    }

    fn setup(&mut self, res: &mut World) {
        Self::SystemData::setup(res);
        self.reader_id = Some(WriteStorage::<HitScanProjectile>::fetch(&res).register_reader());
    }
}

pub fn start_game_controller_thread(
    receiver: mpsc::Receiver<ChannelUpdate>,
) -> Result<impl FnOnce() -> (), Box<dyn std::error::Error>> {
    let (cancelled, cancel) = utils::make_atomic_canceller();
    let game_controller = thread::Builder::new()
        .name("GameController".to_owned())
        .spawn(move || {
            let mut world = World::new();
            world.insert(Receiver(Some(std::sync::Mutex::new(receiver))));
            use crossbeam_channel::unbounded;
            let mut channel = unbounded::<ChannelUpdate>();
            world.insert(channel);
            world.register::<Position>();
            world.register::<Velocity>();
            let mut last_time = std::time::Instant::now();
            let get_delta_time = move || {
                let time_now = std::time::Instant::now();
                let delta = time_now.duration_since(last_time);
                last_time = time_now;
                TimeDeltaSecs(delta.as_secs_f64())
            };
            let mut client_update_handler = ClientUpdateHandler;
            let mut update_position = UpdatePosition;
            while !cancelled() {
                world.insert(get_delta_time());
                client_update_handler.run_now(&world);
                update_position.run_now(&world);
                // dispatcher.dispatch(&mut world);
                world.maintain();
            }
        })?;
    let terminate_game_controller = move || {
        info!("requesting game controller thread to stop...");
        cancel();
        game_controller.join().unwrap()
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
                projectiles: vec![],
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
            api::ClientUpdate::ProjectileCreated(proj) => self.handle_projectile_created(id, proj),
        };
        debug!(" --> state: {:?}", self.state.players);
        Ok(())
    }

    pub fn progress_projectiles(&mut self) -> Result<(), String> {
        let now_ms = utils::unix_time().as_millis() as u64;
        let mut remaining_alive_enemies: Vec<&mut api::Enemy> =
            self.state.enemies.iter_mut().collect();
        for player_proj in self.state.projectiles.iter_mut() {
            let delta_ms = now_ms - player_proj.projectile.origin.time_ms;
            let delta_secs = delta_ms as f32 / 1000.0;
            let pos_update_vector = player_proj.projectile.vel * delta_secs;
            remaining_alive_enemies = remaining_alive_enemies
                .into_iter()
                .filter_map(|enemy: &mut api::Enemy| {
                    if *player_proj.current_info.num_penetrations.as_ref().unwrap() > 0 {
                        // Check if position update vector at all intersects the enemy bounding box
                        let projectile_hits_enemy =
                            projectile_ray_scans_enemy(&player_proj.projectile, &enemy);
                        if projectile_hits_enemy {
                            // TODO: add way to optionally backoff/decrease {speed,damage} on the projectile
                            enemy.health -= player_proj.current_info.damage;
                            if enemy.health < 0 {
                                enemy.status = api::EnemyStatus::Dead;
                            }
                            *player_proj.current_info.num_penetrations.as_mut().unwrap() -= 1;
                        }
                    }
                    return match enemy.status {
                        api::EnemyStatus::Alive => Some(enemy),
                        api::EnemyStatus::Dead => None,
                        api::EnemyStatus::Unspecified => panic!("invalid enemy state"),
                    };
                })
                .collect();
            // move the projectile forward
            player_proj.projectile.origin = api::PositionStamped {
                xy: player_proj.projectile.origin.xy + pos_update_vector,
                time_ms: now_ms,
            };
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
            self.progress_projectiles()?;
            self.broadcast_state()?;
            // We'll wait as long as the full timeout for any client messages.
            // Thus, the timeout is the worst-case granularity of internal updates.
            // If we get client message *more* frequently, we expect to see
            // higher granularity to compensate.
            match self
                .update_channel_rx
                .recv_timeout(config::AWAIT_CLIENT_MSG_TIMEOUT_MS())
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
    pub fn handle_projectile_created(
        &mut self,
        id: api::PlayerId,
        projectile: api::ProjectileSnaphot,
    ) {
        let projectile_info = lookup_projectile_info(projectile.projectile_type);
        match projectile_info.speed {
            Some(speed) => {
                // TODO: how do we handle possibility of crossing
                // over enemies which should have been hit (due to
                // insufficient granularity)
                self.state.projectiles.push(api::PlayerProjectile {
                    player_id: id,
                    projectile: api::ProjectileSnaphot {
                        projectile_type: projectile.projectile_type,
                        origin: projectile.origin,
                        vel: projectile.vel.normalize().scale(speed),
                    },
                    current_info: projectile_info.clone(),
                });
            }
            None => {
                let (mut in_projectile_path, _): (Vec<&mut api::Enemy>, Vec<&mut api::Enemy>) =
                    self.state
                        .enemies
                        .iter_mut()
                        .partition(|ref enemy| projectile_ray_scans_enemy(&projectile, enemy));
                for enemy in in_projectile_path.iter_mut() {
                    enemy.health -= projectile_info.damage;
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
                        authr_status: api::AuthorizationStatus::GoodStanding,
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

pub fn projectile_ray_scans_enemy(proj: &api::ProjectileSnaphot, enemy: &api::Enemy) -> bool {
    use nc::math::Isometry;
    use nc::math::Point;
    use nc::query::RayCast;
    use nc::shape::shape::Shape;
    use std::f32;

    let (half_width, half_height) = (0.5, 0.5);
    let enemy_shape = nc::shape::Cuboid::new(Vec2::new(half_width, half_height));
    let enemy_hit_boundary = enemy_shape.aabb(&Isometry::new(enemy.position.xy, 0f32));
    let projectile_ray = nc::query::Ray::new(Point::from(proj.origin.xy), proj.vel.normalize());
    // TODO: this currently shoots through walls, fix that
    enemy_hit_boundary.intersects_ray(
        &Isometry::identity(),
        &projectile_ray,
        f32::MAX, // e.g. scalar scaling of vector
    )
}
