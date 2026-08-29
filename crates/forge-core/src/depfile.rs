pub fn parse(text: &str) -> Vec<String> {
	let mut inputs = Vec::new();
	let mut token = String::new();
	let mut dependencies = false;
	let mut chars = text.chars().peekable();
	while let Some(mut ch) = chars.next() {
		if ch == '\\' {
			let mut count = 1;
			while chars.peek() == Some(&'\\') {
				chars.next();
				count += 1;
			}
			if chars
				.peek()
				.is_some_and(|c| matches!(c, ' ' | '\t' | '\r' | '\n' | '#' | ':'))
			{
				token.extend(std::iter::repeat_n('\\', count / 2));
				if count % 2 == 1 {
					ch = chars.next().unwrap();
					if ch == '\r' && chars.peek() == Some(&'\n') {
						chars.next();
					}
					if matches!(ch, '\r' | '\n') {
						finish(&mut token, dependencies, &mut inputs);
					} else {
						token.push(ch);
					}
				}
			} else {
				token.extend(std::iter::repeat_n('\\', count));
			}
			continue;
		}
		match ch {
			':' if !dependencies => {
				token.clear();
				dependencies = true;
			}
			'#' => {
				for next in chars.by_ref() {
					if next == '\n' {
						break;
					}
				}
				finish(&mut token, dependencies, &mut inputs);
				dependencies = false;
			}
			'\n' | '\r' => {
				finish(&mut token, dependencies, &mut inputs);
				dependencies = false;
			}
			' ' | '\t' => finish(&mut token, dependencies, &mut inputs),
			'$' => {
				if chars.peek() == Some(&'$') {
					chars.next();
				}
				token.push('$');
			}
			_ => token.push(ch),
		}
	}
	finish(&mut token, dependencies, &mut inputs);
	inputs
}

fn finish(token: &mut String, dependencies: bool, inputs: &mut Vec<String>) {
	if dependencies && !token.is_empty() {
		inputs.push(std::mem::take(token));
	} else {
		token.clear();
	}
}

#[cfg(test)]
mod tests {
	use super::parse;

	#[test]
	fn sources_and_every_dependency_extension_are_preserved() {
		assert_eq!(
			parse("out.o: main.c api.h data.strange extensionless"),
			["main.c", "api.h", "data.strange", "extensionless"]
		);
	}

	#[test]
	fn compiler_escaping() {
		assert_eq!(
			parse(r"out: space\ name.weird hash\#name dollar$$name back\slash double\\slash colon\:name"),
			[
				"space name.weird",
				"hash#name",
				"dollar$name",
				r"back\slash",
				r"double\\slash",
				"colon:name"
			]
		);
		assert_eq!(
			parse("out: trailing\\\\ next trailing\\\\\\ space # ignored\n"),
			["trailing\\", "next", "trailing\\ space"]
		);
	}

	#[test]
	fn continuations_targets_and_phony_rules() {
		assert_eq!(
			parse("one.o two.o: main.c \\\n api.h \\\r\n config\napi.h:\nconfig:\nother: second.c\n"),
			["main.c", "api.h", "config", "second.c"]
		);
		assert!(parse("empty:\n# comment\nno rule").is_empty());
	}
}
