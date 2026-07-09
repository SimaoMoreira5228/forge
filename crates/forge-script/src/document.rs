use std::path::PathBuf;

use forge_diagnostics::{ForgeDiagnostic, codes};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceExpr {
	File(PathBuf),
	Glob(String),
}

impl SourceExpr {
	pub fn pattern(&self) -> std::borrow::Cow<'_, str> {
		match self {
			SourceExpr::File(p) => p.to_string_lossy(),
			SourceExpr::Glob(p) => std::borrow::Cow::Borrowed(p),
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
	Text(String),
	Glob(String),
	GlobFiles(String),
}

impl Value {
	pub fn into_string(self) -> String {
		match self {
			Value::Text(s) | Value::Glob(s) | Value::GlobFiles(s) => s,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
	Library,
	Binary,
	Test,
	Rule,
}

impl TargetKind {
	pub fn table_name(self) -> &'static str {
		match self {
			TargetKind::Library => "library",
			TargetKind::Binary => "binary",
			TargetKind::Test => "test",
			TargetKind::Rule => "rule",
		}
	}

	fn allowed_keys(self) -> &'static [&'static str] {
		match self {
			TargetKind::Library | TargetKind::Binary => &[
				"visibility",
				"compatible_with",
				"srcs",
				"hdrs",
				"deps",
				"includes",
				"defines",
				"flags",
				"compiler",
				"standard",
				"system_libs",
				"env",
			],
			TargetKind::Test => &[
				"visibility",
				"compatible_with",
				"srcs",
				"deps",
				"args",
				"data",
				"timeout_secs",
				"env",
			],
			TargetKind::Rule => &["visibility", "compatible_with", "command", "args", "inputs", "outputs", "env"],
		}
	}
}

#[derive(Debug, Clone)]
pub struct TargetDecl {
	pub kind: TargetKind,
	pub name: String,
	pub visibility: forge_core::Visibility,
	pub compatible_with: Vec<String>,
	pub sources: Vec<SourceExpr>,
	pub headers: Vec<SourceExpr>,
	pub deps: Vec<String>,
	pub includes: Vec<String>,
	pub defines: Vec<String>,
	pub flags: Vec<String>,
	pub compiler: Option<String>,
	pub standard: Option<String>,
	pub system_libs: Vec<String>,
	pub command: Option<String>,
	pub args: Vec<String>,
	pub inputs: Vec<SourceExpr>,
	pub outputs: Vec<(PathBuf, bool)>,
	pub env: std::collections::BTreeMap<String, String>,
	pub timeout_secs: u64,
	pub data: Vec<PathBuf>,
}

impl TargetDecl {
	pub fn new(kind: TargetKind, name: impl Into<String>) -> Self {
		Self {
			kind,
			name: name.into(),
			visibility: forge_core::Visibility::Package,
			compatible_with: vec![],
			sources: vec![],
			headers: vec![],
			deps: vec![],
			includes: vec![],
			defines: vec![],
			flags: vec![],
			compiler: None,
			standard: None,
			system_libs: vec![],
			command: None,
			args: vec![],
			inputs: vec![],
			outputs: vec![],
			env: std::collections::BTreeMap::new(),
			timeout_secs: 60,
			data: vec![],
		}
	}
}

fn unknown_key(key: &str, kind: TargetKind, suggestion: Option<&str>) -> ForgeDiagnostic {
	let d = ForgeDiagnostic::error(
		codes::script::UNKNOWN_KEY,
		format!("unknown key `{key}` in {} declaration", kind.table_name()),
	);
	match suggestion {
		Some(s) => d.with_help(format!("did you mean `{s}`?")),
		None => d,
	}
}

pub struct FieldsBuilder {
	decl: TargetDecl,
}

impl FieldsBuilder {
	pub fn new(kind: TargetKind, name: impl Into<String>) -> Result<Self, ForgeDiagnostic> {
		Ok(Self {
			decl: TargetDecl::new(kind, name),
		})
	}

	fn check(&self, key: &str) -> Result<(), ForgeDiagnostic> {
		if self.decl.kind.allowed_keys().contains(&key) {
			return Ok(());
		}
		let suggestion = forge_diagnostics::suggest::closest(key, self.decl.kind.allowed_keys().iter().copied());
		Err(unknown_key(key, self.decl.kind, suggestion))
	}

	pub fn string_list(&mut self, key: &str, values: Vec<String>) -> Result<(), ForgeDiagnostic> {
		self.check(key)?;
		let decl = &mut self.decl;
		match key {
			"srcs" => decl.sources.extend(values.into_iter().map(to_source_expr)),
			"hdrs" => decl.headers.extend(values.into_iter().map(to_source_expr)),
			"inputs" => decl.inputs.extend(values.into_iter().map(to_source_expr)),
			"deps" => decl.deps.extend(values),
			"includes" => decl.includes.extend(values),
			"defines" => decl.defines.extend(values),
			"flags" => decl.flags.extend(values),
			"compatible_with" => decl.compatible_with.extend(values),
			"system_libs" => decl.system_libs.extend(values),
			"args" | "data" => {
				if key == "args" {
					decl.args.extend(values)
				} else {
					decl.data.extend(values.iter().map(PathBuf::from))
				}
			}
			_ => unreachable!("string_list on non-list key"),
		}
		Ok(())
	}

	pub fn string(&mut self, key: &str, value: String) -> Result<(), ForgeDiagnostic> {
		self.check(key)?;
		let decl = &mut self.decl;
		match key {
			"visibility" => decl.visibility = parse_visibility(&value)?,
			"compiler" => decl.compiler = Some(value),
			"standard" => decl.standard = Some(value),
			"command" => decl.command = Some(value),
			_ => unreachable!("string on unexpected key"),
		}
		Ok(())
	}

	pub fn integer(&mut self, _key: &str, value: i64) -> Result<(), ForgeDiagnostic> {
		self.check("timeout_secs")?;
		self.decl.timeout_secs = value.max(1) as u64;
		Ok(())
	}

	pub fn output_entry(&mut self, entry: String) -> Result<(), ForgeDiagnostic> {
		let is_dir = entry.ends_with('/');
		let path = PathBuf::from(entry.trim_end_matches('/'));
		self.decl.outputs.push((path, is_dir));
		Ok(())
	}

	pub fn env_entry(&mut self, k: String, v: String) -> Result<(), ForgeDiagnostic> {
		self.decl.env.insert(k, v);
		Ok(())
	}

	pub fn visibility_patterns(&mut self, patterns: Vec<String>) -> Result<(), ForgeDiagnostic> {
		self.decl.visibility = forge_core::Visibility::Patterns(patterns);
		Ok(())
	}

	pub fn finish(self) -> Result<TargetDecl, ForgeDiagnostic> {
		match self.decl.kind {
			TargetKind::Rule if self.decl.command.is_none() => Err(ForgeDiagnostic::error(
				codes::script::WRONG_TYPE,
				format!("rule `{}` requires `command`", self.decl.name),
			)),
			_ => Ok(self.decl),
		}
	}
}

fn to_source_expr(v: String) -> SourceExpr {
	if v.contains('*') || v.contains('?') {
		SourceExpr::Glob(v)
	} else {
		SourceExpr::File(PathBuf::from(v))
	}
}

fn parse_visibility(text: &str) -> Result<forge_core::Visibility, ForgeDiagnostic> {
	match text {
		"public" => Ok(forge_core::Visibility::Public),
		"package" | "private" => Ok(forge_core::Visibility::Package),
		other => Err(
			ForgeDiagnostic::error(codes::script::WRONG_TYPE, format!("invalid visibility `{other}`"))
				.with_help("expected \"public\", \"package\", or an array of //pkg patterns"),
		),
	}
}
