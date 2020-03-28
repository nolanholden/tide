use crate::api_types as api;
use crate::config;
use crate::intercomm::ChannelUpdate;
use crate::utils;
use crate::utils::SerialIdGenerator;

use mio_extras::timer::Timeout;
use ws;
use ws::util::Token;

use std::sync::mpsc;

pub fn set_up_websockets_server<'a>(
    update_channel_tx: &'a mpsc::Sender<ChannelUpdate>,
    player_id_gen: &'a utils::PlayerIdGenerator,
) -> (ws::WebSocket<ServerFactory<'a>>, ws::Sender) {
    let server_factory = ServerFactory {
        update_channel: &update_channel_tx,
        player_id_gen: player_id_gen,
    };
    let socket = ws::Builder::new().build(server_factory).unwrap();
    let broadcaster = socket.broadcaster();
    (socket, broadcaster)
}

const PING: Token = Token(1);

// websockets game server
pub struct GameServer<'a> {
    out: ws::Sender,
    update_channel: mpsc::Sender<ChannelUpdate>,
    player_id: Option<api::PlayerId>,
    ping_timeout: Option<Timeout>,
    player_id_gen: &'a utils::PlayerIdGenerator,
}

impl<'a> GameServer<'a> {
    fn new(
        out: ws::Sender,
        update_channel: mpsc::Sender<ChannelUpdate>,
        player_id_gen: &'a utils::PlayerIdGenerator,
    ) -> GameServer<'_> {
        GameServer {
            out: out,
            update_channel: update_channel,
            player_id: None,
            ping_timeout: None,
            player_id_gen: player_id_gen,
        }
    }
}

impl<'a> GameServer<'_> {
    fn send_out(&mut self, data: String) -> ws::Result<()> {
        debug!("server sending: [{}]", data);
        self.out.send(data)
    }

    fn send_ping(&mut self) -> ws::Result<()> {
        self.out.ping(utils::custom_time_ns().to_string().into())
    }
}

impl<'a> ws::Handler for GameServer<'_> {
    fn on_open(&mut self, shake: ws::Handshake) -> ws::Result<()> {
        let id: api::PlayerId = self.player_id_gen.get_next_id();
        info!("Connection with player [{}] now open, with ip addresses = {{ local=[{}], peer=[{}], remote=[{}] }}", &id, shake.local_addr.unwrap(), shake.peer_addr.unwrap(), shake.remote_addr()?.unwrap());

        // setup time sync timeouts
        self.player_id = Some(id.clone());
        self.send_ping()?;
        self.out
            .timeout(config::WEBSOCKETS_PINGPONG_INTERVAL_MS(), PING)?;
        self.update_channel
            .send(ChannelUpdate {
                id: id.clone(),
                update: api::ClientUpdate::PlayerConnected(()),
            })
            .unwrap();
        let player_id_assignment_msg =
            serde_json::ser::to_string(&api::ServerUpdate::YourPlayerId(api::PlayerIdMessage {
                player_id: id,
            }))
            .unwrap();
        self.out.send(player_id_assignment_msg)
    }

    fn on_message(&mut self, msg: ws::Message) -> ws::Result<()> {
        debug!(
            "server got from player [{}]: [{}]",
            self.player_id.as_ref().unwrap(),
            msg
        );
        let id = self.player_id.as_ref().unwrap().clone();
        match msg {
            ws::Message::Text(json) => match serde_json::from_str(&json) {
                Ok(update) => {
                    self.update_channel
                        .send(ChannelUpdate { id, update })
                        .unwrap();
                }
                Err(error) => {
                    let error_msg = format!(
                        "unrecognized message from player [{}]: [{}], error: [{:?}]",
                        self.player_id.as_ref().unwrap(),
                        json,
                        error
                    );
                    warn!("{}", error_msg);
                    return self.send_out(error_msg);
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

    fn on_timeout(&mut self, event: Token) -> ws::Result<()> {
        match event {
            // PING timeout has occured, send a ping and reschedule
            PING => {
                self.send_ping()?;
                self.ping_timeout.take();
                self.out.timeout(5_000, PING)
            }
            // No other timeouts are possible
            _ => Err(ws::Error::new(
                ws::ErrorKind::Internal,
                "unrecognized timeout token encountered",
            )),
        }
    }

    fn on_new_timeout(&mut self, event: Token, timeout: Timeout) -> ws::Result<()> {
        // Cancel the old timeout and replace.
        // This ensures there is only one ping timeout at a time
        match event {
            PING => {
                if let Some(t) = self.ping_timeout.take() {
                    self.out.cancel(t)?
                }
                self.ping_timeout = Some(timeout)
            }
            _ => {
                return Err(ws::Error::new(
                    ws::ErrorKind::Internal,
                    "unrecognized timeout token encountered",
                ))
            }
        }

        Ok(())
    }

    fn on_frame(&mut self, frame: ws::Frame) -> ws::Result<Option<ws::Frame>> {
        // If the frame is a pong, print the round-trip time.
        // The pong should contain data from out ping, but it isn't guaranteed to.
        if frame.opcode() == ws::OpCode::Pong {
            if let Ok(pong) = std::str::from_utf8(frame.payload())?.parse::<u64>() {
                let now = utils::custom_time_ns();
                debug!(
                    "round trip time for player [{}] = {}ms",
                    self.player_id.as_ref().unwrap(),
                    (now - pong) as f32 / 1_000_000f32
                );
            } else {
                warn!("received bad pong");
            }
        }

        // Run default frame validation
        DefaultHandler.on_frame(frame)
    }

    fn on_close(&mut self, code: ws::CloseCode, reason: &str) {
        info!("WebSocket closing for [{:?}], reason: [{}]", code, reason);
        // Clean up time sync timeout.
        if let Some(t) = self.ping_timeout.take() {
            self.out.cancel(t).unwrap();
        }
        // Close ipc channel.
        self.update_channel
            .send(ChannelUpdate {
                id: self.player_id.as_ref().unwrap().clone(),
                update: api::ClientUpdate::PlayerDisconnected(()),
            })
            .unwrap();

        // Inform all existing clients of this player being disconnected.
        // TODO: can we build in reconnection?
        let player_disconnected_msg = serde_json::ser::to_string(
            &api::ServerUpdate::PlayerDisconnected(api::PlayerIdMessage {
                player_id: self.player_id.as_ref().unwrap().clone(),
            }),
        )
        .unwrap();
        self.out.broadcast(player_disconnected_msg).unwrap();
    }
}

// For accessing the default handler implementation
struct DefaultHandler;

impl ws::Handler for DefaultHandler {}

pub struct ServerFactory<'a> {
    update_channel: &'a mpsc::Sender<ChannelUpdate>,
    player_id_gen: &'a utils::PlayerIdGenerator,
}

impl<'a> ws::Factory for ServerFactory<'a> {
    type Handler = GameServer<'a>;
    fn connection_made(&mut self, sender: ws::Sender) -> GameServer<'a> {
        info!(
            "connected with client, connection id=[{}]",
            sender.connection_id()
        );
        GameServer::new(sender, self.update_channel.clone(), self.player_id_gen)
    }
}
