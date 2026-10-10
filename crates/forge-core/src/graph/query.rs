use std::collections::BTreeSet;

use forge_diagnostics::{ForgeDiagnostic, codes};

use super::build_graph::BuildGraph;
use super::component::{ComponentId, ComponentKind};

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
	Pattern(String),
	Kind { kind: String, of: Box<Expr> },
	Filter { pattern: String, of: Box<Expr> },
	Deps(Box<Expr>),
	Rdeps(Box<Expr>),
	Tests(Box<Expr>),
	Affected { file: String },
	SomePath { from: Box<Expr>, to: Box<Expr> },
	AllPaths { from: Box<Expr>, to: Box<Expr> },
	Union(Box<Expr>, Box<Expr>),
	Intersect(Box<Expr>, Box<Expr>),
	Difference(Box<Expr>, Box<Expr>),
}

pub fn parse(text: &str) -> Result<Expr, ForgeDiagnostic> {
	let tokens = tokenize(text)?;
	let mut parser = TokenStream { tokens, position: 0 };
	let expr = parser.parse_union()?;
	parser.expect_end()?;
	Ok(expr)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
	Plus,
	Dash,
	Intersect,
	Open,
	Close,
	Comma,
	Word(String),
	Quoted(String),
}

fn tokenize(text: &str) -> Result<Vec<Token>, ForgeDiagnostic> {
	let mut tokens = Vec::new();
	let mut chars = text.chars().peekable();
	while let Some(&c) = chars.peek() {
		match c {
			' ' | '\t' => {
				chars.next();
			}
			'+' => {
				tokens.push(Token::Plus);
				chars.next();
			}
			'-' => {
				tokens.push(Token::Dash);
				chars.next();
			}
			'(' => {
				tokens.push(Token::Open);
				chars.next();
			}
			')' => {
				tokens.push(Token::Close);
				chars.next();
			}
			',' => {
				tokens.push(Token::Comma);
				chars.next();
			}
			'"' | '\'' => {
				chars.next();
				let mut quoted = String::new();
				loop {
					match chars.next() {
						Some(end) if end == c => break,
						Some(ch) => quoted.push(ch),
						None => return Err(query_error("unterminated string")),
					}
				}
				tokens.push(Token::Quoted(quoted));
			}
			_ => {
				let mut word = String::new();
				while let Some(&ch) = chars.peek() {
					if ch.is_whitespace() || "()+-,\"".contains(ch) {
						break;
					}
					word.push(ch);
					chars.next();
				}
				if word == "intersect" {
					tokens.push(Token::Intersect);
				} else if word.is_empty() {
					return Err(query_error("stray separator"));
				} else {
					tokens.push(Token::Word(word));
				}
			}
		}
	}
	Ok(tokens)
}

struct TokenStream {
	tokens: Vec<Token>,
	position: usize,
}

impl TokenStream {
	fn peek(&self) -> Option<&Token> {
		self.tokens.get(self.position)
	}

	fn next(&mut self) -> Option<Token> {
		let token = self.tokens.get(self.position).cloned();
		if token.is_some() {
			self.position += 1;
		}
		token
	}

	fn expect_end(&self) -> Result<(), ForgeDiagnostic> {
		if self.position == self.tokens.len() {
			Ok(())
		} else {
			Err(query_error(format!("unexpected `{}`", describe(self.peek()))))
		}
	}

	fn eat(&mut self, token: &Token) -> bool {
		if self.peek() == Some(token) {
			self.position += 1;
			true
		} else {
			false
		}
	}

	fn parse_union(&mut self) -> Result<Expr, ForgeDiagnostic> {
		let mut left = self.parse_intersection()?;
		loop {
			if self.eat(&Token::Plus) {
				let right = self.parse_intersection()?;
				left = Expr::Union(Box::new(left), Box::new(right));
			} else if self.eat(&Token::Dash) {
				let right = self.parse_intersection()?;
				left = Expr::Difference(Box::new(left), Box::new(right));
			} else {
				return Ok(left);
			}
		}
	}

	fn parse_intersection(&mut self) -> Result<Expr, ForgeDiagnostic> {
		let mut left = self.parse_primary()?;
		while self.eat(&Token::Intersect) {
			let right = self.parse_primary()?;
			left = Expr::Intersect(Box::new(left), Box::new(right));
		}
		Ok(left)
	}

	fn parse_primary(&mut self) -> Result<Expr, ForgeDiagnostic> {
		match self.next() {
			Some(Token::Open) => {
				let inner = self.parse_union()?;
				if !self.eat(&Token::Close) {
					return Err(query_error("missing closing `)`"));
				}
				Ok(inner)
			}
			Some(Token::Word(word)) => match word.as_str() {
				"kind" | "filter" | "deps" | "rdeps" | "tests" | "somepath" | "allpaths" | "affected" => {
					self.parse_function(&word)
				}
				_ => Ok(Expr::Pattern(word)),
			},
			Some(Token::Quoted(quoted)) => Ok(Expr::Pattern(quoted)),
			other => Err(query_error(format!(
				"expected expression, found {}",
				describe(other.as_ref())
			))),
		}
	}

	fn parse_function(&mut self, name: &str) -> Result<Expr, ForgeDiagnostic> {
		if !self.eat(&Token::Open) {
			return Err(query_error(format!("`{name}` requires arguments")));
		}
		let first = self.parse_union()?;
		match name {
			"kind" => {
				let kind = match first {
					Expr::Pattern(k) | Expr::Filter { pattern: k, .. } => k,
					other => return Err(argument_error(name, "a kind name", &other)),
				};
				expect_comma(self)?;
				let of = self.parse_union()?;
				expect_close(self)?;
				Ok(Expr::Kind { kind, of: Box::new(of) })
			}
			"filter" => {
				let pattern = match first {
					Expr::Pattern(p) | Expr::Filter { pattern: p, .. } => p,
					other => return Err(argument_error(name, "a pattern", &other)),
				};
				expect_comma(self)?;
				let of = self.parse_union()?;
				expect_close(self)?;
				Ok(Expr::Filter {
					pattern,
					of: Box::new(of),
				})
			}
			"somepath" | "allpaths" => {
				expect_comma(self)?;
				let to = self.parse_union()?;
				expect_close(self)?;
				let from = first;
				if name == "somepath" {
					Ok(Expr::SomePath {
						from: Box::new(from),
						to: Box::new(to),
					})
				} else {
					Ok(Expr::AllPaths {
						from: Box::new(from),
						to: Box::new(to),
					})
				}
			}
			"affected" => {
				expect_close(self)?;
				let file = match first {
					Expr::Pattern(f) => f,
					other => return Err(argument_error(name, "a file path", &other)),
				};
				Ok(Expr::Affected { file })
			}
			_ => {
				expect_close(self)?;
				let boxed = Box::new(first);
				Ok(match name {
					"deps" => Expr::Deps(boxed),
					"rdeps" => Expr::Rdeps(boxed),
					_ => Expr::Tests(boxed),
				})
			}
		}
	}
}

fn expect_comma(stream: &mut TokenStream) -> Result<(), ForgeDiagnostic> {
	stream
		.eat(&Token::Comma)
		.then_some(())
		.ok_or_else(|| query_error("expected `,`"))
}

fn expect_close(stream: &mut TokenStream) -> Result<(), ForgeDiagnostic> {
	stream
		.eat(&Token::Close)
		.then_some(())
		.ok_or_else(|| query_error("missing closing `)`"))
}

fn describe(token: Option<&Token>) -> String {
	match token {
		Some(Token::Word(w)) => format!("`{w}`"),
		Some(Token::Quoted(q)) => format!("`\"{q}\"`"),
		Some(Token::Plus) => "`+`".into(),
		Some(Token::Dash) => "`-`".into(),
		Some(Token::Intersect) => "`intersect`".into(),
		Some(Token::Open) => "`(`".into(),
		Some(Token::Close) => "`)`".into(),
		Some(Token::Comma) => "`,`".into(),
		None => "end of query".into(),
	}
}

fn query_error(message: impl Into<String>) -> ForgeDiagnostic {
	ForgeDiagnostic::error(codes::script::BAD_EXPRESSION, format!("query: {}", message.into()))
		.with_help("examples: kind(binary, //...)  rdeps(//:math) intersect kind(test, //...)")
}

fn argument_error(function: &str, expected: &str, _found: &Expr) -> ForgeDiagnostic {
	query_error(format!("`{function}` expects {expected} as its first argument"))
}

pub fn evaluate(graph: &BuildGraph, expr: &Expr) -> Result<BTreeSet<ComponentId>, ForgeDiagnostic> {
	match expr {
		Expr::Pattern(pattern) => Ok(graph.filter(std::slice::from_ref(pattern)).into_iter().collect()),
		Expr::Kind { kind, of } => {
			let wanted = normalize_kind(kind)
				.ok_or_else(|| query_error(format!("unknown kind `{kind}`; expected library, binary, test, or rule")))?;
			Ok(evaluate(graph, of)?
				.into_iter()
				.filter(|id| matches_kind(graph.component(*id).kind.clone(), wanted))
				.collect())
		}
		Expr::Filter { pattern, of } => {
			let mut regex_cache = None;
			Ok(evaluate(graph, of)?
				.into_iter()
				.filter(|id| label_matches(graph.component(*id).label.to_string(), pattern, &mut regex_cache))
				.collect())
		}
		Expr::Deps(of) => {
			let mut out = BTreeSet::new();
			for id in evaluate(graph, of)? {
				out.extend(graph.transitive_dependencies(id));
			}
			Ok(out)
		}
		Expr::Rdeps(of) => {
			let mut out = BTreeSet::new();
			for id in evaluate(graph, of)? {
				out.insert(id);
				out.extend(graph.transitive_dependents(id));
			}
			Ok(out)
		}
		Expr::Tests(of) => {
			let subjects = evaluate(graph, of)?;
			Ok(graph
				.node_ids()
				.filter(|id| matches_kind(graph.component(*id).kind.clone(), "test"))
				.filter(|test| graph.transitive_dependencies(*test).iter().any(|dep| subjects.contains(dep)))
				.collect())
		}
		Expr::Affected { file } => {
			let mut out = BTreeSet::new();
			for id in graph.components_containing_source(file) {
				out.insert(id);
				out.extend(graph.transitive_dependents(id));
			}
			Ok(out)
		}
		Expr::SomePath { from, to } => {
			let sources = evaluate(graph, from)?;
			let targets = evaluate(graph, to)?;
			for source in &sources {
				for target in &targets {
					if let Some(path) = shortest_path(graph, *source, *target) {
						return Ok(path.into_iter().collect());
					}
				}
			}
			Err(query_error("no path exists between the given sets"))
		}
		Expr::AllPaths { from, to } => {
			let sources = evaluate(graph, from)?;
			let targets = evaluate(graph, to)?;
			let mut out = BTreeSet::new();
			let mut any = false;
			for source in &sources {
				for target in &targets {
					let paths = graph.all_paths(*source, *target);
					any |= !paths.is_empty();
					for path in paths {
						out.extend(path);
					}
				}
			}
			if any {
				Ok(out)
			} else {
				Err(query_error("no path exists between the given sets"))
			}
		}
		Expr::Union(a, b) => {
			let mut out = evaluate(graph, a)?;
			out.extend(evaluate(graph, b)?);
			Ok(out)
		}
		Expr::Intersect(a, b) => {
			let left = evaluate(graph, a)?;
			let right = evaluate(graph, b)?;
			Ok(left.intersection(&right).copied().collect())
		}
		Expr::Difference(a, b) => {
			let left = evaluate(graph, a)?;
			let right = evaluate(graph, b)?;
			Ok(left.difference(&right).copied().collect())
		}
	}
}

fn shortest_path(graph: &BuildGraph, from: ComponentId, to: ComponentId) -> Option<Vec<ComponentId>> {
	if from == to {
		return Some(vec![from]);
	}
	let mut parents: BTreeMap<ComponentId, ComponentId> = BTreeMap::new();
	let mut queue = std::collections::VecDeque::from([from]);
	while let Some(current) = queue.pop_front() {
		for dependency in graph.dependencies_of(current) {
			if parents.contains_key(&dependency) || dependency == from {
				continue;
			}
			parents.insert(dependency, current);
			if dependency == to {
				let mut path = vec![to];
				let mut walk = to;
				while let Some(&parent) = parents.get(&walk) {
					path.push(parent);
					walk = parent;
				}
				path.reverse();
				return Some(path);
			}
			queue.push_back(dependency);
		}
	}
	None
}

fn normalize_kind(kind: &str) -> Option<&'static str> {
	match kind {
		"library" => Some("library"),
		"binary" => Some("binary"),
		"test" => Some("test"),
		"rule" | "generic" => Some("rule"),
		_ => None,
	}
}

#[allow(clippy::match_like_matches_macro)]
fn matches_kind(actual: ComponentKind, wanted: &str) -> bool {
	match (actual, wanted) {
		(ComponentKind::Library { .. }, "library") => true,
		(ComponentKind::Binary, "binary") => true,
		(ComponentKind::Test, "test") => true,
		(ComponentKind::Generic { .. }, "rule") => true,
		_ => false,
	}
}

fn label_matches(label: String, pattern: &str, cache: &mut Option<(String, regex::Regex)>) -> bool {
	if cache.as_ref().is_none_or(|(source, _)| source != pattern) {
		let compiled = regex::Regex::new(pattern).unwrap_or_else(|_| regex::Regex::new(r"$^").expect("never"));
		*cache = Some((pattern.to_string(), compiled));
	}
	cache.as_ref().map(|(_, compiled)| compiled.is_match(&label)).unwrap_or(false)
}

use std::collections::BTreeMap;

#[cfg(test)]
mod tests {
	use super::*;
	use crate::BuildGraph;
	use crate::graph::component::{LinkType, Visibility};
	use crate::graph::edge::DependencyEdge;

	fn graph_fixture() -> BuildGraph {
		let mut g = BuildGraph::new();
		let base = g
			.add_component(component(
				"//lib:base",
				crate::graph::component::ComponentKind::Library { link: LinkType::Static },
			))
			.unwrap();
		let math = g
			.add_component(component(
				"//lib:math",
				crate::graph::component::ComponentKind::Library { link: LinkType::Static },
			))
			.unwrap();
		let app = g.add_component(component("//app:demo", ComponentKind::Binary)).unwrap();
		let test = g.add_component(component("//app:test", ComponentKind::Test)).unwrap();
		g.add_dependency(math, "//lib:base", DependencyEdge::Hard).unwrap();
		g.add_dependency(app, "//lib:math", DependencyEdge::Hard).unwrap();
		g.add_dependency(test, "//lib:math", DependencyEdge::Hard).unwrap();
		let _ = base;
		g
	}

	fn component(label: &str, kind: ComponentKind) -> crate::graph::component::Component {
		crate::graph::component::Component {
			label: crate::label::Label::parse(label, "").unwrap(),
			kind,
			visibility: Visibility::Public,
			compatible_with: vec![],
			sources: vec![],
			headers: vec![],
			configuration: Default::default(),
		}
	}

	fn labels(graph: &BuildGraph, ids: &BTreeSet<ComponentId>) -> Vec<String> {
		let mut names: Vec<String> = ids.iter().map(|id| graph.component(*id).label.to_string()).collect();
		names.sort();
		names
	}

	#[test]
	fn patterns_and_set_operators() {
		let g = graph_fixture();
		let eval = |q| labels(&g, &evaluate(&g, &parse(q).unwrap()).unwrap());

		assert_eq!(eval("kind(library, //...)"), vec!["//lib:base", "//lib:math"]);
		assert_eq!(eval("//app:demo"), vec!["//app:demo"]);
		assert_eq!(
			eval("//... - kind(binary, //...)"),
			vec!["//app:test", "//lib:base", "//lib:math"]
		);
		assert_eq!(
			eval("kind(binary, //...) + kind(test, //...)"),
			vec!["//app:demo", "//app:test"]
		);
	}

	#[test]
	fn traversal_functions() {
		let g = graph_fixture();
		let eval = |q| labels(&g, &evaluate(&g, &parse(q).unwrap()).unwrap());

		assert_eq!(eval("deps(//app:demo)"), vec!["//lib:base", "//lib:math"]);
		assert_eq!(
			eval("rdeps(//lib:base)"),
			vec!["//app:demo", "//app:test", "//lib:base", "//lib:math"]
		);
		assert_eq!(eval("tests(//lib:math)"), vec!["//app:test"]);
	}

	#[test]
	fn intersections_and_paths() {
		let g = graph_fixture();
		let eval = |q| labels(&g, &evaluate(&g, &parse(q).unwrap()).unwrap());

		assert_eq!(eval("kind(test, //...) intersect rdeps(//lib:math)"), vec!["//app:test"]);

		let path = eval("somepath(//app:demo, //lib:base)");
		assert!(path.contains(&"//app:demo".to_string()));
		assert!(path.contains(&"//lib:base".to_string()));
		assert_eq!(path.len(), 3);

		let all = eval("allpaths(//app:demo, //lib:base)");
		assert!(all.contains(&"//app:demo".to_string()));
		assert!(all.contains(&"//lib:base".to_string()));
		assert!(all.contains(&"//lib:math".to_string()));
	}

	#[test]
	fn filter_uses_regex_on_labels() {
		let g = graph_fixture();
		let hits = evaluate(&g, &parse("filter(\".*:m.*\", //...)").unwrap()).unwrap();
		assert_eq!(labels(&g, &hits), vec!["//lib:math"]);
	}

	#[test]
	fn parse_errors_name_the_problem() {
		assert!(parse("kind(binary //...)").is_err());
		assert!(parse("kind(binary, //...").is_err());
		assert!(parse("").is_err());
		assert!(parse("deps()").is_err());
	}

	#[test]
	fn parens_group() {
		let g = graph_fixture();
		let hits = evaluate(&g, &parse("(//app:demo + //app:test) - kind(test, //...)").unwrap()).unwrap();
		assert_eq!(labels(&g, &hits), vec!["//app:demo"]);
	}
}
