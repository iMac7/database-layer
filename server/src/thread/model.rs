use super::messages::{
    create_comment_mutation, create_thread_mutation, edit_comment_mutation,
    set_thread_archived_mutation,
};
use serde::Serialize;
use sqlx::{MySqlPool, Row};

use crate::database::Executor;
use crate::datetime::DateTime;
use crate::event::{
    CreateCommentEventPayload, CreateThreadEventPayload, SetThreadStateEventPayload,
};
use crate::instance::Instance;
use crate::operation;
use crate::subscription::Subscription;
use crate::uuid::{Uuid, UuidFetcher};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Threads {
    pub first_comment_ids: Vec<i32>,
}

impl Threads {
    pub async fn fetch_all_threads<'a, E>(
        first: i32,
        after: Option<String>,
        instance: Option<Instance>,
        subject_id: Option<i32>,
        executor: E,
    ) -> Result<Self, operation::Error>
    where
        E: Executor<'a>,
    {
        let mut transaction = executor.begin().await?;

        let instance_id = match instance.as_ref() {
            Some(instance) => Some(Instance::fetch_id(instance, &mut transaction).await?),
            None => None,
        };

        let after_parsed = match after.as_ref() {
            Some(date) => DateTime::parse_from_rfc3339(date)?,
            None => DateTime::now(),
        };

        // TODO: use alias for MAX(GREATEST(...)) when sqlx supports it
        let result = sqlx::query!(
            r#"
                WITH RECURSIVE descendants AS (
                    SELECT id, parent_id
                    FROM term_taxonomy
                    WHERE (? is null OR id = ?)

                    UNION

                    SELECT tt.id, tt.parent_id
                    FROM term_taxonomy tt
                    JOIN descendants d ON tt.parent_id = d.id
                ), subject_entities AS (
                SELECT id as entity_id FROM descendants

                UNION

                SELECT tte.entity_id
                FROM descendants
                JOIN term_taxonomy_entity tte ON descendants.id = tte.term_taxonomy_id

                UNION

                SELECT entity_link.child_id
                FROM descendants
                JOIN term_taxonomy_entity tte ON descendants.id = tte.term_taxonomy_id
                JOIN entity_link ON entity_link.parent_id = tte.entity_id

                UNION

                SELECT entity_link.child_id
                FROM descendants
                JOIN term_taxonomy_entity tte ON descendants.id = tte.term_taxonomy_id
                JOIN entity_link parent_link ON parent_link.parent_id = tte.entity_id
                JOIN entity_link ON entity_link.parent_id = parent_link.child_id
                )
                SELECT comment.id
                FROM comment
                JOIN uuid ON uuid.id = comment.id
                JOIN comment answer ON comment.id = answer.parent_id OR
                    comment.id = answer.id
                JOIN uuid parent_uuid ON parent_uuid.id = comment.uuid_id
                JOIN subject_entities ON subject_entities.entity_id = comment.uuid_id
                WHERE
                    comment.uuid_id IS NOT NULL
                    AND uuid.trashed = 0
                    AND comment.archived = 0
                    AND (? is null OR comment.instance_id = ?)
                    AND parent_uuid.discriminator != "user"
                GROUP BY comment.id
                HAVING MAX(GREATEST(answer.date, comment.date)) < ?
                ORDER BY MAX(GREATEST(answer.date, comment.date)) DESC
                LIMIT ?;
            "#,
            subject_id,
            subject_id,
            instance_id,
            instance_id,
            after_parsed,
            first
        )
        .fetch_all(&mut transaction)
        .await?;

        let first_comment_ids: Vec<i32> = result.iter().map(|child| child.id as i32).collect();

        Ok(Self { first_comment_ids })
    }

    pub async fn fetch(id: i32, pool: &MySqlPool) -> Result<Self, sqlx::Error> {
        Self::fetch_via_transaction(id, pool).await
    }

    pub async fn fetch_via_transaction<'a, E>(id: i32, executor: E) -> Result<Self, sqlx::Error>
    where
        E: Executor<'a>,
    {
        let result = sqlx::query!(
            r#"SELECT id FROM comment WHERE uuid_id = ? ORDER BY date DESC"#,
            id
        )
        .fetch_all(executor)
        .await?;

        let first_comment_ids: Vec<i32> = result.iter().map(|child| child.id as i32).collect();

        Ok(Self { first_comment_ids })
    }

    pub async fn set_archive<'a, E>(
        payload: &set_thread_archived_mutation::Payload,
        executor: E,
    ) -> Result<(), operation::Error>
    where
        E: Executor<'a>,
    {
        let mut transaction = executor.begin().await?;

        let number_comments = payload.ids.len();
        if number_comments == 0 {
            return Ok(());
        }
        let params = format!("?{}", ", ?".repeat(number_comments - 1));
        let query_str = format!("SELECT id, archived FROM comment WHERE id IN ( {params} )");
        let mut query = sqlx::query(&query_str);
        for id in payload.ids.iter() {
            query = query.bind(id);
        }
        let comments = query.fetch_all(&mut transaction).await?;
        if comments.len() < number_comments {
            return Err(operation::Error::BadRequest {
                reason: "not all given ids are comments".to_string(),
            });
        }

        let is_archived_after = payload.archived;
        for comment in comments {
            let id: i32 = comment.get("id");
            let is_archived_before: bool = comment.get("archived");
            if is_archived_after != is_archived_before {
                sqlx::query!(
                    r#"
                        UPDATE comment
                            SET archived = ?
                            WHERE id = ?
                    "#,
                    is_archived_after,
                    id
                )
                .execute(&mut transaction)
                .await?;

                SetThreadStateEventPayload::new(is_archived_after, payload.user_id, id)
                    .save(&mut transaction)
                    .await
                    .map_err(|error| operation::Error::InternalServerError {
                        error: Box::new(error),
                    })?;
            }
        }

        transaction.commit().await?;

        Ok(())
    }

    pub async fn comment_thread<'a, E>(
        payload: &create_comment_mutation::Payload,
        executor: E,
    ) -> Result<Uuid, operation::Error>
    where
        E: Executor<'a>,
    {
        if payload.content.is_empty() {
            return Err(operation::Error::BadRequest {
                reason: "content is empty".to_string(),
            });
        };

        let mut transaction = executor.begin().await?;

        let thread = sqlx::query!(
            r#"
                SELECT instance_id, archived
                    FROM comment
                    WHERE id = ?
            "#,
            payload.thread_id
        )
        .fetch_one(&mut transaction)
        .await
        .map_err(|error| match error {
            sqlx::Error::RowNotFound => operation::Error::BadRequest {
                reason: "thread does not exist".to_string(),
            },
            error => error.into(),
        })?;

        if thread.archived != 0 {
            // TODO: test is missing
            return Err(operation::Error::BadRequest {
                reason: "thread is already archived".to_string(),
            });
        }

        sqlx::query!(
            r#"
                INSERT INTO uuid (trashed, discriminator)
                    VALUES (0, 'comment')
            "#
        )
        .execute(&mut transaction)
        .await?;

        sqlx::query!(
            r#"
                INSERT INTO comment (id, date, archived, title, content, uuid_id, parent_id, author_id, instance_id )
                    VALUES (LAST_INSERT_ID(), ?, 0, NULL, ?, NULL, ?, ?, ?)
            "#,
            DateTime::now(),
            payload.content,
            payload.thread_id,
            payload.user_id,
            thread.instance_id
        )
        .execute(&mut transaction)
        .await?;

        let value = sqlx::query!(r#"SELECT LAST_INSERT_ID() as id"#)
            .fetch_one(&mut transaction)
            .await?;
        let comment_id = value.id as i32;

        CreateCommentEventPayload::new(
            payload.thread_id,
            comment_id,
            payload.user_id,
            thread.instance_id,
        )
        .save(&mut transaction)
        .await
        .map_err(|error| operation::Error::InternalServerError {
            error: Box::new(error),
        })?;

        if payload.subscribe {
            for object_id in [payload.thread_id, comment_id].iter() {
                let subscription = Subscription {
                    object_id: *object_id,
                    user_id: payload.user_id,
                    send_email: payload.send_email,
                };
                subscription.save(&mut transaction).await?;
            }
        }

        let comment = Uuid::fetch_via_transaction(comment_id, &mut transaction)
            .await
            .map_err(|error| operation::Error::InternalServerError {
                error: Box::new(error),
            })?;

        transaction.commit().await?;

        Ok(comment)
    }

    pub async fn start_thread<'a, E>(
        payload: &create_thread_mutation::Payload,
        executor: E,
    ) -> Result<Uuid, operation::Error>
    where
        E: Executor<'a>,
    {
        if payload.content.is_empty() {
            return Err(operation::Error::BadRequest {
                reason: "content is empty".to_string(),
            });
        }

        let mut transaction = executor.begin().await?;

        let instance_id = sqlx::query!(
            r#"
                SELECT i.id as instance_id
                    FROM uuid
                    JOIN (
                        SELECT id, instance_id FROM attachment_container
                        UNION ALL
                        SELECT id, instance_id FROM blog_post
                        UNION ALL
                        SELECT id, instance_id FROM comment
                        UNION ALL
                        SELECT id, instance_id FROM entity
                        UNION ALL
                        SELECT er.id, e.instance_id FROM entity_revision er JOIN entity e ON er.repository_id = e.id
                        UNION ALL
                        SELECT id, instance_id FROM page_repository
                        UNION ALL
                        SELECT pr.id, p.instance_id FROM page_revision pr JOIN page_repository p ON pr.page_repository_id = p.id
                        UNION ALL
                        SELECT ta.id, t.instance_id FROM term_taxonomy ta JOIN term t ON t.id = ta.term_id
                        UNION ALL
                        SELECT user.id, 1 FROM user) u
                    JOIN instance i ON i.id = u.instance_id
                    WHERE u.id = ?
            "#,
            payload.object_id
        )
        .fetch_one(&mut transaction)
        .await.map_err(|error| match error {
            sqlx::Error::RowNotFound => operation::Error::BadRequest{
                reason: "UUID not found".to_string(),
            },
            error => error.into(),})?
        .instance_id;

        sqlx::query!(
            r#"
                INSERT INTO uuid (trashed, discriminator)
                    VALUES (0, 'comment')
            "#
        )
        .execute(&mut transaction)
        .await?;

        sqlx::query!(
            r#"
                INSERT INTO comment ( id , date , archived , title , content , uuid_id , parent_id , author_id , instance_id )
                    VALUES (LAST_INSERT_ID(), ?, 0, ?, ?, ?, NULL, ?, ?)
            "#,
            DateTime::now(),
            payload.title,
            payload.content,
            payload.object_id,
            payload.user_id,
            instance_id
        )
        .execute(&mut transaction)
        .await?;

        let value = sqlx::query!(r#"SELECT LAST_INSERT_ID() as id"#)
            .fetch_one(&mut transaction)
            .await?;
        let thread_id = value.id as i32;

        CreateThreadEventPayload::new(thread_id, payload.object_id, payload.user_id, instance_id)
            .save(&mut transaction)
            .await
            .map_err(|error| operation::Error::InternalServerError {
                error: Box::new(error),
            })?;

        if payload.subscribe {
            let subscription = Subscription {
                object_id: thread_id,
                user_id: payload.user_id,
                send_email: payload.send_email,
            };
            subscription.save(&mut transaction).await?;
        }

        let comment = Uuid::fetch_via_transaction(thread_id, &mut transaction)
            .await
            .map_err(|error| operation::Error::InternalServerError {
                error: Box::new(error),
            })?;

        transaction.commit().await?;

        Ok(comment)
    }

    pub async fn edit_comment<'a, E>(
        payload: &edit_comment_mutation::Payload,
        executor: E,
    ) -> Result<operation::SuccessOutput, operation::Error>
    where
        E: Executor<'a>,
    {
        if payload.content.is_empty() {
            return Err(operation::Error::BadRequest {
                reason: "content is empty".to_string(),
            });
        }

        let mut transaction = executor.begin().await?;

        let comment = sqlx::query!(
            r#"
                SELECT content, author_id, archived, trashed
                FROM comment JOIN uuid ON uuid.id = comment.id
                WHERE comment.id = ?
            "#,
            payload.comment_id
        )
        .fetch_one(&mut transaction)
        .await
        .map_err(|error| match error {
            sqlx::Error::RowNotFound => operation::Error::BadRequest {
                reason: "no comment with given ID".to_string(),
            },
            error => error.into(),
        })?;

        if payload.user_id as i64 != comment.author_id {
            return Err(operation::Error::BadRequest {
                reason: "given user is not author of the comment".to_string(),
            });
        }

        if comment.archived != 0 {
            return Err(operation::Error::BadRequest {
                reason: "archived comment cannot be edited".to_string(),
            });
        }

        if comment.trashed != 0 {
            return Err(operation::Error::BadRequest {
                reason: "trashed comment cannot be edited".to_string(),
            });
        }

        if payload.content != comment.content.as_deref().unwrap_or("") {
            sqlx::query!(
                // todo: update edit_date (after database migration)
                // UPDATE comment SET content = ?, edit_date = ? WHERE id = ?
                r#"
                    UPDATE comment SET content = ? WHERE id = ?
                "#,
                payload.content,
                // DateTime::now(),
                payload.comment_id,
            )
            .execute(&mut transaction)
            .await?;
        }

        transaction.commit().await?;

        Ok(operation::SuccessOutput { success: true })
    }
}
