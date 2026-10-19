use std::task::Poll;

use twilight_gateway::{
    Message, ShardState,
    error::{ReceiveMessageError, ReceiveMessageErrorType},
};

use super::Envelope;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GatewayConnectionStatus {
    #[default]
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

impl GatewayConnectionStatus {
    pub(super) fn set(&mut self, next: Self) -> Option<Self> {
        if *self == next {
            return None;
        }
        *self = next;
        Some(next)
    }

    pub(super) fn observe_poll(
        &mut self,
        state: ShardState,
        poll: &Poll<Option<Result<Message, ReceiveMessageError>>>,
    ) -> Option<Self> {
        let next = match (state, poll) {
            (ShardState::FatallyClosed, _) | (_, Poll::Ready(None)) => Self::Disconnected,
            (ShardState::Active, Poll::Ready(Some(Err(error))))
                if matches!(error.kind(), ReceiveMessageErrorType::Deserializing { .. }) =>
            {
                return None;
            }
            (_, Poll::Ready(Some(Err(_) | Ok(Message::Close(_))))) => Self::Reconnecting,
            (ShardState::Active, _) => return None,
            _ if *self == Self::Connecting => Self::Connecting,
            _ => Self::Reconnecting,
        };
        self.set(next)
    }

    pub(super) fn confirm_session(
        &mut self,
        state: ShardState,
        envelope: &Envelope<'_>,
    ) -> Option<Self> {
        if state == ShardState::Active
            && envelope.op == 0
            && matches!(envelope.t, Some("READY" | "RESUMED"))
        {
            self.set(Self::Connected)
        } else {
            None
        }
    }
}
