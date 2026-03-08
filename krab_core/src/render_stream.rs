use crate::Render;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspenseState {
    Pending,
    Resolved,
    Error,
}

impl SuspenseState {
    fn as_str(self) -> &'static str {
        match self {
            SuspenseState::Pending => "pending",
            SuspenseState::Resolved => "resolved",
            SuspenseState::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChunkedStreamWriter {
    chunk_size: usize,
    flush_threshold: usize,
    pending: String,
    chunks: Vec<String>,
    flush_count: usize,
}

impl Default for ChunkedStreamWriter {
    fn default() -> Self {
        Self::new(1024, 4096)
    }
}

impl ChunkedStreamWriter {
    pub fn new(chunk_size: usize, flush_threshold: usize) -> Self {
        Self {
            chunk_size: chunk_size.max(128),
            flush_threshold: flush_threshold.max(chunk_size.max(128)),
            pending: String::new(),
            chunks: Vec::new(),
            flush_count: 0,
        }
    }

    pub fn write(&mut self, input: &str) {
        self.pending.push_str(input);
        self.flush_if_ready();
    }

    pub fn write_suspense_marker(&mut self, boundary_id: &str, state: SuspenseState) {
        self.write(&format!(
            "<!--krab:suspense:{}:{}-->",
            boundary_id,
            state.as_str()
        ));
    }

    pub fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }

        while self.pending.len() > self.chunk_size {
            let split_at = nearest_char_boundary(&self.pending, self.chunk_size);
            let chunk = self.pending[..split_at].to_string();
            self.chunks.push(chunk);
            self.pending = self.pending[split_at..].to_string();
        }

        if !self.pending.is_empty() {
            self.chunks.push(std::mem::take(&mut self.pending));
        }

        self.flush_count += 1;
    }

    pub fn finish(mut self) -> Vec<String> {
        self.flush();
        self.chunks
    }

    pub fn flush_count(&self) -> usize {
        self.flush_count
    }

    fn flush_if_ready(&mut self) {
        if self.pending.len() >= self.flush_threshold {
            self.flush();
        }
    }
}

pub fn render_to_chunk_stream(renderable: &impl Render, writer: &mut ChunkedStreamWriter) {
    writer.write(&renderable.render());
}

fn nearest_char_boundary(s: &str, target: usize) -> usize {
    if target >= s.len() {
        return s.len();
    }
    let mut i = target;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_writer_splits_and_flushes() {
        let mut writer = ChunkedStreamWriter::new(128, 256);
        writer.write(&"a".repeat(300));

        let chunks = writer.finish();
        assert!(chunks.len() >= 3);
        assert_eq!(chunks.concat(), "a".repeat(300));
    }

    #[test]
    fn suspense_markers_are_hydration_compatible_comments() {
        let mut writer = ChunkedStreamWriter::new(32, 64);
        writer.write_suspense_marker("home-data", SuspenseState::Pending);
        writer.write("<div data-krab-hydration=\"home-data\">fallback</div>");
        writer.write_suspense_marker("home-data", SuspenseState::Resolved);
        let html = writer.finish().concat();

        assert!(html.contains("<!--krab:suspense:home-data:pending-->"));
        assert!(html.contains("data-krab-hydration=\"home-data\""));
        assert!(html.contains("<!--krab:suspense:home-data:resolved-->"));
    }

    #[test]
    fn backpressure_flushes_when_threshold_reached() {
        let mut writer = ChunkedStreamWriter::new(128, 128);
        writer.write("hello");
        assert_eq!(writer.flush_count(), 0);
        writer.write(&"x".repeat(130));
        assert!(writer.flush_count() >= 1);
    }
}
