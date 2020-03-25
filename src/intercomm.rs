use crate::api_types as api;

pub struct ChannelUpdate {
    pub id: api::PlayerId,
    pub update: api::ClientUpdate,
}
