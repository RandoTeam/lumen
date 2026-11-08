//! Stop-string matching shared by the mock and llama.cpp backends.
//!
//! A stop string may be split across token boundaries, so the stream keeps the
//! last `max(len(stop)) - 1` characters back until it knows whether they start
//! a stop sequence. The text handed to the sink therefore never contains a
//! partial stop string, and the concatenation of all emitted pieces is the
//! visible completion.

/// Holds back the tail of the stream so a stop string split across tokens is
/// never emitted. `generated` accumulates the whole completion; `emitted` is
/// the byte offset already sent to the sink.
#[derive(Debug)]
pub(crate) struct StopBuffer {
    stops: Vec<String>,
    keep_chars: usize,
    generated: String,
    emitted: usize,
    hit: bool,
}

impl StopBuffer {
    pub(crate) fn new(stops: &[String]) -> Self {
        let stops: Vec<String> = stops
            .iter()
            .filter(|stop| !stop.is_empty())
            .cloned()
            .collect();
        let longest = stops
            .iter()
            .map(|stop| stop.chars().count())
            .max()
            .unwrap_or(0);
        Self {
            stops,
            keep_chars: longest.saturating_sub(1),
            generated: String::new(),
            emitted: 0,
            hit: false,
        }
    }

    pub(crate) fn hit(&self) -> bool {
        self.hit
    }

    /// Append one token and return the text that is now safe to emit.
    pub(crate) fn push(&mut self, token: &str) -> String {
        if self.hit {
            return String::new();
        }
        self.generated.push_str(token);

        if let Some(index) = self.find_stop() {
            self.hit = true;
            let end = index.max(self.emitted);
            return self.take(end);
        }

        let safe_end = safe_prefix_end(&self.generated, self.keep_chars);
        if safe_end > self.emitted {
            self.take(safe_end)
        } else {
            String::new()
        }
    }

    /// Flush the held tail at the end of a stream. Returns nothing when a stop
    /// string was hit.
    pub(crate) fn finish(&mut self) -> String {
        if self.hit {
            return String::new();
        }
        let end = self.generated.len();
        if end > self.emitted {
            self.take(end)
        } else {
            String::new()
        }
    }

    fn take(&mut self, end: usize) -> String {
        let end = end.min(self.generated.len());
        if end <= self.emitted {
            return String::new();
        }
        let text = self.generated[self.emitted..end].to_string();
        self.emitted = end;
        text
    }

    fn find_stop(&self) -> Option<usize> {
        self.stops
            .iter()
            .filter_map(|stop| self.generated.find(stop))
            .min()
    }
}

/// Byte index that leaves exactly `keep_chars` trailing characters, always on
/// a UTF-8 boundary.
fn safe_prefix_end(text: &str, keep_chars: usize) -> usize {
    if keep_chars == 0 {
        return text.len();
    }
    let total = text.chars().count();
    if total <= keep_chars {
        return 0;
    }
    text.char_indices()
        .nth(total - keep_chars)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_buffer_without_stops_emits_everything() {
        let mut buffer = StopBuffer::new(&[]);
        assert_eq!(buffer.push("alpha"), "alpha");
        assert_eq!(buffer.push(" beta"), " beta");
        assert_eq!(buffer.finish(), "");
        assert!(!buffer.hit());
    }

    #[test]
    fn stop_buffer_withholds_tail_and_truncates_at_stop() {
        let mut buffer = StopBuffer::new(&[" gamma".to_string()]);
        assert_eq!(buffer.push("alpha"), "");
        assert_eq!(buffer.push(" beta"), "alpha");
        assert_eq!(buffer.push(" gamma"), " beta");
        assert!(buffer.hit());
        assert_eq!(buffer.finish(), "");
    }

    #[test]
    fn stop_buffer_finds_stop_spanning_tokens() {
        let mut buffer = StopBuffer::new(&[" beta gamma".to_string()]);
        assert_eq!(buffer.push("alpha"), "");
        assert_eq!(buffer.push(" beta"), "");
        assert_eq!(buffer.push(" gamma"), "alpha");
        assert!(buffer.hit());
    }

    #[test]
    fn stop_buffer_stops_at_start() {
        let mut buffer = StopBuffer::new(&["alpha".to_string()]);
        assert_eq!(buffer.push("alpha"), "");
        assert!(buffer.hit());
        assert_eq!(buffer.finish(), "");
    }

    #[test]
    fn stop_buffer_keeps_utf8_boundaries() {
        let mut buffer = StopBuffer::new(&[" мир".to_string()]);
        assert_eq!(buffer.push("Привет"), "При");
        assert_eq!(buffer.push(" мир"), "вет");
        assert!(buffer.hit());
    }

    #[test]
    fn stop_buffer_flushes_tail_at_end() {
        let mut buffer = StopBuffer::new(&["zzz".to_string()]);
        assert_eq!(buffer.push("he"), "");
        assert_eq!(buffer.finish(), "he");
    }
}
