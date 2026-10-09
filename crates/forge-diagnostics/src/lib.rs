pub mod codes;
pub mod suggest;

use std::fmt;

use miette::{Diagnostic, NamedSource, SourceSpan};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
	Error,
	Warning,
}

impl Severity {
	pub fn prefix(self) -> &'static str {
		match self {
			Severity::Error => "E",
			Severity::Warning => "W",
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticCode(pub u16);

impl DiagnosticCode {
	pub const fn new(n: u16) -> Self {
		Self(n)
	}
}

impl fmt::Display for DiagnosticCode {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:03}", self.0)
	}
}

type RelatedNotes = Vec<(String, Option<(NamedSource<String>, SourceSpan)>)>;

#[derive(Debug, Clone)]
pub struct ForgeDiagnostic {
	pub severity: Severity,
	pub code: DiagnosticCode,
	pub message: String,
	pub help: Option<String>,
	pub source: Option<NamedSource<String>>,
	pub span: Option<SourceSpan>,
	pub related: RelatedNotes,
	pub diagnostics: Vec<ForgeDiagnostic>,
}

impl ForgeDiagnostic {
	pub fn error(code: u16, message: impl Into<String>) -> Self {
		Self {
			severity: Severity::Error,
			code: DiagnosticCode::new(code),
			message: message.into(),
			help: None,
			source: None,
			span: None,
			related: Vec::new(),
			diagnostics: Vec::new(),
		}
	}

	pub fn batch(diagnostics: Vec<Self>) -> Self {
		let mut items = diagnostics.into_iter();
		let mut first = items
			.next()
			.unwrap_or_else(|| Self::error(codes::script::PARSE_ERROR, "build failed"));
		first.diagnostics.extend(items);
		first
	}

	pub fn warning(code: u16, message: impl Into<String>) -> Self {
		Self {
			severity: Severity::Warning,
			..Self::error(code, message)
		}
	}

	pub fn io(path: &std::path::Path, e: std::io::Error) -> Self {
		Self::error(crate::codes::hermetic::HERMETIC_VIOLATION, format!("{}: {e}", path.display()))
	}

	pub fn with_help(mut self, help: impl Into<String>) -> Self {
		self.help = Some(help.into());
		self
	}

	pub fn with_source(mut self, name: &str, contents: impl Into<String>) -> Self {
		self.source = Some(NamedSource::new(name, contents.into()));
		self
	}

	pub fn at(mut self, span: impl Into<SourceSpan>) -> Self {
		self.span = Some(span.into());
		self
	}

	pub fn related_note(mut self, note: impl Into<String>) -> Self {
		self.related.push((note.into(), None));
		self
	}
}

impl fmt::Display for ForgeDiagnostic {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}[{}] {}", self.severity.prefix(), self.code, self.message)?;
		if let Some(help) = &self.help {
			write!(f, "\n  help: {help}")?;
		}
		Ok(())
	}
}

impl std::error::Error for ForgeDiagnostic {}

impl Diagnostic for ForgeDiagnostic {
	fn related(&self) -> Option<Box<dyn Iterator<Item = &dyn Diagnostic> + '_>> {
		if self.diagnostics.is_empty() {
			None
		} else {
			Some(Box::new(self.diagnostics.iter().map(|d| d as &dyn Diagnostic)))
		}
	}

	fn code<'a>(&'a self) -> Option<std::boxed::Box<dyn std::fmt::Display + 'a>> {
		Some(Box::new(format!("{}[{}]", self.severity.prefix(), self.code)))
	}

	fn help<'a>(&'a self) -> Option<std::boxed::Box<dyn std::fmt::Display + 'a>> {
		self.help.as_ref().map(|h| Box::new(h) as Box<dyn std::fmt::Display>)
	}

	fn source_code(&self) -> Option<&dyn miette::SourceCode> {
		self.source.as_ref().map(|s| s as &dyn miette::SourceCode)
	}

	fn labels(&self) -> Option<Box<dyn Iterator<Item = miette::LabeledSpan> + '_>> {
		let primary = self
			.span
			.map(|span| miette::LabeledSpan::new(Some(self.message.clone()), span.offset(), span.len()));
		if self.related.is_empty() {
			return primary.map(|p| Box::new(std::iter::once(p)) as Box<dyn Iterator<Item = _>>);
		}
		let mut spans = Vec::new();
		if let Some(p) = primary {
			spans.push(p);
		}
		for (note, _) in &self.related {
			spans.push(miette::LabeledSpan::new(Some(note.clone()), 0, 0));
		}
		Some(Box::new(spans.into_iter()))
	}
}

#[derive(Debug, Default)]
pub struct DiagnosticSink {
	items: Vec<ForgeDiagnostic>,
}

impl DiagnosticSink {
	pub fn push(&mut self, d: ForgeDiagnostic) {
		self.items.push(d);
	}

	pub fn extend(&mut self, other: DiagnosticSink) {
		self.items.extend(other.items);
	}

	pub fn errors(&self) -> impl Iterator<Item = &ForgeDiagnostic> {
		self.items.iter().filter(|d| d.severity == Severity::Error)
	}

	pub fn has_errors(&self) -> bool {
		self.errors().next().is_some()
	}

	pub fn into_items(self) -> Vec<ForgeDiagnostic> {
		self.items
	}

	pub fn report(&self) -> miette::Result<()> {
		if self.has_errors() {
			Err(miette::Report::new(ForgeDiagnostic::batch(self.items.clone())))
		} else {
			for diagnostic in &self.items {
				eprintln!("{:?}", miette::Report::new(diagnostic.clone()));
			}
			Ok(())
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn batching_keeps_each_source_and_span() {
		let mut sink = DiagnosticSink::default();
		for name in ["one.toml", "two.toml"] {
			sink.push(
				ForgeDiagnostic::error(101, "invalid syntax")
					.with_source(
						name, "broken
",
					)
					.at(0..6),
			);
		}
		let report = sink.report().unwrap_err();
		let mut rendered = String::new();
		miette::GraphicalReportHandler::new()
			.render_report(&mut rendered, report.as_ref())
			.unwrap();
		assert!(rendered.contains("one.toml:1:1"), "{rendered}");
		assert!(rendered.contains("two.toml:1:1"), "{rendered}");
		assert_eq!(rendered.matches("broken").count(), 2, "{rendered}");
	}
}
