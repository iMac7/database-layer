use actix_web::HttpResponse;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::model::Notifications;
use crate::database::Connection;
use crate::message::MessageResponder;
use crate::operation::{self, Operation, SuccessOutput};

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum NotificationMessage {
    NotificationsQuery(notifications_query::Payload),
    NotificationSetStateMutation(set_state_mutation::Payload),
}

#[async_trait]
impl MessageResponder for NotificationMessage {
    #[allow(clippy::async_yields_async)]
    async fn handle(&self, connection: Connection<'_, '_>) -> HttpResponse {
        match self {
            NotificationMessage::NotificationsQuery(payload) => {
                payload.handle("NotificationsQuery", connection).await
            }
            NotificationMessage::NotificationSetStateMutation(payload) => {
                payload
                    .handle("NotificationSetStateMutation", connection)
                    .await
            }
        }
    }
}

pub mod notifications_query {
    use super::*;

    #[derive(Deserialize, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Payload {
        pub user_id: i32,
    }

    #[async_trait]
    impl Operation for Payload {
        type Output = Notifications;

        async fn execute(&self, connection: Connection<'_, '_>) -> operation::Result<Self::Output> {
            Ok(match connection {
                Connection::Pool(pool) => Notifications::fetch(self.user_id, pool).await?,
                Connection::Transaction(transaction) => {
                    Notifications::fetch_via_transaction(self.user_id, transaction).await?
                }
            })
        }
    }
}

pub mod set_state_mutation {
    use super::*;

    #[derive(Deserialize, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Payload {
        pub ids: Vec<i32>,
        pub user_id: i32,
        pub unread: bool,
    }

    #[async_trait]
    impl Operation for Payload {
        type Output = SuccessOutput;

        async fn execute(&self, connection: Connection<'_, '_>) -> operation::Result<Self::Output> {
            match connection {
                Connection::Pool(pool) => Notifications::set_notification_state(self, pool).await?,
                Connection::Transaction(transaction) => {
                    Notifications::set_notification_state(self, transaction).await?
                }
            };
            Ok(SuccessOutput { success: true })
        }
    }
}
