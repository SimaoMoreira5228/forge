use rhai::{Engine, MultiInputsStream, Position, Token, TokenIterator, TokenizeState, TokenizerControlBlock};

pub fn tokenize(engine: &Engine, text: &str) -> Result<Vec<(Token, Position)>, String> {
	let control: rhai::TokenizerControl = std::rc::Rc::new(std::cell::RefCell::new(TokenizerControlBlock::new()));
	let mut iter = TokenIterator {
		engine,
		state: TokenizeState {
			tokenizer_control: control.clone(),
			include_comments: true,
			..Default::default()
		},
		pos: Position::new(1, 0),
		stream: MultiInputsStream {
			buf: [None, None],
			streams: std::iter::once(text.chars().peekable()).collect(),
			index: 0,
		},
		token_mapper: None,
	};
	let mut tokens = Vec::new();
	let mut holes: Vec<()> = Vec::new();
	let mut pending_hole = false;
	for (token, pos) in &mut iter {
		if pending_hole {
			pending_hole = false;
			if !matches!(token, Token::LeftBrace) {
				return Err(format!("interpolated string hole does not open with `{{` at {pos:?}"));
			}
			holes.push(());
			tokens.push((token, pos));
			continue;
		}
		let done = matches!(token, Token::EOF);
		if matches!(token, Token::InterpolatedString(_)) {
			tokens.push((token, pos));
			pending_hole = true;
			continue;
		}
		if matches!(token, Token::LexError(_)) {
			tokens.push((token, pos));
			break;
		}
		if !holes.is_empty() {
			match token {
				Token::LeftBrace | Token::LeftParen | Token::LeftBracket | Token::MapStart => {
					holes.push(());
				}
				Token::RightBrace | Token::RightParen | Token::RightBracket => {
					holes.pop();
					if holes.is_empty() {
						control.borrow_mut().is_within_text = true;
					}
				}
				_ => (),
			}
		}
		tokens.push((token, pos));
		if done {
			break;
		}
	}
	if pending_hole {
		return Err("interpolated string hole never opens".into());
	}
	Ok(drop_empty_spans(text, tokens))
}

fn drop_empty_spans(text: &str, tokens: Vec<(Token, Position)>) -> Vec<(Token, Position)> {
	let bounds = token_bounds(text, &tokens);
	let mut kept = Vec::with_capacity(tokens.len());
	for (i, (token, pos)) in tokens.into_iter().enumerate() {
		if matches!(token, Token::EOF) {
			kept.push((token, pos));
			continue;
		}
		if matches!(token, Token::InterpolatedString(_)) {
			kept.push((token, pos));
			continue;
		}
		let from = bounds[i];
		if trim_end(text, from, bounds[i + 1].max(from)) > from {
			kept.push((token, pos));
		}
	}
	kept
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Bracket {
	Map,
	Block,
	Hole,
	Paren,
	Square,
}

pub struct Meta {
	pub in_map: bool,
	pub in_hole: bool,
	pub hole_open: bool,
	pub hole_close: bool,
}

pub fn analyze(spanned: &[SpannedToken]) -> Vec<Meta> {
	let mut meta = Vec::with_capacity(spanned.len());
	for _ in spanned {
		meta.push(Meta {
			in_map: false,
			in_hole: false,
			hole_open: false,
			hole_close: false,
		});
	}
	let mut brackets: Vec<Bracket> = Vec::new();
	for (i, span) in spanned.iter().enumerate() {
		meta[i].in_map = matches!(brackets.last(), Some(Bracket::Map));
		meta[i].in_hole = brackets.iter().any(|b| matches!(b, Bracket::Hole));
		match span.token {
			rhai::Token::LeftBrace => {
				if i > 0 && matches!(spanned[i - 1].token, rhai::Token::InterpolatedString(_)) {
					meta[i].hole_open = true;
					brackets.push(Bracket::Hole);
				} else {
					brackets.push(Bracket::Block);
				}
			}
			rhai::Token::LeftParen => brackets.push(Bracket::Paren),
			rhai::Token::LeftBracket => brackets.push(Bracket::Square),
			rhai::Token::MapStart => brackets.push(Bracket::Map),
			rhai::Token::RightBrace | rhai::Token::RightParen | rhai::Token::RightBracket => {
				if matches!(brackets.pop(), Some(Bracket::Hole)) {
					meta[i].hole_close = true;
				}
			}
			_ => (),
		}
	}
	meta
}

pub struct LineTable {
	starts: Vec<usize>,
}

impl LineTable {
	pub fn new(text: &str) -> Self {
		let mut starts = vec![0];
		for (i, ch) in text.char_indices() {
			if ch == '\n' {
				starts.push(i + 1);
			}
		}
		Self { starts }
	}

	pub fn offset(&self, text: &str, line: usize, col: usize) -> Option<usize> {
		let base = *self.starts.get(line.saturating_sub(1))?;
		let end = text.get(base..)?.find('\n').map(|i| base + i).unwrap_or(text.len());
		let mut byte = base;
		for (count, (i, ch)) in text[base..end].char_indices().enumerate() {
			if count >= col {
				break;
			}
			byte = base + i + ch.len_utf8();
		}
		Some(byte)
	}
}

fn token_bounds(text: &str, tokens: &[(Token, Position)]) -> Vec<usize> {
	let table = LineTable::new(text);
	let mut bounds = Vec::with_capacity(tokens.len() + 1);
	for (token, pos) in tokens {
		if pos.is_none() {
			if matches!(token, Token::InterpolatedString(_)) {
				bounds.push(usize::MAX);
			} else {
				bounds.push(text.len());
			}
			continue;
		}
		let line = pos.line().unwrap_or(1);
		let col = pos.position().unwrap_or(1).saturating_sub(1);
		bounds.push(table.offset(text, line, col).unwrap_or(text.len()));
	}
	bounds.push(text.len());
	let mut next = text.len();
	for bound in bounds.iter_mut().rev() {
		if *bound == usize::MAX {
			*bound = next;
		} else {
			next = *bound;
		}
	}
	bounds
}

fn trim_end(text: &str, from: usize, mut to: usize) -> usize {
	while to > from {
		match text[..to].chars().next_back() {
			Some(c) if c == ' ' || c == '\t' || c == '\r' || c == '\n' => {
				to -= c.len_utf8();
			}
			_ => break,
		}
	}
	to
}

pub struct SpannedToken<'a> {
	pub token: &'a Token,
	pub line: usize,
	pub text: &'a str,
	pub newlines_before: usize,
	pub end: usize,
}

pub fn span_tokens<'a>(text: &'a str, tokens: &'a [(Token, Position)]) -> Vec<SpannedToken<'a>> {
	let bounds = token_bounds(text, tokens);
	let mut out = Vec::with_capacity(tokens.len());
	let mut cursor = 0;
	for (i, (token, pos)) in tokens.iter().enumerate() {
		let from = bounds[i].max(cursor);
		let to = trim_end(text, from, bounds[i + 1].max(from));
		let span_text = text.get(from..to).unwrap_or("");
		let newlines_before = text.get(cursor..from).map(|g| g.matches('\n').count()).unwrap_or(0);
		cursor = from + span_text.len();
		out.push(SpannedToken {
			token,
			line: pos.line().unwrap_or(1),
			text: span_text,
			newlines_before,
			end: cursor,
		});
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	fn corpus_files() -> Vec<std::path::PathBuf> {
		let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prelude");
		let mut files = Vec::new();
		let mut stack = vec![root];
		while let Some(top) = stack.pop() {
			for entry in std::fs::read_dir(&top).unwrap() {
				let entry = entry.unwrap();
				let path = entry.path();
				if entry.file_type().unwrap().is_dir() {
					stack.push(path);
				} else if path.extension().is_some_and(|ext| ext == "rhai") {
					files.push(path);
				}
			}
		}
		files.sort();
		files
	}

	#[test]
	fn token_stream_covers_corpus() {
		let engine = Engine::new();
		let files = corpus_files();
		assert!(!files.is_empty());
		let mut map_starts = 0;
		for path in &files {
			let text = std::fs::read_to_string(path).unwrap();
			let tokens = tokenize(&engine, &text).expect("tokenize test input");
			assert!(!tokens.is_empty(), "{}", path.display());
			assert!(
				matches!(tokens.last(), Some((Token::EOF, _))),
				"{} does not end at EOF",
				path.display()
			);
			for (token, _) in &tokens {
				match token {
					Token::MapStart => map_starts += 1,
					Token::LexError(err) => panic!("{}: {err}", path.display()),
					_ => (),
				}
			}
		}
		assert!(map_starts > 0, "expected #{{ tokens in corpus");
	}

	#[test]
	fn token_stream_yields_positioned_comments() {
		let engine = Engine::new();
		let text = "// leading\nlet x = 1; // trailing\n/* block\nspanning */\nlet y = `a ${x + 1} b`;\n";
		let tokens = tokenize(&engine, text).expect("tokenize test input");
		let comments: Vec<(String, usize)> = tokens
			.iter()
			.filter_map(|(token, pos)| match token {
				Token::Comment(comment) => Some((comment.to_string(), pos.line().unwrap_or(0))),
				_ => None,
			})
			.collect();
		assert_eq!(comments.len(), 3, "{comments:?}");
		assert!(comments[0].1 == 1, "{comments:?}");
		assert!(comments[1].1 == 2, "{comments:?}");
		assert!(comments[2].1 == 3, "{comments:?}");
		assert!(matches!(tokens.last(), Some((Token::EOF, _))));
	}

	#[test]
	fn token_spans_slice_exact_source_text() {
		let engine = Engine::new();
		let text = "let x = 1; // hi\nm(`a ${x} b`);\n";
		let tokens = tokenize(&engine, text).expect("tokenize test input");
		let spanned = span_tokens(text, &tokens);
		let texts: Vec<&str> = spanned.iter().map(|s| s.text).collect();
		assert_eq!(
			texts,
			vec![
				"let", "x", "=", "1", ";", "// hi", "m", "(", "`a $", "{", "x", "}", " b`", ")", ";", ""
			],
			"{texts:?}"
		);
		let mut rebuilt = String::new();
		let mut cursor = 0;
		for span in &spanned {
			let start = text[cursor..]
				.find(span.text)
				.map(|i| cursor + i)
				.filter(|_| !span.text.is_empty());
			if let Some(from) = start {
				rebuilt.push_str(&text[cursor..from]);
				rebuilt.push_str(span.text);
				cursor = from + span.text.len();
			}
		}
		rebuilt.push_str(&text[cursor..]);
		assert_eq!(rebuilt, text);
	}

	#[test]
	fn token_spans_round_trip_corpus() {
		let engine = Engine::new();
		for path in corpus_files() {
			let text = std::fs::read_to_string(&path).unwrap();
			let tokens = tokenize(&engine, &text).expect("tokenize test input");
			let spanned = span_tokens(&text, &tokens);
			let mut rebuilt = String::new();
			let mut cursor = 0;
			for span in &spanned {
				if span.text.is_empty() {
					continue;
				}
				let rest = &text[cursor..];
				let at = rest
					.find(span.text)
					.unwrap_or_else(|| panic!("{}: span {:?} not found after {cursor}", path.display(), span.text));
				rebuilt.push_str(&rest[..at]);
				rebuilt.push_str(span.text);
				cursor += at + span.text.len();
			}
			rebuilt.push_str(&text[cursor..]);
			assert_eq!(rebuilt, text, "round trip failed for {}", path.display());
			for (i, span) in spanned.iter().enumerate() {
				if matches!(span.token, Token::EOF) {
					assert_eq!(span.text, "", "{}", path.display());
				} else if tokens[i].1.is_none() {
					assert_eq!(span.text, "", "{}: position-less non-empty span", path.display());
				} else {
					assert!(
						!span.text.is_empty(),
						"{}: empty span at {i}: {:?} at {:?}",
						path.display(),
						span.token,
						tokens[i].1
					);
				}
			}
		}
	}

	#[test]
	fn token_stream_shapes_for_maps_blocks_and_shebang() {
		let engine = Engine::new();
		let text = "#!/usr/bin/env forge\nlet m = #{ a: 1 };\nif true { print(m); }\n/* outer /* inner */ still */\n";
		let tokens = tokenize(&engine, text).expect("tokenize test input");
		let kinds: Vec<String> = tokens
			.iter()
			.map(|(token, _)| match token {
				Token::Reserved(_) => "reserved".to_string(),
				Token::MapStart => "mapstart".to_string(),
				Token::LeftBrace => "{".to_string(),
				Token::RightBrace => "}".to_string(),
				Token::Comment(_) => "comment".to_string(),
				Token::EOF => "eof".to_string(),
				_ => "other".to_string(),
			})
			.collect();
		assert!(kinds.contains(&"reserved".to_string()), "{kinds:?}");
		assert!(kinds.contains(&"mapstart".to_string()), "{kinds:?}");
		assert!(kinds.iter().filter(|k| k.as_str() == "{").count() == 1, "{kinds:?}");
		assert!(kinds.iter().filter(|k| k.as_str() == "comment").count() == 1, "{kinds:?}");
		assert!(matches!(tokens.last(), Some((Token::EOF, _))));
	}
}
