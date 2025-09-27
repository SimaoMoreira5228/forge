use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods};
use std::io::{self, Write};
use std::sync::Mutex;

use std::sync::LazyLock;
static PROGRESS_STATE: LazyLock<Mutex<Option<(u64, u64, String)>>> = LazyLock::new(|| Mutex::new(None));

#[derive(Clone)]
pub struct LogApi;

impl UserData for LogApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "log")]
impl LogApi {
	pub fn new() -> Self {
		Self
	}

	/// Info level logging
	fn info(message: String) -> Result<()> {
		log::info!("{}", message);
		Ok(())
	}

	/// Warning level logging
	fn warn(message: String) -> Result<()> {
		log::warn!("{}", message);
		Ok(())
	}

	/// Error level logging
	fn error(message: String) -> Result<()> {
		log::error!("{}", message);
		Ok(())
	}

	/// Debug level logging
	fn debug(message: String) -> Result<()> {
		log::debug!("{}", message);
		Ok(())
	}

	/// Trace level logging
	fn trace(message: String) -> Result<()> {
		log::trace!("{}", message);
		Ok(())
	}

	/// Progress logging
	fn progress(current: u64, total: u64, message: Option<String>) -> Result<()> {
		let message = message.unwrap_or_else(|| "Progress".to_string());

		{
			let mut progress_state = PROGRESS_STATE.lock().unwrap();
			*progress_state = Some((current, total, message.clone()));
		}

		let percentage = if total > 0 {
			(current as f64 / total as f64 * 100.0) as u32
		} else {
			0
		};

		let bar_width = 30;
		let filled = (bar_width as f64 * current as f64 / total.max(1) as f64) as usize;
		let empty = bar_width - filled;

		let bar = format!(
			"[{}{}] {}% ({}/{}) {}",
			"=".repeat(filled),
			" ".repeat(empty),
			percentage,
			current,
			total,
			message
		);

		eprint!("\r{}", bar);
		io::stderr().flush().unwrap();

		if current >= total {
			eprintln!();
		}

		Ok(())
	}

	/// Print without newline (useful for progress updates)
	fn print(message: String) -> Result<()> {
		print!("{}", message);
		io::stdout().flush().unwrap();
		Ok(())
	}

	/// Print with newline
	fn println(message: String) -> Result<()> {
		println!("{}", message);
		Ok(())
	}
}

pub fn create_log_table(lua: &Lua) -> Result<Table> {
	LogApi::create_log_table(lua)
}
