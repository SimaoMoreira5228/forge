use crate::rhai_tokens::{Bracket, Meta, SpannedToken};

pub enum Emit<'a> {
	Tokens { indent: usize, tokens: Vec<usize> },
	Verbatim { text: &'a str },
}

fn is_shebang(text: &str) -> bool {
	text.find('\n').map(|i| &text[..i]).unwrap_or(text).starts_with("#!")
}

fn fence_marker(comment: &str) -> Option<bool> {
	match comment.trim() {
		"// forge-fmt: off" => Some(true),
		"// forge-fmt: on" => Some(false),
		_ => None,
	}
}

fn is_doc_comment(token: &rhai::Token) -> bool {
	matches!(token, rhai::Token::Comment(c) if c.starts_with("//!") || c.starts_with("///"))
}

fn split_semi(tokens: &[usize], spanned: &[SpannedToken], meta: &[Meta]) -> Vec<Vec<usize>> {
	let mut pieces = Vec::new();
	let mut current = Vec::new();
	let mut brackets: Vec<Bracket> = Vec::new();
	for (k, index) in tokens.iter().enumerate() {
		let span = &spanned[*index];
		current.push(*index);
		match span.token {
			rhai::Token::LeftBrace | rhai::Token::LeftParen | rhai::Token::LeftBracket | rhai::Token::MapStart => {
				brackets.push(Bracket::Block)
			}
			rhai::Token::RightBrace | rhai::Token::RightParen | rhai::Token::RightBracket => {
				brackets.pop();
			}
			rhai::Token::SemiColon => {
				if meta[*index].in_hole {
					continue;
				}
				let code_follows = tokens[k + 1..].iter().any(|i| {
					!matches!(spanned[*i].token, rhai::Token::Comment(_))
						&& !matches!(
							spanned[*i].token,
							rhai::Token::RightBrace | rhai::Token::RightParen | rhai::Token::RightBracket
						)
				});
				if code_follows && matches!(brackets.last(), None | Some(Bracket::Block)) {
					pieces.push(std::mem::take(&mut current));
				}
			}
			_ => (),
		}
	}
	if !current.is_empty() {
		pieces.push(current);
	}
	let mut merged: Vec<Vec<usize>> = Vec::new();
	for piece in pieces {
		let starts_with_comment = piece
			.first()
			.is_some_and(|i| matches!(spanned[*i].token, rhai::Token::Comment(_)));
		if starts_with_comment && let Some(last) = merged.last_mut() {
			last.extend(piece);
			continue;
		}
		if !piece.is_empty() {
			merged.push(piece);
		}
	}
	if merged.is_empty() {
		merged.push(Vec::new());
	}
	merged
}

struct Piece {
	tokens: Vec<usize>,
	verbatim: Option<usize>,
	doc: bool,
	gap: usize,
}

fn match_brackets(spanned: &[SpannedToken]) -> Vec<Option<usize>> {
	let mut matches = vec![None; spanned.len()];
	let mut stack: Vec<(usize, Bracket)> = Vec::new();
	for (i, span) in spanned.iter().enumerate() {
		match span.token {
			rhai::Token::LeftBrace | rhai::Token::LeftParen | rhai::Token::LeftBracket | rhai::Token::MapStart => {
				stack.push((
					i,
					match span.token {
						rhai::Token::LeftBrace => Bracket::Block,
						rhai::Token::LeftParen => Bracket::Paren,
						rhai::Token::LeftBracket => Bracket::Square,
						_ => Bracket::Map,
					},
				));
			}
			rhai::Token::RightBrace | rhai::Token::RightParen | rhai::Token::RightBracket => {
				if let Some((open, _)) = stack.pop() {
					matches[open] = Some(i);
					matches[i] = Some(open);
				}
			}
			_ => (),
		}
	}
	matches
}

fn assign_indents(pieces: &[Piece], spanned: &[SpannedToken]) -> Vec<usize> {
	assign_indents_full(pieces, spanned).0
}

fn assign_indents_full(pieces: &[Piece], spanned: &[SpannedToken]) -> (Vec<usize>, std::collections::HashMap<usize, usize>) {
	let mut indents = Vec::with_capacity(pieces.len());
	let mut token_indents = std::collections::HashMap::new();
	let mut curly: usize = 0;
	let mut parens: Vec<Bracket> = Vec::new();
	for piece in pieces {
		if piece.tokens.is_empty() {
			indents.push(0);
			continue;
		}
		for index in &piece.tokens {
			let mut indent = curly;
			if parens.iter().any(|b| matches!(b, Bracket::Paren | Bracket::Square)) {
				indent += 1;
			}
			token_indents.insert(*index, indent);
			match spanned[*index].token {
				rhai::Token::LeftBrace | rhai::Token::MapStart => curly += 1,
				rhai::Token::RightBrace => curly = curly.saturating_sub(1),
				rhai::Token::LeftParen | rhai::Token::LeftBracket => parens.push(Bracket::Paren),
				rhai::Token::RightParen | rhai::Token::RightBracket => {
					parens.pop();
				}
				_ => (),
			}
		}
		let first = piece.tokens[0];
		let mut indent = token_indents[&first];
		if matches!(
			spanned[first].token,
			rhai::Token::RightBrace | rhai::Token::RightParen | rhai::Token::RightBracket
		) {
			indent = indent.saturating_sub(1);
		}
		if indent == curly
			&& (crate::rhai_spacing::is_binary_op(spanned[first].token)
				|| crate::rhai_spacing::is_unary_prefix(spanned[first].token)
				|| crate::rhai_spacing::is_glued(spanned[first].token))
		{
			indent += 1;
		}
		indents.push(indent);
	}
	(indents, token_indents)
}

fn starts_close(piece: &Piece, spanned: &[SpannedToken]) -> bool {
	piece.tokens.first().is_some_and(|i| {
		matches!(
			spanned[*i].token,
			rhai::Token::RightBrace | rhai::Token::RightParen | rhai::Token::RightBracket
		)
	})
}

fn ends_open(piece: &Piece, spanned: &[SpannedToken]) -> bool {
	piece.tokens.last().is_some_and(|i| {
		matches!(
			spanned[*i].token,
			rhai::Token::LeftBrace | rhai::Token::MapStart | rhai::Token::LeftParen | rhai::Token::LeftBracket
		)
	})
}

pub fn lay_out<'a>(text: &'a str, spanned: &'a [SpannedToken<'a>]) -> (Vec<Emit<'a>>, std::collections::BTreeSet<usize>) {
	let meta = crate::rhai_tokens::analyze(spanned);
	let source: Vec<&str> = text.split('\n').collect();
	let at = |line: usize| source.get(line.saturating_sub(1)).copied().unwrap_or("");
	let mut by_line: Vec<Vec<usize>> = Vec::new();
	for (i, span) in spanned.iter().enumerate() {
		if matches!(span.token, rhai::Token::EOF) {
			continue;
		}
		while by_line.len() <= span.line {
			by_line.push(Vec::new());
		}
		by_line[span.line].push(i);
	}
	let shebang = is_shebang(text);
	let mut line_starts = vec![0];
	for (i, ch) in text.char_indices() {
		if ch == '\n' {
			line_starts.push(i + 1);
		}
	}
	let line_end = |line: usize| line_starts.get(line).copied().unwrap_or(text.len());
	let mut pieces: Vec<Piece> = Vec::new();
	let mut fenced = false;
	let mut last_kept_end = 0;
	for (number, group) in by_line.iter().enumerate() {
		if number == 0 {
			continue;
		}
		if shebang && number == 1 {
			pieces.push(Piece {
				tokens: group.clone(),
				verbatim: Some(number),
				doc: false,
				gap: 0,
			});
			last_kept_end = line_end(number);
			continue;
		}
		if group.is_empty() {
			if fenced {
				pieces.push(Piece {
					tokens: Vec::new(),
					verbatim: Some(number),
					doc: false,
					gap: 0,
				});
				last_kept_end = line_end(number);
			}
			continue;
		}
		if group.len() == 1
			&& let rhai::Token::Comment(comment) = &spanned[group[0]].token
			&& let Some(off) = fence_marker(comment)
		{
			fenced = off;
			pieces.push(Piece {
				tokens: group.clone(),
				verbatim: Some(number),
				doc: false,
				gap: 0,
			});
			last_kept_end = line_end(number);
			continue;
		}
		if fenced {
			pieces.push(Piece {
				tokens: group.clone(),
				verbatim: Some(number),
				doc: false,
				gap: 0,
			});
			last_kept_end = line_end(number);
			continue;
		}
		let mut first = true;
		for part in split_semi(group, spanned, &meta) {
			if part.is_empty() {
				continue;
			}
			let doc = part.len() == 1 && is_doc_comment(spanned[part[0]].token);
			let piece_gap = if first {
				let start = spanned[part[0]].end.saturating_sub(spanned[part[0]].text.len());
				text.get(last_kept_end..start).map(|g| g.matches('\n').count()).unwrap_or(0)
			} else {
				0
			};
			if let Some(last) = part.last() {
				last_kept_end = spanned[*last].end;
			}
			first = false;
			pieces.push(Piece {
				tokens: part,
				verbatim: None,
				doc,
				gap: piece_gap,
			});
		}
	}
	let matches = match_brackets(spanned);
	let mut dropped: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
	pack_pieces(spanned, &meta, &matches, &mut pieces, &mut dropped);
	let indents = assign_indents(&pieces, spanned);
	let mut emits = Vec::new();
	let mut prev_ends_open = false;
	let mut prev_is_doc = false;
	let mut emitted_anything = false;
	for (n, piece) in pieces.iter().enumerate() {
		if let Some(line) = piece.verbatim {
			emits.push(Emit::Verbatim { text: at(line) });
			prev_ends_open = false;
			prev_is_doc = false;
			emitted_anything = true;
			continue;
		}
		if piece.gap > 1 && emitted_anything && !(prev_is_doc || prev_ends_open || starts_close(piece, spanned)) {
			emits.push(Emit::Tokens {
				indent: 0,
				tokens: Vec::new(),
			});
		}
		emits.push(Emit::Tokens {
			indent: indents[n],
			tokens: piece.tokens.clone(),
		});
		prev_ends_open = ends_open(piece, spanned);
		prev_is_doc = piece.doc;
		emitted_anything = true;
	}
	(emits, dropped)
}

fn is_confined(
	open: usize,
	close: usize,
	open_piece: usize,
	close_piece: usize,
	pieces: &[Piece],
	dropped: &std::collections::BTreeSet<usize>,
) -> bool {
	for i in open + 1..close {
		if dropped.contains(&i) {
			continue;
		}
		match pieces.iter().position(|p| p.tokens.contains(&i)) {
			Some(p) if p == open_piece || p == close_piece => (),
			_ => return false,
		}
	}
	true
}

fn pack_pieces(
	spanned: &[SpannedToken],
	meta: &[crate::rhai_tokens::Meta],
	matches: &[Option<usize>],
	pieces: &mut Vec<Piece>,
	dropped: &mut std::collections::BTreeSet<usize>,
) {
	let mut pairs: Vec<(usize, usize)> = Vec::new();
	for (open, close) in matches.iter().enumerate() {
		if let Some(close) = close {
			if !matches!(
				spanned[open].token,
				rhai::Token::LeftParen | rhai::Token::LeftBracket | rhai::Token::MapStart | rhai::Token::LeftBrace
			) {
				continue;
			}
			if *close > open {
				pairs.push((open, *close));
			}
		}
	}
	pairs.sort_by_key(|(open, close)| close - open);
	let token_piece =
		|pieces: &[Piece], index: usize| -> Option<usize> { pieces.iter().position(|p| p.tokens.contains(&index)) };
	for (open, close) in pairs {
		let inner_clean = spanned
			.iter()
			.skip(open + 1)
			.take(close.saturating_sub(open + 1))
			.all(|span| !matches!(span.token, rhai::Token::Comment(_)) && !span.text.contains('\n'));
		if !inner_clean {
			continue;
		}
		let open_piece = match token_piece(pieces, open) {
			Some(p) => p,
			None => continue,
		};
		let close_piece = match token_piece(pieces, close) {
			Some(p) => p,
			None => continue,
		};
		if pieces[open_piece].verbatim.is_some()
			|| pieces[close_piece].verbatim.is_some()
			|| pieces[open_piece..=close_piece].iter().any(|p| p.verbatim.is_some())
		{
			continue;
		}
		let trailing_comma =
			(open + 1 < close) && matches!(spanned[close - 1].token, rhai::Token::Comma) && !dropped.contains(&(close - 1));
		if matches!(spanned[open].token, rhai::Token::LeftBrace)
			&& !is_confined(open, close, open_piece, close_piece, pieces, dropped)
		{
			continue;
		}
		let (_indents, token_indents) = assign_indents_full(pieces, spanned);
		let mut joined: Vec<usize> = (open..=close).filter(|i| !dropped.contains(i)).collect();
		if trailing_comma {
			joined.retain(|i| *i != close - 1);
		}
		let flat = crate::rhai_spacing::render_tokens(&joined, spanned, meta, dropped);
		let open_indent = token_indents.get(&open).copied().unwrap_or(0);
		let width = open_indent * crate::rhai_spacing::TAB_WIDTH + flat.len();
		if width <= crate::rhai_spacing::MAX_WIDTH {
			if trailing_comma {
				dropped.insert(close - 1);
			}
			if open_piece != close_piece {
				let mut merged = Vec::new();
				for piece in pieces.drain(open_piece..=close_piece) {
					merged.extend(piece.tokens);
				}
				let gap = spanned[merged[0]].newlines_before;
				merged.retain(|i| !dropped.contains(i));
				let doc = merged.len() == 1 && is_doc_comment(spanned[merged[0]].token);
				pieces.insert(
					open_piece,
					Piece {
						tokens: merged,
						verbatim: None,
						doc,
						gap,
					},
				);
			}
			continue;
		}
		if open_piece != close_piece {
			continue;
		}
		let mut segments: Vec<Vec<usize>> = vec![Vec::new()];
		let mut depth: usize = 0;
		for (i, span) in spanned.iter().enumerate().take(close).skip(open + 1) {
			if dropped.contains(&i) {
				continue;
			}
			match span.token {
				rhai::Token::LeftBrace | rhai::Token::LeftParen | rhai::Token::LeftBracket | rhai::Token::MapStart => {
					depth += 1
				}
				rhai::Token::RightBrace | rhai::Token::RightParen | rhai::Token::RightBracket => {
					depth = depth.saturating_sub(1)
				}
				rhai::Token::Comma if depth == 0 => {
					segments.last_mut().expect("segment").push(i);
					segments.push(Vec::new());
					continue;
				}
				_ => (),
			}
			segments.last_mut().expect("segment").push(i);
		}
		if segments.len() < 2 {
			continue;
		}
		if open_piece != close_piece {
			continue;
		}
		let mut rebuilt: Vec<Piece> = Vec::new();
		for piece in pieces.drain(..open_piece) {
			rebuilt.push(piece);
		}
		let mut range: Vec<Piece> = pieces.drain(..=close_piece - open_piece).collect();
		let head = range.remove(0);
		let head_tokens: Vec<usize> = head.tokens.iter().copied().filter(|i| *i <= open).collect();
		let gap = head_tokens.first().map(|i| spanned[*i].newlines_before).unwrap_or(0);
		rebuilt.push(Piece {
			tokens: head_tokens,
			verbatim: None,
			doc: false,
			gap,
		});
		for segment in segments {
			if segment.is_empty() {
				continue;
			}
			rebuilt.push(Piece {
				tokens: segment,
				verbatim: None,
				doc: false,
				gap: 0,
			});
		}
		let tail_tokens: Vec<usize> = head.tokens.iter().copied().filter(|i| *i >= close).collect();
		if !tail_tokens.is_empty() {
			rebuilt.push(Piece {
				tokens: tail_tokens,
				verbatim: None,
				doc: false,
				gap: 0,
			});
		}
		rebuilt.append(pieces);
		*pieces = rebuilt;
	}
}
