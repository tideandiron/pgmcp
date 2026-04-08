// src/streaming/mod.rs
//
// Streaming serialization layer for pgmcp.
// Items here are consumed by the query tool handler (feat/018).
// Dead-code lint fires until the query tool integrates this layer.
#![allow(dead_code)]
//
// This module converts tokio-postgres row collections into the output format
// (JSON or CSV) requested by the query tool caller.
//
// Components:
// - `BatchSizer` — adaptive batch size calculator
// - `json`       — JSON row encoder with OID-specific fast paths
// - `csv`        — CSV row encoder

pub mod csv;
pub mod json;

// ── BatchSizer ────────────────────────────────────────────────────────────────

/// Target encoded bytes per batch event (64 KiB).
///
/// Matches the write buffer pre-allocation size and the target SSE event size.
pub const TARGET_EVENT_BYTES: usize = 65_536;

/// Initial batch size before any measurements.
pub const INITIAL_BATCH_SIZE: usize = 100;

/// Minimum batch size; never go below 1 row.
pub const MIN_BATCH_SIZE: usize = 1;

/// Maximum batch size; cap at 1000 rows to prevent unbounded memory use.
pub const MAX_BATCH_SIZE: usize = 1_000;

/// Adaptive batch-size calculator per the design spec section 3.2.
///
/// Computes how many rows to include in each "batch" (logical group) based on
/// observed average row size. The goal is to keep each batch close to
/// [`TARGET_EVENT_BYTES`] (64 KiB), minimising both memory pressure (too many
/// rows per batch) and overhead (too few rows per batch).
///
/// # Adaptation algorithm
///
/// 1. First batch size is always [`INITIAL_BATCH_SIZE`] (100 rows).
/// 2. After encoding each batch, call [`BatchSizer::record`] to supply the
///    actual encoded byte count for that batch.
/// 3. The sizer computes `avg_row_bytes = cumulative_bytes / cumulative_rows`.
/// 4. Next batch size = `clamp(TARGET_EVENT_BYTES / avg_row_bytes, 1, 1000)`.
/// 5. Reset by dropping and creating a new `BatchSizer` for each query.
///
/// # Example
///
/// ```rust,ignore
/// let mut sizer = BatchSizer::new();
/// let first_batch_rows = sizer.next_batch_size(); // 100
/// // ... encode rows ...
/// sizer.record(first_batch_rows, encoded_bytes);
/// let second_batch_rows = sizer.next_batch_size(); // adapted
/// ```
#[derive(Debug)]
pub struct BatchSizer {
    /// Cumulative encoded bytes across all recorded batches.
    total_bytes: usize,
    /// Cumulative row count across all recorded batches.
    total_rows: usize,
    /// Batch size to use for the next iteration.
    next_size: usize,
}

impl BatchSizer {
    /// Create a fresh `BatchSizer` for a new query.
    ///
    /// The first call to [`next_batch_size`](BatchSizer::next_batch_size)
    /// returns [`INITIAL_BATCH_SIZE`] (100).
    pub fn new() -> Self {
        Self {
            total_bytes: 0,
            total_rows: 0,
            next_size: INITIAL_BATCH_SIZE,
        }
    }

    /// Return the batch size to use for the next iteration.
    ///
    /// Before any calls to [`record`](BatchSizer::record), this returns
    /// [`INITIAL_BATCH_SIZE`] (100).
    #[inline]
    pub fn next_batch_size(&self) -> usize {
        self.next_size
    }

    /// Record the byte count for a completed batch and compute the next size.
    ///
    /// - `rows_in_batch`: number of rows encoded in this batch.
    /// - `bytes_encoded`: total bytes written for this batch (all rows, full JSON/CSV).
    ///
    /// After this call, [`next_batch_size`](BatchSizer::next_batch_size) returns
    /// the updated size.
    pub fn record(&mut self, rows_in_batch: usize, bytes_encoded: usize) {
        if rows_in_batch == 0 {
            return;
        }
        self.total_bytes += bytes_encoded;
        self.total_rows += rows_in_batch;

        let avg_row_bytes = self.total_bytes / self.total_rows;
        let avg_row_bytes = avg_row_bytes.max(1); // guard against zero

        let computed = TARGET_EVENT_BYTES / avg_row_bytes;
        self.next_size = computed.clamp(MIN_BATCH_SIZE, MAX_BATCH_SIZE);
    }
}

impl Default for BatchSizer {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_sizer_initial_size_is_100() {
        let sizer = BatchSizer::new();
        assert_eq!(sizer.next_batch_size(), INITIAL_BATCH_SIZE);
    }

    #[test]
    fn batch_sizer_adapts_after_first_batch_small_rows() {
        // 100 rows, each 64 bytes → avg_row_bytes = 64
        // next = clamp(65536 / 64, 1, 1000) = clamp(1024, 1, 1000) = 1000
        let mut sizer = BatchSizer::new();
        sizer.record(100, 6_400);
        assert_eq!(sizer.next_batch_size(), MAX_BATCH_SIZE);
    }

    #[test]
    fn batch_sizer_adapts_after_first_batch_large_rows() {
        // 100 rows, each 4096 bytes → avg_row_bytes = 4096
        // next = clamp(65536 / 4096, 1, 1000) = clamp(16, 1, 1000) = 16
        let mut sizer = BatchSizer::new();
        sizer.record(100, 409_600);
        assert_eq!(sizer.next_batch_size(), 16);
    }

    #[test]
    fn batch_sizer_clamps_to_min_for_enormous_rows() {
        // Single row of 1 MB → avg = 1MB
        // next = clamp(65536 / 1048576, 1, 1000) = clamp(0, 1, 1000) = 1
        let mut sizer = BatchSizer::new();
        sizer.record(1, 1_048_576);
        assert_eq!(sizer.next_batch_size(), MIN_BATCH_SIZE);
    }

    #[test]
    fn batch_sizer_clamps_to_max_for_tiny_rows() {
        // 100 rows of 1 byte each → avg = 1
        // next = clamp(65536 / 1, 1, 1000) = 1000
        let mut sizer = BatchSizer::new();
        sizer.record(100, 100);
        assert_eq!(sizer.next_batch_size(), MAX_BATCH_SIZE);
    }

    #[test]
    fn batch_sizer_record_zero_rows_is_no_op() {
        let mut sizer = BatchSizer::new();
        sizer.record(0, 1000);
        // State unchanged — next_size is still initial.
        assert_eq!(sizer.next_batch_size(), INITIAL_BATCH_SIZE);
    }

    #[test]
    fn batch_sizer_accumulates_across_batches() {
        // Batch 1: 100 rows, 1000 bytes → avg so far = 10 bytes/row
        // → next = clamp(65536 / 10, 1, 1000) = 1000
        let mut sizer = BatchSizer::new();
        sizer.record(100, 1_000);
        assert_eq!(sizer.next_batch_size(), MAX_BATCH_SIZE);

        // Batch 2: 1000 rows, 500_000 bytes → cumulative = 1100 rows, 501_000 bytes
        // avg = 501000 / 1100 = 455 bytes/row
        // next = clamp(65536 / 455, 1, 1000) = clamp(144, 1, 1000) = 144
        sizer.record(1_000, 500_000);
        let next = sizer.next_batch_size();
        assert!(
            (MIN_BATCH_SIZE..=MAX_BATCH_SIZE).contains(&next),
            "next_size {next} out of range"
        );
        assert!(
            next < MAX_BATCH_SIZE,
            "sizer should adapt downward for large rows"
        );
    }

    #[test]
    fn batch_sizer_default_equals_new() {
        let default = BatchSizer::default();
        let new = BatchSizer::new();
        assert_eq!(default.next_batch_size(), new.next_batch_size());
    }

    #[test]
    fn batch_sizer_target_event_bytes_is_64k() {
        assert_eq!(TARGET_EVENT_BYTES, 65_536);
    }

    #[test]
    fn batch_sizer_max_is_1000() {
        assert_eq!(MAX_BATCH_SIZE, 1_000);
    }

    #[test]
    fn batch_sizer_min_is_1() {
        assert_eq!(MIN_BATCH_SIZE, 1);
    }
}
