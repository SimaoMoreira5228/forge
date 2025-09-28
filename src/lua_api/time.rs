use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods};
use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static TIMERS: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone)]
pub struct TimeApi;

impl UserData for TimeApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "time")]
impl TimeApi {
	pub fn new() -> Self {
		Self
	}

	/// Get current Unix timestamp
	fn now() -> Result<u64> {
		let now = SystemTime::now();
		let timestamp = now.duration_since(UNIX_EPOCH).map_err(mlua::Error::external)?.as_secs();
		Ok(timestamp)
	}

	/// Get current Unix timestamp with milliseconds
	fn now_millis() -> Result<u64> {
		let now = SystemTime::now();
		let timestamp = now.duration_since(UNIX_EPOCH).map_err(mlua::Error::external)?.as_millis();
		Ok(timestamp as u64)
	}

	/// Format timestamp
	fn format(timestamp: u64, format: Option<String>) -> Result<String> {
		let format = format.unwrap_or_else(|| "%Y-%m-%d %H:%M:%S".to_string());
		let dt = chrono::DateTime::from_timestamp(timestamp as i64, 0)
			.ok_or_else(|| mlua::Error::external("Invalid timestamp"))?;
		Ok(dt.format(&format).to_string())
	}

	/// Sleep for specified duration (in seconds)
	fn sleep(duration: f64) -> Result<()> {
		let duration = Duration::from_secs_f64(duration);
		std::thread::sleep(duration);
		Ok(())
	}

	/// Start a named timer
	fn start_timer(name: Option<String>) -> Result<()> {
		let name = name.unwrap_or_else(|| "default".to_string());
		let mut timers = TIMERS.lock().unwrap();
		timers.insert(name.clone(), Instant::now());
		Ok(())
	}

	/// Get elapsed time since timer start
	fn elapsed(name: Option<String>) -> Result<Option<f64>> {
		let name = name.unwrap_or_else(|| "default".to_string());
		let timers = TIMERS.lock().unwrap();

		if let Some(start_time) = timers.get(&name) {
			let elapsed = start_time.elapsed().as_secs_f64();
			Ok(Some(elapsed))
		} else {
			Ok(None)
		}
	}

	/// Calculate duration between two timestamps
	fn since(start_time: Option<u64>, end_time: Option<u64>) -> Result<Option<u64>> {
		let end_time = end_time.unwrap_or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs());

		if let Some(start_time) = start_time {
			Ok(if end_time >= start_time {
				Some(end_time - start_time)
			} else {
				None
			})
		} else {
			Ok(None)
		}
	}
}

pub fn create_time_table(lua: &Lua) -> Result<Table> {
	TimeApi::create_time_table(lua)
}
