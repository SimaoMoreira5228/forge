use std::fmt;

use forge_diagnostics::ForgeDiagnostic;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct Label {
	package: String,
	name: String,
}

impl Label {
	pub fn new(package: impl Into<String>, name: impl Into<String>) -> Self {
		Self {
			package: normalize_package(package.into()),
			name: name.into(),
		}
	}

	pub fn parse(text: &str, context: &str) -> Result<Self, ForgeDiagnostic> {
		let text = text.trim();
		if let Some(rest) = text.strip_prefix("//") {
			return match rest.split_once(':') {
				Some((pkg, name)) => Ok(Self::new(pkg, name)),
				None if !rest.is_empty() => Ok(Self::new(rest.to_string(), last_segment(rest))),
				None => Err(unknown_target(text)),
			};
		}
		let name = text.strip_prefix(':').unwrap_or(text);
		if name.is_empty() || name.contains('/') {
			return Err(unknown_target(text));
		}
		Ok(Self::new(context, name))
	}

	pub fn package(&self) -> &str {
		&self.package
	}

	pub fn name(&self) -> &str {
		&self.name
	}
}

fn last_segment(path: &str) -> &str {
	path.rsplit('/').next().unwrap_or(path)
}

fn normalize_package(pkg: String) -> String {
	pkg.trim_matches('/').to_string()
}

fn unknown_target(text: &str) -> ForgeDiagnostic {
	ForgeDiagnostic::error(2, format!("unknown target `{text}`"))
}

impl fmt::Display for Label {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.package.is_empty() {
			write!(f, "//:{}", self.name)
		} else {
			write!(f, "//{}:{}", self.package, self.name)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_qualified_labels() {
		let l = Label::parse("//lib/math:utils", "").unwrap();
		assert_eq!(l.package(), "lib/math");
		assert_eq!(l.name(), "utils");
		assert_eq!(l.to_string(), "//lib/math:utils");
	}

	#[test]
	fn implicit_name_from_package() {
		let l = Label::parse("//lib/math", "").unwrap();
		assert_eq!(l.name(), "math");
	}

	#[test]
	fn resolves_relative_against_context() {
		assert_eq!(Label::parse("calc", "bin").unwrap(), Label::new("bin", "calc"));
		assert_eq!(Label::parse(":calc", "bin").unwrap(), Label::new("bin", "calc"));
	}

	#[test]
	fn rejects_garbage() {
		assert!(Label::parse("//", "").is_err());
		assert!(Label::parse("", "x").is_err());
		assert!(Label::parse("a/b", "").is_err());
	}
}
