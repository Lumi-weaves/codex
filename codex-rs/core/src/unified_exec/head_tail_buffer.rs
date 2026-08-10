use crate::unified_exec::UNIFIED_EXEC_OUTPUT_MAX_BYTES;
use crate::unified_exec::format_output_omission_marker;
use std::collections::VecDeque;

/// A capped buffer that preserves a stable prefix ("head") and suffix ("tail"),
/// dropping the middle once it exceeds the configured maximum. The buffer is
/// symmetric meaning 50% of the capacity is allocated to the head and 50% is
/// allocated to the tail.
#[derive(Debug)]
#[cfg_attr(test, derive(Eq, PartialEq))]
pub(crate) struct HeadTailBuffer {
    max_bytes: usize,
    head_budget: usize,
    tail_budget: usize,
    head: Vec<u8>,
    tail: VecDeque<u8>,
    omitted_bytes: usize,
}

impl Default for HeadTailBuffer {
    fn default() -> Self {
        Self::new(UNIFIED_EXEC_OUTPUT_MAX_BYTES)
    }
}

impl HeadTailBuffer {
    /// Create a new buffer that retains at most `max_bytes` of output.
    ///
    /// The retained output is split across a prefix ("head") and suffix ("tail")
    /// budget, dropping bytes from the middle once the limit is exceeded.
    pub(crate) fn new(max_bytes: usize) -> Self {
        let head_budget = max_bytes / 2;
        let tail_budget = max_bytes.saturating_sub(head_budget);
        Self {
            max_bytes,
            head_budget,
            tail_budget,
            head: Vec::new(),
            tail: VecDeque::new(),
            omitted_bytes: 0,
        }
    }

    // Used for tests.
    #[allow(dead_code)]
    /// Total bytes currently retained by the buffer (head + tail).
    pub(crate) fn retained_bytes(&self) -> usize {
        self.head.len().saturating_add(self.tail.len())
    }

    // Used for tests.
    #[allow(dead_code)]
    /// Total bytes that were dropped from the middle due to the size cap.
    pub(crate) fn omitted_bytes(&self) -> usize {
        self.omitted_bytes
    }

    /// Total bytes observed by the buffer, including bytes omitted by the cap.
    pub(crate) fn total_bytes(&self) -> usize {
        self.retained_bytes().saturating_add(self.omitted_bytes)
    }

    /// Append a chunk of bytes to the buffer.
    ///
    /// Bytes are first added to the head until the head budget is full; any
    /// remaining bytes are added to the tail, with older tail bytes being
    /// dropped to preserve the tail budget.
    pub(crate) fn push_chunk(&mut self, chunk: Vec<u8>) {
        if chunk.is_empty() {
            return;
        }
        if self.max_bytes == 0 {
            self.omitted_bytes = self.omitted_bytes.saturating_add(chunk.len());
            return;
        }

        // Fill the head budget first, then keep a capped tail.
        let remaining_head = self.head_budget.saturating_sub(self.head.len());
        let head_len = remaining_head.min(chunk.len());
        if head_len > 0 {
            self.head.extend_from_slice(&chunk[..head_len]);
        }
        self.push_to_tail(&chunk[head_len..]);
    }

    /// Snapshot the retained output as a list of chunks.
    ///
    /// The returned chunks are ordered as: head chunks first, then tail chunks.
    /// Omitted bytes are not represented in the snapshot.
    pub(crate) fn snapshot_chunks(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::with_capacity(2);
        if !self.head.is_empty() {
            out.push(self.head.clone());
        }
        if !self.tail.is_empty() {
            out.push(self.tail.iter().copied().collect());
        }
        out
    }

    /// Return the retained output as a single byte vector.
    ///
    /// The output is formed by concatenating head chunks, then tail chunks.
    /// Omitted bytes are not represented in the returned value.
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.retained_bytes());
        out.extend_from_slice(&self.head);
        out.extend(self.tail.iter().copied());
        out
    }

    /// Return the retained output with an explicit marker between the head and
    /// tail when bytes were omitted.
    pub(crate) fn to_bytes_with_omission_marker(&self) -> Vec<u8> {
        if self.omitted_bytes == 0 {
            return self.to_bytes();
        }

        let marker = format_output_omission_marker(self.omitted_bytes);
        let marker_delimiter_bytes = 2;
        let mut out = Vec::with_capacity(
            self.retained_bytes()
                .saturating_add(marker.len())
                .saturating_add(marker_delimiter_bytes),
        );
        out.extend_from_slice(&self.head);
        out.push(b'\n');
        out.extend_from_slice(marker.as_bytes());
        out.push(b'\n');
        out.extend(self.tail.iter().copied());
        out
    }

    /// Return a bounded head/tail excerpt of the retained output.
    ///
    /// The excerpt keeps a true prefix and a true most-recent suffix of the
    /// logical retained bytes, at most `max_bytes` total. Whenever either the
    /// transcript already omitted bytes or this excerpt trims content, one
    /// bounded omission marker is inserted between the head and tail. The
    /// returned omitted count is `total_bytes - retained_output_content_bytes`
    /// (saturating): everything observed minus the content bytes the excerpt
    /// retains. This keeps model-visible completion fragments small while
    /// preserving the most recent output. Returns
    /// `(excerpt_bytes, omitted_output_bytes)`.
    pub(crate) fn bounded_excerpt(&self, max_bytes: usize) -> (Vec<u8>, usize) {
        if max_bytes == 0 {
            return (Vec::new(), self.total_bytes());
        }
        if self.omitted_bytes == 0 && self.retained_bytes() <= max_bytes {
            return (self.to_bytes(), 0);
        }

        // One bounded omission marker covers any omission (prior transcript
        // drops plus excerpt trimming). Size it pessimistically against the
        // worst possible omitted count so the excerpt can never exceed the
        // cap even when trimming changes the count.
        let marker_max_len =
            format!("\n{}\n", format_output_omission_marker(self.total_bytes())).len();
        let content_budget = max_bytes.saturating_sub(marker_max_len);
        let head_budget = content_budget / 2;
        let tail_budget = content_budget.saturating_sub(head_budget);

        // Select a non-overlapping prefix and suffix from the LOGICAL retained
        // byte sequence (head followed by tail), independent of how those
        // bytes happen to be distributed across the internal halves. This
        // keeps the true most-recent suffix even when the whole transcript
        // lives in the head half (e.g. a 4 KiB transcript in the 1 MiB store).
        let retained = self.to_bytes();
        let prefix_len = head_budget.min(retained.len());
        // Reserve the suffix so the prefix and suffix never overlap.
        let suffix_len = tail_budget.min(retained.len().saturating_sub(prefix_len));
        let suffix_start = retained.len().saturating_sub(suffix_len);
        let content_kept = prefix_len.saturating_add(suffix_len);

        // Omitted output is everything observed minus the content bytes the
        // excerpt actually retains (saturating; includes prior transcript
        // omissions and any excerpt trimming).
        let omitted = self.total_bytes().saturating_sub(content_kept);
        let marker = if omitted > 0 {
            format!("\n{}\n", format_output_omission_marker(omitted))
        } else {
            String::new()
        };
        let mut out = Vec::with_capacity(content_kept.saturating_add(marker.len()));
        out.extend_from_slice(&retained[..prefix_len]);
        out.extend_from_slice(marker.as_bytes());
        out.extend_from_slice(&retained[suffix_start..]);
        if out.len() > max_bytes {
            // Marker-only edge case (cap smaller than the marker itself).
            out.truncate(max_bytes);
        }
        debug_assert!(
            out.len() <= max_bytes,
            "bounded excerpt exceeded its cap: {} > {}",
            out.len(),
            max_bytes
        );
        (out, omitted)
    }

    /// Drain the retained output and omission metadata, resetting this buffer's
    /// contents while preserving its configured capacity.
    pub(crate) fn drain(&mut self) -> Self {
        Self {
            max_bytes: self.max_bytes,
            head_budget: self.head_budget,
            tail_budget: self.tail_budget,
            head: std::mem::take(&mut self.head),
            tail: std::mem::take(&mut self.tail),
            omitted_bytes: std::mem::take(&mut self.omitted_bytes),
        }
    }

    /// Append retained output from another buffer and preserve any omissions it
    /// already recorded.
    pub(crate) fn push_buffer(&mut self, mut buffer: Self) {
        self.push_chunk(std::mem::take(&mut buffer.head));
        self.push_chunk(buffer.tail.drain(..).collect());
        self.omitted_bytes = self.omitted_bytes.saturating_add(buffer.omitted_bytes);
    }

    fn push_to_tail(&mut self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        if self.tail_budget == 0 {
            self.omitted_bytes = self.omitted_bytes.saturating_add(chunk.len());
            return;
        }

        if chunk.len() >= self.tail_budget {
            // This single chunk is larger than the whole tail budget. Keep only the last
            // tail_budget bytes and drop everything else.
            let start = chunk.len().saturating_sub(self.tail_budget);
            let kept = &chunk[start..];
            let dropped = chunk.len().saturating_sub(kept.len());
            self.omitted_bytes = self
                .omitted_bytes
                .saturating_add(self.tail.len())
                .saturating_add(dropped);
            self.tail.clear();
            self.tail.extend(kept);
            return;
        }

        self.tail.extend(chunk);
        self.trim_tail_to_budget();
    }

    fn trim_tail_to_budget(&mut self) {
        let excess = self.tail.len().saturating_sub(self.tail_budget);
        if excess > 0 {
            drop(self.tail.drain(..excess));
            self.omitted_bytes = self.omitted_bytes.saturating_add(excess);
        }
    }
}

#[cfg(test)]
#[path = "head_tail_buffer_tests.rs"]
mod tests;
