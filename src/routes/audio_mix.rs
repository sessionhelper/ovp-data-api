use axum::{
    extract::{Path, Query, State},
    http::header,
    response::IntoResponse,
    routing::get,
    Router,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::db::{participants, sessions as session_db};
use crate::error::AppError;
use crate::routes::AppState;
use crate::storage::audio;

/// 2 MB raw PCM chunk size.
const CHUNK_SIZE: usize = 2_097_152;
/// 2 bytes per sample * 2 channels = 4 bytes per stereo frame.
const FRAME_SIZE: usize = 4;
/// Number of stereo frames per chunk.
const SAMPLES_PER_CHUNK: usize = CHUNK_SIZE / FRAME_SIZE; // 524288
/// Duration of one chunk in seconds.
const CHUNK_DURATION: f64 = SAMPLES_PER_CHUNK as f64 / 48_000.0; // ~10.922s

/// Maximum allowed time range per request (seconds).
const MAX_RANGE_SECS: f64 = 60.0;

#[derive(Debug, Deserialize)]
struct MixParams {
    start: Option<f64>,
    end: f64,
    format: Option<String>,
}

async fn mixed_audio(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Query(params): Query<MixParams>,
) -> Result<impl IntoResponse, AppError> {
    let start = params.start.unwrap_or(0.0);
    let end = params.end;
    let format = params.format.unwrap_or_else(|| "wav".to_string());

    // Validate
    if start < 0.0 || end <= start {
        return Err(AppError::BadRequest(
            "start must be >= 0 and end must be > start".to_string(),
        ));
    }
    if end - start > MAX_RANGE_SECS {
        return Err(AppError::BadRequest(format!(
            "time range exceeds maximum of {MAX_RANGE_SECS}s"
        )));
    }
    if format != "wav" && format != "opus" {
        return Err(AppError::BadRequest(
            "format must be 'wav' or 'opus'".to_string(),
        ));
    }

    // Look up session
    let session = session_db::get(&state.pool, session_id).await?;
    let s3_prefix = session
        .s3_prefix
        .ok_or_else(|| AppError::BadRequest("session has no s3_prefix".to_string()))?;

    // Determine chunk range
    let start_chunk = (start / CHUNK_DURATION).floor() as u32;
    let end_chunk = (end / CHUNK_DURATION).floor() as u32;

    // Get all participants with pseudo_ids
    let participants = participants::list(&state.pool, session_id).await?;
    let pseudo_ids: Vec<String> = participants
        .iter()
        .filter_map(|p| p.user_pseudo_id.clone())
        .collect();

    if pseudo_ids.is_empty() {
        return Err(AppError::BadRequest(
            "no participants with audio found".to_string(),
        ));
    }

    // Download all needed chunks for all speakers concurrently
    let mut download_futures = Vec::new();
    for pseudo_id in &pseudo_ids {
        for seq in start_chunk..=end_chunk {
            let client = state.s3_client.clone();
            let bucket = state.s3_bucket.clone();
            let prefix = s3_prefix.clone();
            let pid = pseudo_id.clone();
            download_futures.push(tokio::spawn(async move {
                let result =
                    audio::download_chunk(&client, &bucket, &prefix, &pid, seq).await;
                (pid, seq, result)
            }));
        }
    }

    // Collect results: speaker -> seq -> bytes
    let chunks_per_seq = (end_chunk - start_chunk + 1) as usize;
    let mut speaker_chunks: Vec<Vec<Option<Vec<u8>>>> = Vec::new();
    // Map: pseudo_id index -> chunks array indexed by (seq - start_chunk)
    let mut pid_index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, pid) in pseudo_ids.iter().enumerate() {
        pid_index.insert(pid.clone(), i);
        speaker_chunks.push(vec![None; chunks_per_seq]);
    }

    for fut in download_futures {
        let (pid, seq, result) = fut
            .await
            .map_err(|e| AppError::Internal(format!("join error: {e}")))?;
        match result {
            Ok(bytes) => {
                if let Some(&idx) = pid_index.get(&pid) {
                    speaker_chunks[idx][(seq - start_chunk) as usize] = Some(bytes);
                }
            }
            Err(AppError::NotFound(_)) => {
                // Speaker was silent for this chunk — skip
            }
            Err(e) => return Err(e),
        }
    }

    // Concatenate each speaker's chunks into a contiguous buffer
    let total_chunk_samples = chunks_per_seq * CHUNK_SIZE; // total bytes across all chunks
    let mut speaker_buffers: Vec<Vec<u8>> = Vec::new();
    for chunks in &speaker_chunks {
        let mut buf = vec![0u8; total_chunk_samples];
        for (i, chunk) in chunks.iter().enumerate() {
            if let Some(data) = chunk {
                let offset = i * CHUNK_SIZE;
                let copy_len = data.len().min(CHUNK_SIZE);
                buf[offset..offset + copy_len].copy_from_slice(&data[..copy_len]);
            }
        }
        speaker_buffers.push(buf);
    }

    // Calculate sample byte offsets within the concatenated buffer.
    // Each stereo frame = 4 bytes (left_i16 + right_i16), so byte offset = frame_index * FRAME_SIZE.
    let start_byte = ((start - start_chunk as f64 * CHUNK_DURATION) * 48_000.0) as usize * FRAME_SIZE;
    let end_byte = {
        let end_offset_in_chunks =
            ((end - start_chunk as f64 * CHUNK_DURATION) * 48_000.0) as usize * FRAME_SIZE;
        end_offset_in_chunks.min(total_chunk_samples)
    };

    if start_byte >= end_byte || start_byte >= total_chunk_samples {
        return Err(AppError::BadRequest("calculated range is empty".to_string()));
    }

    // Mix all speakers' samples with clipping
    let pcm_len = end_byte - start_byte;
    // Ensure we work on an even number of bytes (aligned to i16 samples)
    let pcm_len = pcm_len & !1;
    let mut mixed = vec![0i16; pcm_len / 2];

    for buf in &speaker_buffers {
        let slice = &buf[start_byte..start_byte + pcm_len];
        for (i, sample) in mixed.iter_mut().enumerate() {
            let s = i16::from_le_bytes([slice[i * 2], slice[i * 2 + 1]]);
            let sum = (*sample as i32) + (s as i32);
            *sample = sum.clamp(-32768, 32767) as i16;
        }
    }

    // Convert mixed i16 back to le bytes
    let mut pcm_bytes = Vec::with_capacity(pcm_len);
    for s in &mixed {
        pcm_bytes.extend_from_slice(&s.to_le_bytes());
    }

    // Encode based on format
    // TODO: Add Opus encoding (needs audiopus + ogg crates and libopus-dev).
    // For now, always produce WAV.
    if format == "opus" {
        tracing::warn!("opus encoding not yet implemented, falling back to wav");
    }

    let wav_bytes = encode_wav(&pcm_bytes, 48_000, 2, 16);

    Ok((
        [
            (header::CONTENT_TYPE, "audio/wav"),
            (header::CACHE_CONTROL, "private, max-age=3600"),
        ],
        wav_bytes,
    ))
}

/// Encode raw s16le stereo PCM into a WAV file.
fn encode_wav(pcm: &[u8], sample_rate: u32, channels: u16, bits_per_sample: u16) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let byte_rate = sample_rate * channels as u32 * bits_per_sample as u32 / 8;
    let block_align = channels * bits_per_sample / 8;
    let file_size = 36 + data_len;

    let mut buf = Vec::with_capacity(44 + pcm.len());
    // RIFF header
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    // fmt sub-chunk
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // sub-chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&bits_per_sample.to_le_bytes());
    // data sub-chunk
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    buf.extend_from_slice(pcm);
    buf
}

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/internal/sessions/{id}/audio/mixed",
        get(mixed_audio),
    )
}
