//! Width-aware text helpers: display width, exact-width fitting, span
//! clipping, byte and count formatting.

use ratatui::style::Style;
use ratatui::text::Span;

// 1. display width ------------------------------------------------------------
pub fn text_width(text: &str) -> usize {
    Span::raw(text).width()
}

// Wide glyphs (Hangul, CJK) take two cells; control characters take none.
pub fn char_width(letter: char) -> usize {
    if letter.is_ascii() {
        return usize::from(!letter.is_ascii_control());
    }
    let mut buffer = [0u8; 4];
    Span::raw(&*letter.encode_utf8(&mut buffer)).width()
}

// 2. exact-width fitting ------------------------------------------------------
// Exactly `width` cells: end-truncated with one `…` when too long, space padded
// when short, control characters turned into spaces.
pub fn fit_display(text: &str, width: usize) -> String {
    let total = text_width(text);
    let limit = if total > width {
        width.saturating_sub(1)
    } else {
        width
    };
    let mut fitted = String::new();
    let mut used = 0usize;
    for letter in text.chars() {
        let shown = if letter.is_control() { ' ' } else { letter };
        let cells = char_width(shown);
        if used + cells > limit {
            break;
        }
        fitted.push(shown);
        used += cells;
    }
    if total > width && width > 0 {
        fitted.push('…');
        used += 1;
    }
    fitted.push_str(&" ".repeat(width.saturating_sub(used)));
    fitted
}

// Spans cut to `width` cells with one trailing `…` carrying the last style;
// text that already fits is returned unchanged.
pub fn clip_spans_to_width(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let total: usize = spans.iter().map(Span::width).sum();
    if total <= width {
        return spans;
    }
    if width == 0 {
        return Vec::new();
    }
    let limit = width - 1;
    let mut clipped: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    let mut last_style = Style::default();
    for span in spans {
        last_style = span.style;
        let mut kept = String::new();
        let mut cut = false;
        for letter in span.content.chars() {
            let cells = char_width(letter);
            if used + cells > limit {
                cut = true;
                break;
            }
            kept.push(letter);
            used += cells;
        }
        if !kept.is_empty() {
            clipped.push(Span::styled(kept, span.style));
        }
        if cut {
            break;
        }
    }
    clipped.push(Span::styled("…", last_style));
    clipped
}

// 3. numbers ------------------------------------------------------------------
pub fn fmt_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let value = bytes as f64;
    if value >= KIB * KIB * KIB {
        format!("{:.1}G", value / (KIB * KIB * KIB))
    } else if value >= KIB * KIB {
        format!("{:.1}M", value / (KIB * KIB))
    } else if value >= KIB {
        format!("{:.1}K", value / KIB)
    } else {
        format!("{bytes}B")
    }
}

// Thousands separators: 1234567 -> 1,234,567.
pub fn fmt_count(count: u64) -> String {
    let digits = count.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

// Milliseconds since the epoch as `YYYY-MM-DD HH:MM:SS` (UTC); empty when out of range.
pub fn fmt_timestamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|stamp| stamp.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

// 4. ids ----------------------------------------------------------------------
// FNV-1a hash of a text key: a stable row id that survives re-sorting.
pub fn stable_id(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hangul_takes_two_cells_and_control_takes_none() {
        assert_eq!(text_width("도구"), 4);
        assert_eq!(char_width('한'), 2);
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('\n'), 0);
    }

    #[test]
    fn fit_display_pads_and_truncates_by_display_width() {
        assert_eq!(fit_display("abc", 5), "abc  ");
        assert_eq!(fit_display("도구목록", 6), "도구… ");
        assert_eq!(fit_display("abcdef", 4), "abc…");
        assert_eq!(fit_display("a\tb", 3), "a b");
        assert_eq!(text_width(&fit_display("도구목록", 7)), 7);
    }

    #[test]
    fn clip_spans_keeps_short_text_and_ends_long_text_with_one_ellipsis() {
        let spans = vec![Span::raw("abc"), Span::raw("def")];
        assert_eq!(clip_spans_to_width(spans.clone(), 6), spans);
        let clipped = clip_spans_to_width(spans, 4);
        let text: String = clipped.iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(text, "abc…");
        assert!(clip_spans_to_width(vec![Span::raw("abc")], 0).is_empty());
    }

    #[test]
    fn stable_id_is_deterministic_and_separates_different_keys() {
        assert_eq!(stable_id("10.0.0.1:443"), stable_id("10.0.0.1:443"));
        assert_ne!(stable_id("10.0.0.1:443"), stable_id("10.0.0.2:443"));
    }

    #[test]
    fn timestamps_render_in_utc_and_out_of_range_is_empty() {
        assert_eq!(fmt_timestamp(0), "1970-01-01 00:00:00");
        assert_eq!(fmt_timestamp(1_700_000_000_000), "2023-11-14 22:13:20");
        assert_eq!(fmt_timestamp(i64::MAX), "");
    }

    #[test]
    fn byte_and_count_formats() {
        assert_eq!(fmt_bytes(512), "512B");
        assert_eq!(fmt_bytes(2048), "2.0K");
        assert_eq!(fmt_bytes(3 * 1024 * 1024), "3.0M");
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1_234_567), "1,234,567");
    }
}
