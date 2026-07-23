use std::fmt;
use std::io::IsTerminal;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

pub struct Progress {
	total: usize,
	current: AtomicUsize,
	profile: String,
	start: Instant,
	output: Mutex<()>,
	interactive: bool,
}

impl Progress {
	pub fn new(total: usize, profile: &str) -> Self {
		Self {
			total,
			current: AtomicUsize::new(0),
			profile: profile.to_string(),
			start: Instant::now(),
			output: Mutex::new(()),
			interactive: std::io::stderr().is_terminal(),
		}
	}

	pub fn set_total(&mut self, total: usize) {
		self.total = total;
	}

	pub fn phase(&self, message: &str) {
		self.print(format_args!("\x1b[2m{message}\x1b[0m"));
	}

	pub fn started(&self, name: &str) {
		self.print(format_args!(
			"\x1b[32m Compiling\x1b[0m {name} \x1b[2m({})\x1b[0m",
			self.profile
		));
	}

	pub fn action_finished(&self, name: &str, cached: bool, duration_ms: Option<u128>) {
		let n = self.current.fetch_add(1, Ordering::SeqCst) + 1;
		let status = if cached { "Fresh" } else { "Finished" };
		let color = if cached { "\x1b[33m" } else { "\x1b[32m" };
		let reset = "\x1b[0m";
		let dim = "\x1b[2m";
		let duration = duration_ms
			.map(|ms| format!(" in {:.2}s", ms as f64 / 1000.0))
			.unwrap_or_default();
		self.print(format_args!(
			"{color} {status:<8}{reset} [{n:>3}/{total}] {name}{duration} {dim}({profile}){reset}",
			total = self.total,
			profile = self.profile
		));
	}

	pub fn header(&self, version: &str) {
		self.print(format_args!("\x1b[1mForge\x1b[0m {version}"));
	}

	pub fn analyzing(&self, count: usize) {
		self.print(format_args!("\x1b[2mAnalyzing {count} packages...\x1b[0m"));
	}

	pub fn planned(&self, count: usize) {
		self.print(format_args!("\x1b[2m  Found {count} actions\x1b[0m"));
	}

	pub fn finished(&self, executed: usize, cached: usize) {
		let elapsed = self.start.elapsed().as_secs_f64();
		self.print(format_args!(
			"\x1b[32m  Finished\x1b[0m `{}` profile target(s) in {elapsed:.2}s — {executed} executed, {cached} cached",
			self.profile
		));
	}

	fn print(&self, message: fmt::Arguments<'_>) {
		if self.interactive {
			let _output = self.output.lock().unwrap();
			eprintln!("{message}");
		}
	}
}
