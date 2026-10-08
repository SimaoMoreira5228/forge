use std::ffi::c_void;
use std::os::windows::io::AsRawHandle;
use std::process::{Child, Command, Output, Stdio};

use crate::execution::confinement::Policy;
use crate::execution::runner::Launch;

const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
const JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION: u32 = 0x0000_0400;

#[repr(C)]
#[derive(Default)]
struct IoCounters {
	read_operation_count: u64,
	write_operation_count: u64,
	other_operation_count: u64,
	read_transfer_count: u64,
	write_transfer_count: u64,
	other_transfer_count: u64,
}

#[repr(C)]
#[derive(Default)]
struct JobObjectBasicLimitInformation {
	per_process_user_time_limit: i64,
	per_job_user_time_limit: i64,
	limit_flags: u32,
	minimum_working_set_size: usize,
	maximum_working_set_size: usize,
	active_process_limit: u32,
	affinity: usize,
	priority_class: u32,
	scheduling_class: u32,
}

#[repr(C)]
#[derive(Default)]
struct JobObjectExtendedLimitInformation {
	basic_limit_information: JobObjectBasicLimitInformation,
	io_info: IoCounters,
	process_memory_limit: usize,
	job_memory_limit: usize,
	peak_process_memory_used: usize,
	peak_job_memory_used: usize,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<JobObjectExtendedLimitInformation>() == 144);

#[link(name = "kernel32")]
unsafe extern "system" {
	fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
	fn SetInformationJobObject(job: *mut c_void, info_class: i32, info: *const c_void, length: u32) -> i32;
	fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
	fn CloseHandle(object: *mut c_void) -> i32;
}

struct Job(*mut c_void);

unsafe impl Send for Job {}

impl Job {
	fn new() -> std::io::Result<Self> {
		let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
		if handle.is_null() {
			return Err(std::io::Error::last_os_error());
		}
		let mut limits = JobObjectExtendedLimitInformation::default();
		limits.basic_limit_information.limit_flags =
			JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
		let set = unsafe {
			SetInformationJobObject(
				handle,
				JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
				std::ptr::from_ref(&limits).cast(),
				std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
			)
		};
		if set == 0 {
			let error = std::io::Error::last_os_error();
			unsafe { CloseHandle(handle) };
			return Err(error);
		}
		Ok(Self(handle))
	}

	fn adopt(&self, child: &Child) -> std::io::Result<()> {
		let assigned = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle()) };
		if assigned == 0 {
			return Err(std::io::Error::last_os_error());
		}
		Ok(())
	}
}

impl Drop for Job {
	fn drop(&mut self) {
		unsafe { CloseHandle(self.0) };
	}
}

pub fn spawn(launch: &Launch, _policy: &Policy) -> std::io::Result<Output> {
	let job = Job::new()?;
	let child = command_for(launch).spawn()?;
	job.adopt(&child)?;
	child.wait_with_output()
}

pub fn probe() -> Result<(), String> {
	Job::new().map(|_| ()).map_err(|e| e.to_string())
}

fn command_for(launch: &Launch) -> Command {
	let mut command = Command::new(&launch.program);
	command.args(&launch.args);
	command.current_dir(&launch.workdir);
	command.stdout(Stdio::piped()).stderr(Stdio::piped());
	command.env_clear();
	command.envs(&launch.env);
	command
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_job_limits_are_the_documented_windows_constants() {
		let limits = JobObjectExtendedLimitInformation::default();
		assert_eq!(limits.basic_limit_information.limit_flags, 0);
		assert_eq!(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, 0x2000, "winbase.h value");
		assert_eq!(JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, 0x400, "winbase.h value");
		assert_eq!(JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS, 9, "JOBOBJECTINFOCLASS value");
	}

	#[cfg(target_pointer_width = "64")]
	#[test]
	fn the_limit_information_matches_the_windows_abi_field_for_field() {
		use std::mem::offset_of;

		assert_eq!(std::mem::size_of::<JobObjectBasicLimitInformation>(), 64);
		assert_eq!(std::mem::size_of::<IoCounters>(), 48);
		assert_eq!(std::mem::size_of::<JobObjectExtendedLimitInformation>(), 144);
		let basic = JobObjectBasicLimitInformation::default();
		let extended = JobObjectExtendedLimitInformation::default();
		assert_eq!(offset_of!(JobObjectBasicLimitInformation, limit_flags), 16);
		assert_eq!(offset_of!(JobObjectBasicLimitInformation, minimum_working_set_size), 24);
		assert_eq!(offset_of!(JobObjectBasicLimitInformation, maximum_working_set_size), 32);
		assert_eq!(offset_of!(JobObjectBasicLimitInformation, active_process_limit), 40);
		assert_eq!(offset_of!(JobObjectBasicLimitInformation, affinity), 48);
		assert_eq!(offset_of!(JobObjectBasicLimitInformation, priority_class), 56);
		assert_eq!(offset_of!(JobObjectBasicLimitInformation, scheduling_class), 60);
		assert_eq!(offset_of!(JobObjectExtendedLimitInformation, io_info), 64);
		assert_eq!(offset_of!(JobObjectExtendedLimitInformation, process_memory_limit), 112);
		assert_eq!(offset_of!(JobObjectExtendedLimitInformation, job_memory_limit), 120);
		assert_eq!(offset_of!(JobObjectExtendedLimitInformation, peak_process_memory_used), 128);
		assert_eq!(offset_of!(JobObjectExtendedLimitInformation, peak_job_memory_used), 136);
		assert_eq!(basic.affinity, 0);
		assert_eq!(extended.peak_job_memory_used, 0);
	}

	#[test]
	fn a_job_is_created_and_closes_onto_its_processes() {
		let job = Job::new().expect("create a job object");
		drop(job);
		assert!(probe().is_ok());
	}

	#[test]
	fn an_action_runs_in_its_job_and_reports_its_output() {
		let launch = Launch {
			program: "cmd.exe".into(),
			args: vec!["/c".into(), "echo confined".into()],
			env: Default::default(),
			workdir: std::env::temp_dir(),
		};
		let output = spawn(
			&launch,
			&Policy {
				writable: vec![],
				readable: vec![],
			},
		)
		.expect("spawn in a job");
		assert!(output.status.success());
		assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "confined");
	}

	#[test]
	fn no_process_outlives_the_action_that_started_it() {
		let dir = std::env::temp_dir().join(format!("forge-job-{}", std::process::id()));
		let _ = std::fs::create_dir_all(&dir);
		let marker = dir.join("outlived.txt");
		let _ = std::fs::remove_file(&marker);
		let launch = Launch {
			program: "cmd.exe".into(),
			args: vec![
				"/c".into(),
				format!(
					"start /b cmd /c \"ping -n 8 127.0.0.1 >nul & echo outlived > {}\"",
					marker.display()
				),
			],
			env: Default::default(),
			workdir: dir.clone(),
		};
		let output = spawn(
			&launch,
			&Policy {
				writable: vec![],
				readable: vec![],
			},
		)
		.expect("spawn in a job");
		assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
		std::thread::sleep(std::time::Duration::from_secs(9));
		assert!(
			!marker.exists(),
			"a process left the job and wrote {} after its action ended",
			marker.display()
		);
		let _ = std::fs::remove_dir_all(&dir);
	}
}
