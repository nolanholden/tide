mod api_types;
mod config;
mod game_control;
mod geography;
mod intercomm;
mod server;
mod utils;

#[macro_use]
extern crate log;

use std::sync::mpsc;

fn parse_args() -> (String,) {
    let args: Vec<String> = std::env::args().collect();
    let socket_address = args[1].clone();
    (socket_address,)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    config::init();

    let (socket_address,) = parse_args();
    info!("starting server, using address [{}]...", socket_address);

    // Create communication channel between websockets servers and
    // the update game_controller.
    let (update_channel_tx, update_channel_rx) = mpsc::channel();

    // Configure websockets server(s).
    let resolver = utils::PlayerIdResolver::new();
    let (socket, broadcaster) = server::set_up_websockets_server(&update_channel_tx, &resolver);

    // Start update game_controller.
    let map = geography::GameMap { max_dimension: 100 };
    let game = game_control::GameController::new(update_channel_rx, broadcaster, map);
    let terminate_fn = game_control::start_game_controller_thread(game)?;

    // Start listening (on event loop).
    if let Err(error) = socket.listen(socket_address) {
        error!("failed to create websocket due to {:?}", error)
    }

    // If the websockets server quit for some reason, terminate the update game_controller.
    terminate_fn();

    info!("game server closed.");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
