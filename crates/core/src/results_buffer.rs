use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct ResultsRingBuffer<T> {
    capacity: usize,
    rows: VecDeque<T>,
    total_rows_seen: u64,
}

impl<T> ResultsRingBuffer<T> {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(
            capacity > 0,
            "results buffer capacity must be greater than 0"
        );
        Self {
            capacity,
            rows: VecDeque::with_capacity(capacity),
            total_rows_seen: 0,
        }
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    #[must_use]
    pub fn total_rows_seen(&self) -> u64 {
        self.total_rows_seen
    }

    #[must_use]
    pub fn earliest_buffered_index(&self) -> u64 {
        self.total_rows_seen.saturating_sub(self.rows.len() as u64)
    }

    #[must_use]
    pub fn latest_buffered_index(&self) -> Option<u64> {
        if self.rows.is_empty() {
            return None;
        }
        Some(self.total_rows_seen - 1)
    }

    pub fn push(&mut self, row: T) {
        if self.rows.len() == self.capacity {
            self.rows.pop_front();
        }
        self.rows.push_back(row);
        self.total_rows_seen += 1;
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        self.rows.get(index)
    }

    #[must_use]
    pub fn visible_rows(&self, start: usize, limit: usize) -> Vec<&T> {
        if limit == 0 || start >= self.rows.len() {
            return Vec::new();
        }

        let end = (start + limit).min(self.rows.len());
        self.rows
            .iter()
            .skip(start)
            .take(end - start)
            .collect::<Vec<_>>()
    }
}

#[cfg(test)]
mod tests {
    use super::ResultsRingBuffer;

    #[test]
    fn keeps_memory_bounded_to_capacity() {
        let mut buffer = ResultsRingBuffer::new(3);
        buffer.push("r1");
        buffer.push("r2");
        buffer.push("r3");
        buffer.push("r4");

        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.total_rows_seen(), 4);
        assert_eq!(buffer.get(0), Some(&"r2"));
        assert_eq!(buffer.get(1), Some(&"r3"));
        assert_eq!(buffer.get(2), Some(&"r4"));
    }

    #[test]
    fn visible_rows_returns_requested_window() {
        let mut buffer = ResultsRingBuffer::new(5);
        buffer.push(10);
        buffer.push(20);
        buffer.push(30);
        buffer.push(40);

        let rows = buffer.visible_rows(1, 2);
        assert_eq!(rows, vec![&20, &30]);
    }

    #[test]
    fn index_metadata_tracks_stream_position() {
        let mut buffer = ResultsRingBuffer::new(2);
        buffer.push("a");
        buffer.push("b");
        buffer.push("c");

        assert_eq!(buffer.total_rows_seen(), 3);
        assert_eq!(buffer.earliest_buffered_index(), 1);
        assert_eq!(buffer.latest_buffered_index(), Some(2));
    }
}
