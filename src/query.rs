use crate::graph::{BuildGraph, ComponentId};
use std::collections::HashSet;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum QueryError {
	#[error("Syntax error: {0}")]
	Syntax(String),
	#[error("Evaluation error: {0}")]
	Evaluation(String),
	#[error("Component not found: {0}")]
	NotFound(String),
}

#[derive(Debug, Clone)]
pub enum QueryExpr {
	Label(String),
	Deps(Box<QueryExpr>),
	Rdeps(Box<QueryExpr>),
	AllPaths(Box<QueryExpr>, Box<QueryExpr>),
	Filter(String, Box<QueryExpr>),
	Attr(String, String, Box<QueryExpr>),
	Union(Box<QueryExpr>, Box<QueryExpr>),
	Intersect(Box<QueryExpr>, Box<QueryExpr>),
	Except(Box<QueryExpr>, Box<QueryExpr>),
}

pub struct QueryEngine<'a> {
	graph: &'a BuildGraph,
}

impl<'a> QueryEngine<'a> {
	pub fn new(graph: &'a BuildGraph) -> Self {
		Self { graph }
	}

	pub fn evaluate(&self, expression: &str) -> Result<HashSet<ComponentId>, QueryError> {
		let expr = self.parse(expression)?;
		self.evaluate_expr(&expr)
	}

	fn parse(&self, expression: &str) -> Result<QueryExpr, QueryError> {
		let tokens = self.tokenize(expression)?;
		let mut pos = 0;
		let expr = self.parse_expression(&tokens, &mut pos)?;
		if pos < tokens.len() {
			return Err(QueryError::Syntax(format!("Unexpected token: {}", tokens[pos])));
		}
		Ok(expr)
	}

	fn tokenize(&self, s: &str) -> Result<Vec<String>, QueryError> {
		let mut tokens = Vec::new();
		let mut current = String::new();
		let mut chars = s.chars().peekable();

		while let Some(&c) = chars.peek() {
			match c {
				'(' | ')' | ',' | '+' | '-' | '^' => {
					if !current.is_empty() {
						tokens.push(current.trim().to_string());
						current.clear();
					}
					tokens.push(c.to_string());
					chars.next();
				}
				' ' | '\t' | '\n' | '\r' => {
					if !current.is_empty() {
						tokens.push(current.trim().to_string());
						current.clear();
					}
					chars.next();
				}
				_ => {
					current.push(c);
					chars.next();
				}
			}
		}
		if !current.is_empty() {
			tokens.push(current.trim().to_string());
		}
		Ok(tokens.into_iter().filter(|t| !t.is_empty()).collect())
	}

	fn parse_expression(&self, tokens: &[String], pos: &mut usize) -> Result<QueryExpr, QueryError> {
		let mut left = self.parse_primary(tokens, pos)?;

		while *pos < tokens.len() {
			let op = &tokens[*pos];
			match op.as_str() {
				"+" | "union" => {
					*pos += 1;
					let right = self.parse_primary(tokens, pos)?;
					left = QueryExpr::Union(Box::new(left), Box::new(right));
				}
				"-" | "except" => {
					*pos += 1;
					let right = self.parse_primary(tokens, pos)?;
					left = QueryExpr::Except(Box::new(left), Box::new(right));
				}
				"^" | "intersect" => {
					*pos += 1;
					let right = self.parse_primary(tokens, pos)?;
					left = QueryExpr::Intersect(Box::new(left), Box::new(right));
				}
				_ => break,
			}
		}

		Ok(left)
	}

	fn parse_primary(&self, tokens: &[String], pos: &mut usize) -> Result<QueryExpr, QueryError> {
		if *pos >= tokens.len() {
			return Err(QueryError::Syntax("Unexpected end of expression".to_string()));
		}

		let token = &tokens[*pos];
		if token == "(" {
			*pos += 1;
			let expr = self.parse_expression(tokens, pos)?;
			if *pos >= tokens.len() || tokens[*pos] != ")" {
				return Err(QueryError::Syntax("Expected ')'".to_string()));
			}
			*pos += 1;
			Ok(expr)
		} else if *pos + 1 < tokens.len() && tokens[*pos + 1] == "(" {
			self.parse_fn(tokens, pos)
		} else {
			*pos += 1;
			Ok(QueryExpr::Label(token.clone()))
		}
	}

	fn parse_fn(&self, tokens: &[String], pos: &mut usize) -> Result<QueryExpr, QueryError> {
		let name = &tokens[*pos];
		*pos += 2; // skip name and (
		let mut args = Vec::new();
		while *pos < tokens.len() && tokens[*pos] != ")" {
			args.push(self.parse_expression(tokens, pos)?);
			if *pos < tokens.len() && tokens[*pos] == "," {
				*pos += 1;
			}
		}
		if *pos >= tokens.len() || tokens[*pos] != ")" {
			return Err(QueryError::Syntax(format!("Expected ')' after {} call", name)));
		}
		*pos += 1;

		match name.as_str() {
			"deps" => {
				if args.len() != 1 {
					return Err(QueryError::Syntax("deps() takes 1 arg".to_string()));
				}
				Ok(QueryExpr::Deps(Box::new(args.remove(0))))
			}
			"rdeps" => {
				if args.len() != 1 {
					return Err(QueryError::Syntax("rdeps() takes 1 arg".to_string()));
				}
				Ok(QueryExpr::Rdeps(Box::new(args.remove(0))))
			}
			"allpaths" => {
				if args.len() != 2 {
					return Err(QueryError::Syntax("allpaths() takes 2 args".to_string()));
				}
				let to = args.pop().unwrap();
				let from = args.pop().unwrap();
				Ok(QueryExpr::AllPaths(Box::new(from), Box::new(to)))
			}
			"filter" | "kind" => {
				if args.len() != 2 {
					return Err(QueryError::Syntax(format!("{}() takes 2 args", name)));
				}
				let inner = args.pop().unwrap();
				let kind_expr = args.pop().unwrap();
				if let QueryExpr::Label(kind_name) = kind_expr {
					Ok(QueryExpr::Filter(kind_name, Box::new(inner)))
				} else {
					Err(QueryError::Syntax(format!(
						"First arg of {}() must be a constant name (e.g. 'test')",
						name
					)))
				}
			}
			"attr" => {
				if args.len() != 3 {
					return Err(QueryError::Syntax("attr() takes 3 args: attr(expr, name, value)".to_string()));
				}
				let value_expr = args.pop().unwrap();
				let name_expr = args.pop().unwrap();
				let inner = args.pop().unwrap();

				if let (QueryExpr::Label(attr_name), QueryExpr::Label(attr_value)) = (name_expr, value_expr) {
					Ok(QueryExpr::Attr(attr_name, attr_value, Box::new(inner)))
				} else {
					Err(QueryError::Syntax("Arguments 2 and 3 of attr() must be constant labels".to_string()))
				}
			}
			_ => Err(QueryError::Syntax(format!("Unknown function: {}", name))),
		}
	}

	fn evaluate_expr(&self, expr: &QueryExpr) -> Result<HashSet<ComponentId>, QueryError> {
		match expr {
			QueryExpr::Label(pattern) => {
				if pattern == "//..." {
					return Ok(self.graph.component_ids().collect());
				}
				let ids = self.graph.components_matching(pattern);
				if ids.is_empty() && !pattern.contains('*') && !pattern.contains("...") {
					return Err(QueryError::NotFound(pattern.clone()));
				}
				Ok(ids.into_iter().collect())
			}
			QueryExpr::Deps(inner) => {
				let start_ids = self.evaluate_expr(inner)?;
				let mut result = HashSet::new();
				for id in start_ids {
					result.insert(id);
					let deps = self.graph.transitive_dependencies(id);
					result.extend(deps);
				}
				Ok(result)
			}
			QueryExpr::Rdeps(inner) => {
				let start_ids = self.evaluate_expr(inner)?;
				let mut result = HashSet::new();
				for id in start_ids {
					result.insert(id);
					let rdeps = self.transitive_reverse_dependencies(id);
					result.extend(rdeps);
				}
				Ok(result)
			}
			QueryExpr::AllPaths(from_expr, to_expr) => {
				let from_ids = self.evaluate_expr(from_expr)?;
				let to_ids = self.evaluate_expr(to_expr)?;
				let mut result = HashSet::new();
				for from in from_ids {
					for to in &to_ids {
						let paths = self.graph.all_paths(from, *to);
						for path in paths {
							result.extend(path);
						}
					}
				}
				Ok(result)
			}
			QueryExpr::Filter(kind_name, inner) => {
				let ids = self.evaluate_expr(inner)?;
				let mut result = HashSet::new();
				for id in ids {
					if let Some(comp) = self.graph.get_component(id) {
						if comp.component_type.kind_name() == kind_name {
							result.insert(id);
						}
					}
				}
				Ok(result)
			}
			QueryExpr::Attr(name, value, inner) => {
				let ids = self.evaluate_expr(inner)?;
				let mut result = HashSet::new();
				for id in ids {
					if let Some(comp) = self.graph.get_component(id) {
						// Check attributes. For now we only check a few fixed ones
						// but in a real system we'd have a general metadata map.
						let matched = match name.as_str() {
							"profile" => comp.target_name.contains(value), // placeholder
							"package" => comp.package.as_str() == value,
							"visibility" => format!("{:?}", comp.visibility).to_lowercase() == value.to_lowercase(),
							_ => false,
						};
						if matched {
							result.insert(id);
						}
					}
				}
				Ok(result)
			}
			QueryExpr::Union(left, right) => {
				let mut l = self.evaluate_expr(left)?;
				let r = self.evaluate_expr(right)?;
				l.extend(r);
				Ok(l)
			}
			QueryExpr::Intersect(left, right) => {
				let l = self.evaluate_expr(left)?;
				let r = self.evaluate_expr(right)?;
				Ok(l.intersection(&r).copied().collect())
			}
			QueryExpr::Except(left, right) => {
				let l = self.evaluate_expr(left)?;
				let r = self.evaluate_expr(right)?;
				Ok(l.difference(&r).copied().collect())
			}
		}
	}

	fn transitive_reverse_dependencies(&self, id: ComponentId) -> Vec<ComponentId> {
		let mut result = Vec::new();
		let mut visited = HashSet::new();
		self.collect_transitive_rdeps(id, &mut result, &mut visited);
		result
	}

	fn collect_transitive_rdeps(
		&self,
		id: ComponentId,
		result: &mut Vec<ComponentId>,
		visited: &mut HashSet<ComponentId>,
	) {
		if visited.contains(&id) {
			return;
		}
		visited.insert(id);
		for rdep_id in self.graph.reverse_dependencies(id) {
			if !result.contains(&rdep_id) {
				result.push(rdep_id);
			}
			self.collect_transitive_rdeps(rdep_id, result, visited);
		}
	}
}
