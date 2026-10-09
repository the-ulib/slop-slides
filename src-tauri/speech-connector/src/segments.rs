/// Split at sentence/paragraph boundaries where possible, then whitespace, preserving all text.
pub fn segments(text: &str) -> Vec<String> {
    let mut remaining = text.trim();
    let mut chunks = Vec::new();
    while !remaining.is_empty() {
        let end = remaining
            .char_indices()
            .nth(350)
            .map_or(remaining.len(), |(i, _)| i);
        let prefix = &remaining[..end];
        let split = if end == remaining.len() {
            end
        } else {
            prefix
                .char_indices()
                .filter(|(i, c)| *i > prefix.len() / 3 && matches!(c, '.' | '!' | '?' | '\n'))
                .map(|(i, c)| i + c.len_utf8())
                .next_back()
                .or_else(|| {
                    prefix
                        .char_indices()
                        .filter(|(_, c)| c.is_whitespace())
                        .map(|(i, _)| i)
                        .next_back()
                })
                .unwrap_or(end)
        };
        let (chunk, rest) = remaining.split_at(split);
        if !chunk.trim().is_empty() {
            chunks.push(chunk.trim().into());
        }
        remaining = rest.trim();
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn segments_preserve_multibyte_text_and_bound_long_words() {
        let text = ("Grüße. Präsentationen sind schön! ".repeat(45)) + &"ä".repeat(800);
        let chunks = segments(&text);
        assert!(chunks.len() > 5);
        assert!(chunks.iter().all(|s| s.chars().count() <= 350));
        assert_eq!(
            chunks
                .join("")
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>(),
            text.chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
        );
    }
}
