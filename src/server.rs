use crate::api_types as api;
use crate::intercomm::ChannelUpdate;
use crate::utils;

use ws;

use std::sync::mpsc;

pub fn set_up_websockets_server<'a>(
    update_channel_tx: &'a mpsc::Sender<ChannelUpdate>,
    player_id_resolver: &'a utils::PlayerIdResolver,
) -> (ws::WebSocket<ServerFactory<'a>>, ws::Sender) {
    let server_factory = ServerFactory {
        update_channel: &update_channel_tx,
        player_id_resolver: &player_id_resolver,
    };
    let socket = ws::Builder::new().build(server_factory).unwrap();
    let broadcaster = socket.broadcaster();
    (socket, broadcaster)
}

// websockets game server
pub struct GameServer<'a> {
    out: ws::Sender,
    update_channel: mpsc::Sender<ChannelUpdate>,
    player_id_resolver: &'a utils::PlayerIdResolver,
    player_id: Option<api::PlayerId>,
}

impl<'a> GameServer<'_> {
    fn new(
        out: ws::Sender,
        update_channel: mpsc::Sender<ChannelUpdate>,
        player_id_resolver: &'a utils::PlayerIdResolver,
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

pub struct ServerFactory<'a> {
    update_channel: &'a mpsc::Sender<ChannelUpdate>,
    player_id_resolver: &'a utils::PlayerIdResolver,
}
impl<'a> ws::Factory for ServerFactory<'a> {
    type Handler = GameServer<'a>;
    fn connection_made(&mut self, sender: ws::Sender) -> GameServer<'a> {
        info!(
            "connected with client, connection id=[{}]",
            sender.connection_id()
        );
        GameServer::new(sender, self.update_channel.clone(), self.player_id_resolver)
    }
}
