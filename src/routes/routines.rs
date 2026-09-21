use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::ObjectStorage;
use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, PartialEq)]
pub struct RoutineResponse {
    pub name: String,
    pub description: Option<String>,
    pub author: RoutineAuthorResponse,
    pub exercises: Vec<RoutineExerciseResponse>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct RoutineAuthorResponse {
    pub name: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct RoutineExerciseResponse {
    pub id: String,
    pub name: String,
    pub gif_url: Option<String>,
    pub position: i32,
    pub sets: Option<i32>,
    pub reps: Option<i32>,
    pub rest_seconds: i32,
}

#[derive(Debug, FromRow)]
struct RoutineRow {
    id: Uuid,
    name: String,
    description: Option<String>,
    author_name: Option<String>,
}

#[derive(Debug, FromRow)]
struct RoutineExerciseRow {
    exercise_id: Uuid,
    exercise_source_id: Option<String>,
    exercise_name: String,
    gif_storage_key: Option<String>,
    position: i32,
    sets: Option<i32>,
    reps: Option<i32>,
    rest_seconds: i32,
}

fn build_response(
    routine: RoutineRow,
    exercises: Vec<RoutineExerciseRow>,
    storage: &dyn ObjectStorage,
) -> RoutineResponse {
    RoutineResponse {
        name: routine.name,
        description: routine.description,
        author: RoutineAuthorResponse {
            name: routine
                .author_name
                .unwrap_or_else(|| "Unknown author".to_string()),
        },
        exercises: exercises
            .into_iter()
            .map(|exercise| RoutineExerciseResponse {
                id: exercise
                    .exercise_source_id
                    .unwrap_or_else(|| exercise.exercise_id.to_string()),
                name: exercise.exercise_name,
                gif_url: exercise
                    .gif_storage_key
                    .as_deref()
                    .map(|key| storage.public_url(key)),
                position: exercise.position,
                sets: exercise.sets,
                reps: exercise.reps,
                rest_seconds: exercise.rest_seconds,
            })
            .collect(),
    }
}

async fn load_routine_response(
    state: &AppState,
    routine: RoutineRow,
) -> AppResult<Json<RoutineResponse>> {
    let exercises = sqlx::query_as::<_, RoutineExerciseRow>(
        r#"
        SELECT
            e.id AS exercise_id,
            e.source_id AS exercise_source_id,
            e.name AS exercise_name,
            e.gif_storage_key,
            re.position,
            re.sets,
            re.reps,
            re.rest_seconds
        FROM routine_exercises re
        JOIN exercises e ON e.id = re.exercise_id
        WHERE re.routine_id = $1
        ORDER BY re.position
        "#,
    )
    .bind(routine.id)
    .fetch_all(&state.pool)
    .await?;

    let response = build_response(routine, exercises, state.storage.as_ref());

    Ok(Json(response))
}

pub async fn get_routine(
    State(state): State<AppState>,
    Path(routine_id): Path<Uuid>,
) -> AppResult<Json<RoutineResponse>> {
    let routine = sqlx::query_as::<_, RoutineRow>(
        r#"
        SELECT
            r.id,
            r.name,
            r.description,
            u.display_name AS author_name
        FROM routines r
        JOIN users u ON u.id = r.created_by
        WHERE r.id = $1
          AND r.is_public = true
        "#,
    )
    .bind(routine_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("routine {routine_id}")))?;

    load_routine_response(&state, routine).await
}

pub async fn get_demo_routine(State(state): State<AppState>) -> AppResult<Json<RoutineResponse>> {
    if state.config.app.env != "development" {
        return Err(AppError::NotFound("demo routine".to_string()));
    }

    let routine = sqlx::query_as::<_, RoutineRow>(
        r#"
        SELECT
            r.id,
            r.name,
            r.description,
            u.display_name AS author_name
        FROM routines r
        JOIN users u ON u.id = r.created_by
        WHERE r.name = $1
          AND r.is_public = true
        ORDER BY r.created_at DESC
        LIMIT 1
        "#,
    )
    .bind("API Test Routine")
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("demo routine".to_string()))?;

    load_routine_response(&state, routine).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageError;
    use async_trait::async_trait;

    struct TestStorage;

    #[async_trait]
    impl ObjectStorage for TestStorage {
        async fn put_object(
            &self,
            _key: &str,
            _bytes: Vec<u8>,
            _content_type: &str,
        ) -> Result<(), StorageError> {
            Err(StorageError::Backend("unused in test".to_string()))
        }

        async fn get_object(&self, _key: &str) -> Result<Vec<u8>, StorageError> {
            Err(StorageError::Backend("unused in test".to_string()))
        }

        async fn delete_object(&self, _key: &str) -> Result<(), StorageError> {
            Err(StorageError::Backend("unused in test".to_string()))
        }

        fn public_url(&self, key: &str) -> String {
            format!("https://cdn.example.test/{key}")
        }

        async fn presigned_put_url(
            &self,
            _key: &str,
            _expires_secs: u32,
        ) -> Result<String, StorageError> {
            Err(StorageError::Backend("unused in test".to_string()))
        }
    }

    #[test]
    fn response_maps_gif_keys_and_missing_gifs() {
        let routine_id = Uuid::nil();
        let response = build_response(
            RoutineRow {
                id: routine_id,
                name: "Strength".to_string(),
                description: Some("A routine".to_string()),
                author_name: Some("Alex".to_string()),
            },
            vec![
                RoutineExerciseRow {
                    exercise_id: Uuid::from_u128(1),
                    exercise_source_id: Some("0001".to_string()),
                    exercise_name: "Push Up".to_string(),
                    gif_storage_key: Some("push-ups.gif".to_string()),
                    position: 1,
                    sets: Some(3),
                    reps: Some(12),
                    rest_seconds: 60,
                },
                RoutineExerciseRow {
                    exercise_id: Uuid::from_u128(2),
                    exercise_source_id: None,
                    exercise_name: "Plank".to_string(),
                    gif_storage_key: None,
                    position: 2,
                    sets: Some(3),
                    reps: None,
                    rest_seconds: 45,
                },
            ],
            &TestStorage,
        );

        assert_eq!(response.author.name, "Alex");
        assert_eq!(
            response.exercises[0].gif_url.as_deref(),
            Some("https://cdn.example.test/push-ups.gif")
        );
        assert_eq!(response.exercises[1].gif_url, None);
        assert_eq!(response.exercises[1].position, 2);
    }
}
