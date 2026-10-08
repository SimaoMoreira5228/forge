use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::time::Instant;

use forge_core::{ActionSpec, Confinement, WorkRequest, WorkResponse, WorkerBinding};
use parking_lot::Mutex;

use crate::execution::confinement::Policy;
use crate::execution::runner::{self, ExecReport, Launch, SandboxRunner};
use crate::toolchain::ToolchainPaths;

pub const MAX_FRAME_BYTES: u64 = 256 * 1024 * 1024;

const LANGUAGE_ENV: [(&str, &str); 2] = [("LANG", "C.UTF-8"), ("LC_ALL", "C.UTF-8")];

const UNCONFINED: Confinement = Confinement {
	root: PathBuf::new(),
	read_only: Vec::new(),
};

struct Slot {
	program: PathBuf,
	running: Mutex<Option<WorkerProcess>>,
}

struct WorkerProcess {
	child: Child,
	stdin: ChildStdin,
	stdout: ChildStdout,
}

#[derive(Debug)]
pub enum FrameError {
	Closed,
	Oversized(u64),
	Io(std::io::Error),
	Json(serde_json::Error),
}

impl std::fmt::Display for FrameError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			FrameError::Closed => write!(f, "the worker closed the connection"),
			FrameError::Oversized(len) => write!(f, "frame of {len} bytes exceeds the {MAX_FRAME_BYTES} byte limit"),
			FrameError::Io(e) => write!(f, "{e}"),
			FrameError::Json(e) => write!(f, "malformed worker frame: {e}"),
		}
	}
}

pub struct WorkerPool<'a> {
	slots: Mutex<BTreeMap<String, Arc<Slot>>>,
	runner: &'a SandboxRunner,
	toolchains: &'a ToolchainPaths,
}

impl<'a> WorkerPool<'a> {
	pub fn new(runner: &'a SandboxRunner, toolchains: &'a ToolchainPaths) -> Self {
		Self {
			slots: Mutex::new(BTreeMap::new()),
			runner,
			toolchains,
		}
	}

	pub fn execute(&self, spec: &ActionSpec, binding: &WorkerBinding, sandbox: &Path) -> ExecReport {
		let slot = self.slot(binding);
		let request = self.runner.work_request(spec, sandbox, self.toolchains);
		let started = Instant::now();
		let mut running = slot.running.lock();
		if running.is_none() {
			*running = WorkerProcess::start(&slot.program, spec).ok();
		}
		let outcome = match running.as_mut() {
			Some(process) => process.exchange(&request),
			None => Err(format!("worker `{}` could not be started", binding.program)),
		};
		if outcome.is_err()
			&& let Some(process) = running.take()
		{
			process.terminate();
		}
		let duration = started.elapsed();
		match outcome {
			Ok(response) => {
				self.runner.record_stdout(spec, sandbox, &response.stdout);
				ExecReport {
					success: response.status == 0,
					stdout_tail: crate::execution::runner::output_tail(&response.stdout),
					stderr_tail: crate::execution::runner::output_tail(&response.stderr),
					duration,
				}
			}
			Err(reason) => ExecReport {
				success: false,
				stdout_tail: String::new(),
				stderr_tail: format!("worker `{}` failed: {reason}", binding.key()),
				duration,
			},
		}
	}

	pub fn live_workers(&self) -> usize {
		self.slots
			.lock()
			.values()
			.filter(|slot| slot.running.lock().is_some())
			.count()
	}

	fn slot(&self, binding: &WorkerBinding) -> Arc<Slot> {
		let key = binding.key();
		let mut slots = self.slots.lock();
		if let Some(slot) = slots.get(&key) {
			return Arc::clone(slot);
		}
		let slot = Arc::new(Slot {
			program: PathBuf::from(self.toolchains.expand(&binding.program)),
			running: Mutex::new(None),
		});
		slots.insert(key, Arc::clone(&slot));
		slot
	}
}

impl Drop for WorkerPool<'_> {
	fn drop(&mut self) {
		for slot in self.slots.lock().values() {
			if let Some(process) = slot.running.lock().take() {
				process.terminate();
			}
		}
	}
}

impl WorkerProcess {
	fn start(program: &Path, spec: &ActionSpec) -> Result<Self, String> {
		let mut command = Command::new(program);
		command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
		command.env_clear();
		for (key, value) in LANGUAGE_ENV {
			command.env(key, value);
		}
		let mut child = command
			.spawn()
			.map_err(|e| format!("launching `{}` for action `{}` failed: {e}", program.display(), spec.name))?;
		let stdin = child.stdin.take().ok_or("worker stdin unavailable")?;
		let stdout = child.stdout.take().ok_or("worker stdout unavailable")?;
		Ok(Self { child, stdin, stdout })
	}

	fn exchange(&mut self, request: &WorkRequest) -> Result<WorkResponse, String> {
		write_frame(&mut self.stdin, request).map_err(|e| e.to_string())?;
		let bytes = read_frame(&mut self.stdout).map_err(|e| e.to_string())?;
		serde_json::from_slice(&bytes).map_err(|e| FrameError::Json(e).to_string())
	}

	fn terminate(mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

pub fn write_frame(w: &mut impl Write, value: &impl serde::Serialize) -> Result<(), FrameError> {
	let body = serde_json::to_vec(value).map_err(FrameError::Json)?;
	if body.len() as u64 > MAX_FRAME_BYTES {
		return Err(FrameError::Oversized(body.len() as u64));
	}
	w.write_all(&(body.len() as u64).to_le_bytes()).map_err(FrameError::Io)?;
	w.write_all(&body).map_err(FrameError::Io)?;
	w.flush().map_err(FrameError::Io)
}

pub fn read_frame(r: &mut impl Read) -> Result<Vec<u8>, FrameError> {
	let mut header = [0u8; 8];
	let mut filled = 0;
	while filled < header.len() {
		match r.read(&mut header[filled..]) {
			Ok(0) if filled == 0 => return Err(FrameError::Closed),
			Ok(0) => return Err(FrameError::Io(std::io::ErrorKind::UnexpectedEof.into())),
			Ok(read) => filled += read,
			Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
			Err(e) => return Err(FrameError::Io(e)),
		}
	}
	let len = u64::from_le_bytes(header);
	if len > MAX_FRAME_BYTES {
		return Err(FrameError::Oversized(len));
	}
	let mut body = vec![0u8; len as usize];
	r.read_exact(&mut body).map_err(FrameError::Io)?;
	Ok(body)
}

pub fn serve(input: &mut impl Read, output: &mut impl Write) -> Result<(), String> {
	loop {
		let frame = match read_frame(input) {
			Ok(frame) => frame,
			Err(FrameError::Closed) => return Ok(()),
			Err(e) => return Err(e.to_string()),
		};
		let request: WorkRequest = serde_json::from_slice(&frame).map_err(|e| FrameError::Json(e).to_string())?;
		write_frame(output, &run(request)).map_err(|e| e.to_string())?;
	}
}

fn run(request: WorkRequest) -> WorkResponse {
	let launch = Launch {
		program: request.program.clone(),
		args: request.args.clone(),
		env: request.env.clone(),
		workdir: request.workdir.clone(),
	};
	let policy = Policy::of(
		request.confinement.as_ref().unwrap_or(&UNCONFINED),
		request.mount.as_ref(),
		&launch.program,
	);
	let output = runner::spawn(&launch, request.mount.as_ref(), &policy);
	match output {
		Ok(output) => WorkResponse {
			status: output.status.code().unwrap_or(-1),
			stdout: output.stdout,
			stderr: output.stderr,
		},
		Err(e) => WorkResponse {
			status: 127,
			stdout: Vec::new(),
			stderr: format!("{e}\n").into_bytes(),
		},
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn frames_round_trip_and_survive_a_closed_pipe() {
		let request = WorkRequest {
			program: "/bin/tool".into(),
			args: vec!["-c".into()],
			env: BTreeMap::from([("LANG".into(), "C.UTF-8".into())]),
			workdir: PathBuf::from("/sandbox"),
			mount: None,
			confinement: Some(Confinement::new("/sandbox", vec![PathBuf::from("/toolchains/gcc/bin")])),
			outputs: vec![PathBuf::from("out.o")],
		};
		let mut bytes = Vec::new();
		write_frame(&mut bytes, &request).unwrap();
		let mut buffer = std::io::Cursor::new(bytes);
		assert_eq!(read_frame(&mut buffer).unwrap(), serde_json::to_vec(&request).unwrap());
		assert!(matches!(read_frame(&mut buffer), Err(FrameError::Closed)));
	}

	#[test]
	fn frames_reject_a_hostile_length_prefix() {
		let mut buffer = (MAX_FRAME_BYTES + 1).to_le_bytes().to_vec();
		buffer.extend_from_slice(b"{}");
		assert!(matches!(read_frame(&mut buffer.as_slice()), Err(FrameError::Oversized(len)) if len == MAX_FRAME_BYTES + 1));
	}

	#[test]
	fn frames_reject_a_truncated_body() {
		let body = serde_json::to_vec(&WorkResponse {
			status: 0,
			stdout: vec![1, 2, 3],
			stderr: vec![],
		})
		.unwrap();
		let mut buffer = (body.len() as u64).to_le_bytes().to_vec();
		buffer.extend_from_slice(&body[..body.len() - 1]);
		assert!(matches!(read_frame(&mut buffer.as_slice()), Err(FrameError::Io(_))));
	}

	#[cfg(unix)]
	#[test]
	fn a_crashed_worker_leaves_no_process_behind_and_is_replaced() {
		let dir = std::env::temp_dir().join(format!("forge-worker-pool-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let pids = dir.join("pids");
		let program = dir.join("crasher");
		std::fs::write(&program, format!("#!/bin/sh\necho $$ >> {}\nexit 9\n", pids.display())).unwrap();
		set_executable(&program);

		let runner = SandboxRunner::new(&dir, &dir.join("forge-out"));
		let toolchains = ToolchainPaths::default();
		let pool = WorkerPool::new(&runner, &toolchains);
		let sandbox = dir.join("sandbox");
		std::fs::create_dir_all(&sandbox).unwrap();
		let spec = probe_spec();
		let binding = WorkerBinding {
			program: program.display().to_string(),
			variant: "batch".into(),
		};

		let first = pool.execute(&spec, &binding, &sandbox);
		assert!(!first.success);
		assert!(first.stderr_tail.contains("worker"), "{}", first.stderr_tail);
		let second = pool.execute(&spec, &binding, &sandbox);
		assert!(!second.success);

		let spawned: Vec<String> = pids_content(&pids).lines().map(str::to_string).collect();
		assert_eq!(spawned.len(), 2, "a dead worker must be replaced, not reused");
		drop(pool);
		for pid in spawned {
			assert!(!process_alive(&pid), "worker {pid} survived the pool");
		}
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[cfg(unix)]
	#[test]
	fn requests_for_one_key_share_a_single_process_and_never_overlap() {
		let dir = std::env::temp_dir().join(format!("forge-worker-serial-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let pids = dir.join("pids");
		let program = dir.join("silent");
		std::fs::write(&program, format!("#!/bin/sh\necho $$ >> {}\nexec sleep 3\n", pids.display())).unwrap();
		set_executable(&program);

		let runner = SandboxRunner::new(&dir, &dir.join("forge-out"));
		let toolchains = ToolchainPaths::default();
		let pool = WorkerPool::new(&runner, &toolchains);
		let sandbox = dir.join("sandbox");
		std::fs::create_dir_all(&sandbox).unwrap();
		let spec = probe_spec();
		let binding = WorkerBinding {
			program: program.display().to_string(),
			variant: "batch".into(),
		};
		assert_eq!(pool.live_workers(), 0, "a pool starts nothing before the first request");

		let started = Instant::now();
		std::thread::scope(|scope| {
			scope.spawn(|| pool.execute(&spec, &binding, &sandbox));
			while spawn_count(&pids) == 0 {
				std::thread::sleep(std::time::Duration::from_millis(10));
			}
			scope.spawn(|| pool.execute(&spec, &binding, &sandbox));
			std::thread::sleep(std::time::Duration::from_millis(200));
			assert_eq!(
				spawn_count(&pids),
				1,
				"a queued request must not start a second process for its key"
			);
		});
		assert!(
			started.elapsed() >= std::time::Duration::from_millis(5500),
			"two three-second requests for one key must not overlap"
		);

		drop(pool);
		assert!(
			!process_alive(pids_content(&pids).lines().next().expect("one spawn")),
			"the pool must not leak its process"
		);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[cfg(unix)]
	fn probe_spec() -> ActionSpec {
		use forge_core::{ActionSpec, ConfigTransition, OutputDeclaration, OutputKind};
		ActionSpec {
			name: "worker probe".into(),
			component: "//:probe".into(),
			configuration: ConfigTransition::Target,
			command: "/bin/true".into(),
			args: Vec::new(),
			inputs: Vec::new(),
			execution_deps: Vec::new(),
			outputs: vec![OutputDeclaration {
				path: "probe.out".into(),
				kind: OutputKind::File,
			}],
			workdir: None,
			is_test: false,
			stdout: None,
			compile_command: None,
			environment_files: Vec::new(),
			argument_files: Vec::new(),
			env: BTreeMap::new(),
			toolchain_id: None,
			worker: None,
		}
	}

	#[cfg(unix)]
	fn set_executable(path: &Path) {
		use std::os::unix::fs::PermissionsExt;
		let mut permissions = std::fs::metadata(path).unwrap().permissions();
		permissions.set_mode(0o755);
		std::fs::set_permissions(path, permissions).unwrap();
	}

	#[cfg(unix)]
	fn pids_content(pids: &Path) -> String {
		std::fs::read_to_string(pids).unwrap_or_default()
	}

	#[cfg(unix)]
	fn spawn_count(pids: &Path) -> usize {
		pids_content(pids).lines().filter(|line| !line.is_empty()).count()
	}

	#[cfg(unix)]
	fn process_alive(pid: &str) -> bool {
		Path::new(&format!("/proc/{pid}")).exists()
	}
}
