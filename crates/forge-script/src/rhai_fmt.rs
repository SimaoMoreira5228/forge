use forge_diagnostics::ForgeDiagnostic;

use crate::rhai_tokens::{span_tokens, tokenize};

pub fn canonical(text: &str) -> Result<String, ForgeDiagnostic> {
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);
	engine.set_max_call_levels(128);
	engine
		.compile(text)
		.map_err(|e| ForgeDiagnostic::error(101, format!("invalid Rhai: {e}")))?;
	let tokens = tokenize(&engine, text).map_err(|e| ForgeDiagnostic::error(101, e))?;
	for (token, _) in &tokens {
		if let rhai::Token::LexError(err) = token {
			return Err(ForgeDiagnostic::error(101, format!("invalid Rhai: {err}")));
		}
	}
	let spanned = span_tokens(text, &tokens);
	let meta = crate::rhai_tokens::analyze(&spanned);
	let (emits, dropped) = crate::rhai_layout::lay_out(text, &spanned);
	let mut out = String::new();
	for emit in &emits {
		match emit {
			crate::rhai_layout::Emit::Verbatim { text } => {
				out.push_str(text);
				out.push('\n');
			}
			crate::rhai_layout::Emit::Tokens { indent, tokens } => {
				if tokens.is_empty() {
					out.push('\n');
					continue;
				}
				for _ in 0..*indent {
					out.push('\t');
				}
				out.push_str(&crate::rhai_spacing::render_tokens(tokens, &spanned, &meta, &dropped));
				out.push('\n');
			}
		}
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn fmt(text: &str) -> String {
		canonical(text).unwrap()
	}

	#[test]
	fn spacing_operators_calls_and_maps() {
		assert_eq!(fmt("let x=1+2*3;\n"), "let x = 1 + 2 * 3;\n");
		assert_eq!(fmt("f( x,y );\n"), "f(x, y);\n");
		assert_eq!(fmt("let m=#{a:1,b : 2};\n"), "let m = #{ a: 1, b: 2 };\n");
		assert_eq!(fmt("let m=#{};\n"), "let m = #{};\n");
		assert_eq!(fmt("a*-b;\n"), "a * -b;\n");
		assert_eq!(fmt("if(x){y();}\n"), "if (x) { y(); }\n");
		assert_eq!(fmt("x??y;\n"), "x ?? y;\n");
		assert_eq!(fmt("let x=a&&(b||c);\n"), "let x = a && (b || c);\n");
		assert_eq!(fmt("fn f(x){return x;}\n"), "fn f(x) { return x; }\n");
	}

	#[test]
	fn spacing_preserves_lines_strings_and_comments() {
		assert_eq!(
			fmt("let s = \"a  b\"; // keep  spaces\n"),
			"let s = \"a  b\"; // keep  spaces\n"
		);
		assert_eq!(fmt("let x=1;//c\nlet y=2;\n"), "let x = 1; //c\nlet y = 2;\n");
		assert_eq!(fmt("fn f( ) {\n\treturn 1;\n}\n"), "fn f() {\n\treturn 1;\n}\n");
		assert_eq!(fmt("let a=1; /* keep */ let b=2;\n"), "let a = 1; /* keep */ let b = 2;\n");
	}

	#[test]
	fn layout_indents_splits_and_collapses() {
		assert_eq!(fmt("let a=1;let b=2;\n"), "let a = 1;\nlet b = 2;\n");
		assert_eq!(fmt("fn f() {\n\n\nlet x=1;\n\n\n}\n"), "fn f() {\n\tlet x = 1;\n}\n");
		assert_eq!(
			fmt("if x {\n\ty();\n} else {\n\tz();\n}\n"),
			"if x {\n\ty();\n} else {\n\tz();\n}\n"
		);
		assert_eq!(fmt("//! doc\n\nfn f() {}\n"), "//! doc\nfn f() {}\n");
		assert_eq!(
			fmt("let x = 1;\n// forge-fmt: off\nlet   y=2;\n// forge-fmt: on\nlet z=3;\n"),
			"let x = 1;\n// forge-fmt: off\nlet   y=2;\n// forge-fmt: on\nlet z = 3;\n"
		);
	}

	#[test]
	fn spacing_rejects_invalid_input() {
		assert!(canonical("let x = ;\n").is_err());
		assert!(canonical("fn (;\n").is_err());
		assert!(canonical("#!/usr/bin/env forge\nlet x = 1;\n").is_err());
		assert!(canonical("let héllo = 1;\n").is_err());
	}

	#[test]
	fn layout_joins_short_blocks() {
		assert_eq!(fmt("if !x { return \"\";\n}\n"), "if !x { return \"\"; }\n");
		assert_eq!(fmt("fn f() {\n}\n"), "fn f() {}\n");
		assert_eq!(fmt("while x>0{x-=1;}\n"), "while x > 0 { x -= 1; }\n");
		assert_eq!(fmt("if(x){y();z();}\n"), "if (x) { y(); z(); }\n");
		assert_eq!(fmt("if a { b();\n} else { c();\n}\n"), "if a { b(); } else { c(); }\n");
		assert_eq!(
			fmt("switch x { 1 => `one`, _ => `other`, }\n"),
			"switch x { 1 => `one`, _ => `other` }\n"
		);
	}

	#[test]
	fn layout_indents_operator_continuations() {
		assert_eq!(
			fmt("fn f(src) {\n\tsrc.ends_with(\".cpp\")\n\t|| is_module(src);\n}\n"),
			"fn f(src) {\n\tsrc.ends_with(\".cpp\")\n\t\t|| is_module(src);\n}\n"
		);
	}

	#[test]
	fn layout_keeps_kr_and_long_blocks_split() {
		assert_eq!(fmt("if x {\n\ty();\n}\n"), "if x {\n\ty();\n}\n");
		assert_eq!(
			fmt("fn f() {\n\tlet x = 1;\n\tlet y = 2;\n}\n"),
			"fn f() {\n\tlet x = 1;\n\tlet y = 2;\n}\n"
		);
		let long = format!("if x {{ y(\"{}\"); }}\n", "a".repeat(120));
		assert_eq!(fmt(&long), long);
	}

	#[test]
	fn consecutive_holes_stay_glued() {
		let cases = [
			"let n = `${kind}${tail.split(\"(\")[0]}`;\n",
			"let s = `a${x}b${y}c`;\n",
			"let s = `${x}`;\n",
			"throw `none of ${a}, ${b}, ${c}`;\n",
		];
		for text in cases {
			let once = fmt(text);
			assert_eq!(once, text, "hole damaged for {text:?}");
			assert_eq!(fmt(&once), once, "not idempotent for {text:?}");
		}
	}

	#[test]
	fn adversarial_fixtures() {
		let cases = [
			("let x = 1;\r\nlet y = 2;\r\n", "let x = 1;\nlet y = 2;\n"),
			("let x = 1;", "let x = 1;\n"),
			("", ""),
			("let x = 1;   \n", "let x = 1;\n"),
			("fn f() {\n        let x = 1;\n}\n", "fn f() {\n\tlet x = 1;\n}\n"),
			("/* a /* b */ c */\nlet x=1;\n", "/* a /* b */ c */\nlet x = 1;\n"),
			(
				"// =====\n// banner\n// =====\nlet x=1;\n",
				"// =====\n// banner\n// =====\nlet x = 1;\n",
			),
			("let s = `a ${m[x]+f(1,2)} b`;\n", "let s = `a ${m[x] + f(1, 2)} b`;\n"),
			(
				"let m=#{a:1};\nif x{d(#{b:2});}\n",
				"let m = #{ a: 1 };\nif x { d(#{ b: 2 }); }\n",
			),
		];
		for (input, expected) in cases {
			let once = fmt(input);
			assert_eq!(once, expected, "wrong output for {input:?}");
			assert_eq!(fmt(&once), once, "not idempotent for {input:?}");
		}
	}

	#[test]
	fn deeply_nested_blocks_do_not_overflow() {
		let mut text = String::new();
		for _ in 0..30 {
			text.push_str("if true {\n");
		}
		text.push_str("let x = 1;\n");
		for _ in 0..30 {
			text.push_str("}\n");
		}
		let once = canonical(&text).unwrap();
		assert_eq!(canonical(&once).unwrap(), once);
	}

	#[test]
	fn generated_file_formats_linearly() {
		let mut text = String::new();
		for i in 0..3000 {
			text.push_str(&format!("let var{i:04} = {i} + {i};\n"));
		}
		let start = std::time::Instant::now();
		let once = canonical(&text).unwrap();
		assert!(start.elapsed().as_secs() < 60, "formatter looks quadratic");
		assert_eq!(canonical(&once).unwrap(), once);
	}

	#[test]
	fn spacing_preserves_token_stream() {
		let engine = rhai::Engine::new();
		let kinds = |tokens: &[(rhai::Token, rhai::Position)]| {
			let mut out = Vec::new();
			let list: Vec<&rhai::Token> = tokens.iter().map(|(token, _)| token).collect();
			for (i, token) in list.iter().enumerate() {
				if matches!(token, rhai::Token::Comma)
					&& matches!(
						list.get(i + 1),
						Some(rhai::Token::RightBrace | rhai::Token::RightParen | rhai::Token::RightBracket)
					) {
					continue;
				}
				out.push(format!("{token:?}"));
			}
			out
		};
		let cases = [
			"let x=1+2*3;\n",
			"f( x,y );\n",
			"let m=#{a:1,b : 2};\n",
			"if(x){y();}\n",
			"let s = `a ${x+1} b ${`c`} d`;\n",
			"// lead\nlet x=1;//trail\n/* a /* b */ c */\n",
			"while x>0{x-=1;}\n",
			"switch x { 1 => `one`, _ => `other`, }\n",
			"f(a, b,);\n",
			"f(\n\ta,\n\tb,\n);\n",
		];
		for text in cases {
			let before = kinds(&crate::rhai_tokens::tokenize(&engine, text).expect("tokenize fixture"));
			let once = canonical(text).unwrap();
			let after = kinds(&crate::rhai_tokens::tokenize(&engine, &once).expect("tokenize output"));
			assert_eq!(before, after, "token stream changed for {text:?} -> {once:?}");
			assert_eq!(canonical(&once).unwrap(), once);
		}
		assert_eq!(fmt("f(a, b,);\n"), "f(a, b);\n");
	}

	#[test]
	fn spacing_is_idempotent_on_fixtures() {
		let cases = [
			"let x=1+2*3;\n",
			"fn f( ) {\n\treturn 1;\n}\n",
			"let m=#{a:1,b : 2};\n",
			"if(x){y();z();}\n",
			"let s = `a ${x+1} b`;\n",
			"switch x { 1 => `one`, _ => `other`, }\n",
		];
		for text in cases {
			let once = canonical(text).unwrap();
			assert_eq!(canonical(&once).unwrap(), once, "not idempotent: {text:?}");
		}
	}
}
