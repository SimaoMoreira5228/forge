pub const MAX_WIDTH: usize = 100;
pub const TAB_WIDTH: usize = 4;

pub(crate) fn is_word(token: &rhai::Token) -> bool {
	if is_unary_prefix(token) {
		return false;
	}
	!matches!(
		token,
		rhai::Token::LeftBrace
			| rhai::Token::RightBrace
			| rhai::Token::LeftParen
			| rhai::Token::RightParen
			| rhai::Token::LeftBracket
			| rhai::Token::RightBracket
			| rhai::Token::Comma
			| rhai::Token::SemiColon
			| rhai::Token::Colon
			| rhai::Token::Comment(_)
			| rhai::Token::EOF
			| rhai::Token::LexError(_)
	)
}

pub(crate) fn is_binary_op(token: &rhai::Token) -> bool {
	matches!(
		token,
		rhai::Token::Plus
			| rhai::Token::Minus
			| rhai::Token::Multiply
			| rhai::Token::Divide
			| rhai::Token::Modulo
			| rhai::Token::PowerOf
			| rhai::Token::LeftShift
			| rhai::Token::RightShift
			| rhai::Token::Equals
			| rhai::Token::EqualsTo
			| rhai::Token::NotEqualsTo
			| rhai::Token::LessThan
			| rhai::Token::GreaterThan
			| rhai::Token::LessThanEqualsTo
			| rhai::Token::GreaterThanEqualsTo
			| rhai::Token::Pipe
			| rhai::Token::Or
			| rhai::Token::XOr
			| rhai::Token::Ampersand
			| rhai::Token::And
			| rhai::Token::In
			| rhai::Token::NotIn
			| rhai::Token::PlusAssign
			| rhai::Token::MinusAssign
			| rhai::Token::MultiplyAssign
			| rhai::Token::DivideAssign
			| rhai::Token::LeftShiftAssign
			| rhai::Token::RightShiftAssign
			| rhai::Token::AndAssign
			| rhai::Token::OrAssign
			| rhai::Token::XOrAssign
			| rhai::Token::ModuloAssign
			| rhai::Token::PowerOfAssign
			| rhai::Token::DoubleArrow
			| rhai::Token::DoubleQuestion
			| rhai::Token::Custom(_)
	)
}

pub(crate) fn is_indexable(token: &rhai::Token) -> bool {
	matches!(
		token,
		rhai::Token::Identifier(_)
			| rhai::Token::StringConstant(_)
			| rhai::Token::InterpolatedString(_)
			| rhai::Token::IntegerConstant(_)
			| rhai::Token::FloatConstant(_)
			| rhai::Token::CharConstant(_)
			| rhai::Token::True
			| rhai::Token::False
			| rhai::Token::Unit
	)
}

pub(crate) fn is_unary_prefix(token: &rhai::Token) -> bool {
	matches!(token, rhai::Token::UnaryPlus | rhai::Token::UnaryMinus | rhai::Token::Bang)
}

pub(crate) fn is_glued(token: &rhai::Token) -> bool {
	matches!(
		token,
		rhai::Token::Period
			| rhai::Token::Elvis
			| rhai::Token::QuestionBracket
			| rhai::Token::ExclusiveRange
			| rhai::Token::InclusiveRange
			| rhai::Token::DoubleColon
	)
}

pub(crate) fn gap_before(prev: Option<&rhai::Token>, next: &rhai::Token, in_map: bool) -> bool {
	let Some(prev) = prev else {
		return false;
	};
	if matches!(next, rhai::Token::Comma | rhai::Token::SemiColon) {
		return false;
	}
	if matches!(next, rhai::Token::RightParen | rhai::Token::RightBracket) {
		return false;
	}
	if matches!(next, rhai::Token::Unit) {
		return !(is_word(prev)
			|| matches!(
				prev,
				rhai::Token::RightParen | rhai::Token::RightBracket | rhai::Token::RightBrace
			));
	}
	if matches!(next, rhai::Token::RightBrace) {
		return !matches!(prev, rhai::Token::MapStart | rhai::Token::LeftBrace);
	}
	if matches!(prev, rhai::Token::LeftParen | rhai::Token::LeftBracket) {
		return false;
	}
	if matches!(prev, rhai::Token::Comment(_)) {
		return true;
	}
	if matches!(prev, rhai::Token::RightBrace) && is_word(next) {
		return true;
	}
	if matches!(prev, rhai::Token::LeftBrace | rhai::Token::MapStart) {
		return true;
	}
	if matches!(prev, rhai::Token::Comma | rhai::Token::SemiColon | rhai::Token::Colon) {
		return true;
	}
	if matches!(next, rhai::Token::LeftBrace) {
		return true;
	}
	if matches!(next, rhai::Token::LeftBracket) {
		return !matches!(
			prev,
			rhai::Token::LeftParen
				| rhai::Token::LeftBracket
				| rhai::Token::RightParen
				| rhai::Token::RightBracket
				| rhai::Token::RightBrace
		) && !is_indexable(prev);
	}
	if matches!(next, rhai::Token::LeftParen) {
		return !matches!(
			prev,
			rhai::Token::Identifier(_) | rhai::Token::RightParen | rhai::Token::RightBracket | rhai::Token::Fn
		);
	}
	if is_unary_prefix(next) {
		return !matches!(
			prev,
			rhai::Token::LeftParen
				| rhai::Token::LeftBracket
				| rhai::Token::MapStart
				| rhai::Token::Comma
				| rhai::Token::SemiColon
		) && !is_unary_prefix(prev);
	}
	if is_glued(next) || is_glued(prev) {
		return false;
	}
	if matches!(next, rhai::Token::Colon) {
		return !in_map;
	}
	if is_binary_op(next) || is_binary_op(prev) {
		return true;
	}
	if matches!(next, rhai::Token::Comment(_)) {
		return true;
	}
	is_word(prev) && is_word(next)
}

pub(crate) fn render_tokens(
	tokens: &[usize],
	spanned: &[crate::rhai_tokens::SpannedToken],
	meta: &[crate::rhai_tokens::Meta],
	dropped: &std::collections::BTreeSet<usize>,
) -> String {
	let mut out = String::new();
	let mut prev: Option<usize> = None;
	for index in tokens {
		if dropped.contains(index) {
			continue;
		}
		let span = &spanned[*index];
		let hole_touch = meta[*index].hole_open
			|| meta[*index].hole_close
			|| prev.is_some_and(|p| meta[p].hole_open)
			|| prev.is_some_and(|p| {
				meta[p].hole_close
					&& matches!(
						span.token,
						rhai::Token::StringConstant(_) | rhai::Token::InterpolatedString(_)
					)
			});
		if prev.is_some() && !hole_touch && gap_before(prev.map(|p| spanned[p].token), span.token, meta[*index].in_map) {
			out.push(' ');
		}
		out.push_str(span.text);
		prev = Some(*index);
	}
	out
}
