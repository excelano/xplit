// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

const ALT_ARROW_PREFIX: &[u8] = b"\x1b[1;3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrow {
    Up,
    Down,
    Right,
    Left,
}

impl Arrow {
    fn from_final_byte(byte: u8) -> Option<Arrow> {
        match byte {
            b'A' => Some(Arrow::Up),
            b'B' => Some(Arrow::Down),
            b'C' => Some(Arrow::Right),
            b'D' => Some(Arrow::Left),
            _ => None,
        }
    }

    pub fn bytes(self) -> &'static [u8] {
        match self {
            Arrow::Up => b"\x1b[1;3A",
            Arrow::Down => b"\x1b[1;3B",
            Arrow::Right => b"\x1b[1;3C",
            Arrow::Left => b"\x1b[1;3D",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Token {
    Bytes(Vec<u8>),
    AltArrow(Arrow),
}

#[derive(Default)]
pub struct Scanner {
    pending: Vec<u8>,
}

impl Scanner {
    pub fn feed(&mut self, input: &[u8]) -> Vec<Token> {
        let mut buf = std::mem::take(&mut self.pending);
        buf.extend_from_slice(input);

        let mut tokens = Vec::new();
        let mut run_start = 0;
        let mut i = 0;
        while i < buf.len() {
            if buf[i] == 0x1b {
                let rest = &buf[i..];
                if rest.len() > ALT_ARROW_PREFIX.len() && rest.starts_with(ALT_ARROW_PREFIX) {
                    if let Some(arrow) = Arrow::from_final_byte(rest[ALT_ARROW_PREFIX.len()]) {
                        push_bytes(&mut tokens, &buf[run_start..i]);
                        tokens.push(Token::AltArrow(arrow));
                        i += ALT_ARROW_PREFIX.len() + 1;
                        run_start = i;
                        continue;
                    }
                } else if ALT_ARROW_PREFIX.starts_with(rest) {
                    push_bytes(&mut tokens, &buf[run_start..i]);
                    self.pending = rest.to_vec();
                    return tokens;
                }
            }
            i += 1;
        }
        push_bytes(&mut tokens, &buf[run_start..]);
        tokens
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn flush(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending)
    }
}

fn push_bytes(tokens: &mut Vec<Token>, bytes: &[u8]) {
    if !bytes.is_empty() {
        tokens.push(Token::Bytes(bytes.to_vec()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reassemble(tokens: &[Token]) -> Vec<u8> {
        tokens
            .iter()
            .flat_map(|token| match token {
                Token::Bytes(bytes) => bytes.clone(),
                Token::AltArrow(arrow) => arrow.bytes().to_vec(),
            })
            .collect()
    }

    fn arrows(tokens: &[Token]) -> Vec<Arrow> {
        tokens
            .iter()
            .filter_map(|token| match token {
                Token::AltArrow(arrow) => Some(*arrow),
                Token::Bytes(_) => None,
            })
            .collect()
    }

    fn feed_chunks(chunks: &[&[u8]]) -> (Vec<Token>, Vec<u8>) {
        let mut scanner = Scanner::default();
        let tokens = chunks
            .iter()
            .flat_map(|chunk| scanner.feed(chunk))
            .collect();
        (tokens, scanner.flush())
    }

    #[test]
    fn plain_text_passes_through_in_one_token() {
        let mut scanner = Scanner::default();
        assert_eq!(
            scanner.feed(b"ls -la\r"),
            vec![Token::Bytes(b"ls -la\r".to_vec())]
        );
        assert!(!scanner.has_pending());
    }

    #[test]
    fn recognizes_each_alt_arrow() {
        for arrow in [Arrow::Up, Arrow::Down, Arrow::Right, Arrow::Left] {
            let mut scanner = Scanner::default();
            assert_eq!(scanner.feed(arrow.bytes()), vec![Token::AltArrow(arrow)]);
        }
    }

    #[test]
    fn alt_arrow_between_text() {
        let mut scanner = Scanner::default();
        assert_eq!(
            scanner.feed(b"ab\x1b[1;3Dcd\x1b[1;3C"),
            vec![
                Token::Bytes(b"ab".to_vec()),
                Token::AltArrow(Arrow::Left),
                Token::Bytes(b"cd".to_vec()),
                Token::AltArrow(Arrow::Right),
            ]
        );
    }

    #[test]
    fn sequence_split_at_every_point() {
        let input = b"x\x1b[1;3Ay";
        for split in 0..=input.len() {
            let (tokens, leftover) = feed_chunks(&[&input[..split], &input[split..]]);
            assert_eq!(arrows(&tokens), vec![Arrow::Up], "split at {split}");
            assert_eq!(reassemble(&tokens), input, "split at {split}");
            assert!(leftover.is_empty());
        }
    }

    #[test]
    fn sequence_split_one_byte_per_read() {
        let input = b"\x1b[1;3B";
        let chunks: Vec<&[u8]> = input.chunks(1).collect();
        let (tokens, leftover) = feed_chunks(&chunks);
        assert_eq!(tokens, vec![Token::AltArrow(Arrow::Down)]);
        assert!(leftover.is_empty());
    }

    #[test]
    fn lone_esc_is_held_then_flushed() {
        let mut scanner = Scanner::default();
        assert!(scanner.feed(b"\x1b").is_empty());
        assert!(scanner.has_pending());
        assert_eq!(scanner.flush(), b"\x1b");
        assert!(!scanner.has_pending());
    }

    #[test]
    fn partial_prefix_followed_by_other_bytes_passes_through() {
        let (tokens, leftover) = feed_chunks(&[b"\x1b[1;", b"5D"]);
        assert_eq!(reassemble(&tokens), b"\x1b[1;5D");
        assert!(arrows(&tokens).is_empty());
        assert!(leftover.is_empty());
    }

    #[test]
    fn esc_followed_by_text_is_not_held() {
        let mut scanner = Scanner::default();
        assert_eq!(
            scanner.feed(b"\x1b:wq\r"),
            vec![Token::Bytes(b"\x1b:wq\r".to_vec())]
        );
        assert!(!scanner.has_pending());
    }

    #[test]
    fn similar_sequences_pass_through() {
        let similar: &[&[u8]] = &[
            b"\x1b[A",
            b"\x1bOA",
            b"\x1b[1;5D",
            b"\x1b[1;2C",
            b"\x1b[1;3E",
            b"\x1b[1;3~",
            b"\x1b[1;33D",
            b"\x1b\x1b[D",
            b"\x1b[200~text\x1b[201~",
        ];
        for input in similar {
            let (tokens, leftover) = feed_chunks(&[input]);
            assert!(arrows(&tokens).is_empty(), "{input:?}");
            assert_eq!(reassemble(&tokens), *input, "{input:?}");
            assert!(leftover.is_empty(), "{input:?}");
        }
    }

    #[test]
    fn meta_esc_before_alt_arrow() {
        let mut scanner = Scanner::default();
        assert_eq!(
            scanner.feed(b"\x1b\x1b[1;3D"),
            vec![Token::Bytes(b"\x1b".to_vec()), Token::AltArrow(Arrow::Left)]
        );
    }

    #[test]
    fn every_byte_value_passes_through() {
        let all: Vec<u8> = (0..=255).collect();
        let (tokens, leftover) = feed_chunks(&[&all]);
        assert_eq!(reassemble(&tokens), all);
        assert!(leftover.is_empty());
    }

    #[test]
    fn control_keys_pass_through() {
        for byte in [0x03, 0x04, 0x1a, b'\t'] {
            let mut scanner = Scanner::default();
            assert_eq!(scanner.feed(&[byte]), vec![Token::Bytes(vec![byte])]);
        }
    }

    #[test]
    fn utf8_passes_through() {
        let text = "héllo → 世界".as_bytes();
        let (tokens, leftover) = feed_chunks(&[&text[..3], &text[3..]]);
        assert_eq!(reassemble(&tokens), text);
        assert!(leftover.is_empty());
    }
}
