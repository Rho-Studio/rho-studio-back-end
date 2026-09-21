use crate::error::AppResult;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, FromRow)]
pub struct ExerciseResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub gif_url: Option<String>,
    pub muscle_group: String,
    pub equipment: Option<String>,
    pub category: Option<String>,
}

#[derive(Debug, FromRow)]
struct ExerciseRow {
    id: Uuid,
    source_id: Option<String>,
    name: String,
    description: Option<String>,
    gif_storage_key: Option<String>,
    muscle_group: String,
    equipment: Option<String>,
    category: Option<String>,
}

pub async fn get_exercises(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<ExerciseResponse>>> {
    let exercises = sqlx::query_as::<_, ExerciseRow>(
        r#"
        SELECT
            id,
            source_id,
            name,
            description,
            gif_storage_key,
            muscle_group,
            equipment,
            category
        FROM exercises
        WHERE is_published = true
        ORDER BY name
        "#,
    )
    .fetch_all(&state.pool)
    .await?;

    let response = exercises
        .into_iter()
        .map(|exercise| ExerciseResponse {
            id: exercise
                .source_id
                .unwrap_or_else(|| exercise.id.to_string()),
            name: exercise.name,
            description: exercise.description,
            gif_url: exercise
                .gif_storage_key
                .as_deref()
                .map(|key| state.storage.public_url(key)),
            muscle_group: exercise.muscle_group,
            equipment: exercise.equipment,
            category: exercise.category,
        })
        .collect();

    Ok(Json(response))
}
