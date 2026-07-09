use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

use forge_diagnostics::ForgeDiagnostic;

use crate::planner::ActionDag;

pub fn execute_dag<T: Send + Sync>(
	dag: &ActionDag,
	shared: &T,
	run: impl Fn(&T, usize) -> Result<(), ForgeDiagnostic> + Sync,
) -> Result<(), ForgeDiagnostic> {
	let pending = AtomicUsize::new(dag.specs.len());
	let indegree: Vec<AtomicUsize> = dag.deps.iter().map(|d| AtomicUsize::new(d.len())).collect();
	let dependents = dag.dependents();

	let ready: Mutex<VecDeque<usize>> = Mutex::new(
		indegree
			.iter()
			.enumerate()
			.filter(|(_, d)| d.load(Ordering::SeqCst) == 0)
			.map(|(i, _)| i)
			.collect(),
	);
	let progress = Condvar::new();
	let failure: Mutex<Option<ForgeDiagnostic>> = Mutex::new(None);

	rayon::scope(|scope| {
		let workers = rayon::current_num_threads().min(dag.specs.len().max(1));
		for _ in 0..workers {
			scope.spawn(|_| {
				loop {
					if failure.lock().is_some() {
						break;
					}

					let next = {
						let mut queue = ready.lock();
						loop {
							if let Some(index) = queue.pop_front() {
								break Some(index);
							}
							let failed = failure.lock().is_some();
							if failed || pending.load(Ordering::Acquire) == 0 {
								break None;
							}
							progress.wait(&mut queue);
						}
					};

					let Some(index) = next else { break };

					if let Err(e) = run(shared, index) {
						let mut slot = failure.lock();
						if slot.is_none() {
							*slot = Some(e);
						}
						drop(slot);
						progress.notify_all();
					}

					for &dependent in &dependents[index] {
						if indegree[dependent].fetch_sub(1, Ordering::AcqRel) == 1 {
							ready.lock().push_back(dependent);
							progress.notify_one();
						}
					}

					if pending.fetch_sub(1, Ordering::AcqRel) == 1 {
						progress.notify_all();
					}
				}
			});
		}
	});

	match failure.into_inner() {
		Some(e) => Err(e),
		None => Ok(()),
	}
}
