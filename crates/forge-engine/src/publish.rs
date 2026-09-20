use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn temp_sibling(target: &Path) -> PathBuf {
	let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
	let name = target
		.file_name()
		.map(|name| name.to_string_lossy().into_owned())
		.unwrap_or_else(|| "output".to_string());
	target.with_file_name(format!(".{name}.forge-tmp-{}-{sequence}", std::process::id()))
}

pub fn publish_file(from: &Path, to: &Path) -> std::io::Result<()> {
	if let Some(parent) = to.parent() {
		std::fs::create_dir_all(parent)?;
	}
	let temporary = temp_sibling(to);
	std::fs::copy(from, &temporary)?;
	std::fs::rename(&temporary, to)
}

pub fn publish_tree(from: &Path, to: &Path) -> std::io::Result<()> {
	let temporary = temp_sibling(to);
	let _ = std::fs::remove_dir_all(&temporary);
	copy_tree(from, &temporary)?;
	let _ = std::fs::remove_dir_all(to);
	std::fs::rename(&temporary, to)
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
	std::fs::create_dir_all(to)?;
	for entry in std::fs::read_dir(from)?.flatten() {
		let source = entry.path();
		let destination = to.join(entry.file_name());
		if entry.file_type()?.is_dir() {
			copy_tree(&source, &destination)?;
		} else {
			std::fs::copy(&source, &destination)?;
		}
	}
	Ok(())
}
