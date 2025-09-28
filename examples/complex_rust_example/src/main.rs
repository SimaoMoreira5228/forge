use crypto_utils::{CryptoUtils, FileMetadata, HashResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_double, c_int, c_void};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::ptr;
use std::time::Instant;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Vector3 {
	pub x: c_double,
	pub y: c_double,
	pub z: c_double,
}

#[repr(C)]
pub struct Matrix {
	pub data: *mut c_double,
	pub rows: usize,
	pub cols: usize,
}

#[link(name = "math_native", kind = "static")]
extern "C" {
	fn fast_fibonacci(n: c_int) -> i64;
	fn fast_factorial(n: c_int) -> u64;
	fn is_prime_fast(n: u64) -> bool;

	fn matrix_create(rows: usize, cols: usize) -> *mut Matrix;
	fn matrix_destroy(mat: *mut Matrix);
	fn matrix_multiply(a: *const Matrix, b: *const Matrix) -> *mut Matrix;
	fn matrix_determinant(mat: *const Matrix) -> c_double;

	fn vector3_add(a: Vector3, b: Vector3) -> Vector3;
	fn vector3_cross(a: Vector3, b: Vector3) -> Vector3;
	fn vector3_dot(a: Vector3, b: Vector3) -> c_double;
	fn vector3_magnitude(v: Vector3) -> c_double;
	fn vector3_normalize(v: Vector3) -> Vector3;
}

pub struct MathNative;

impl MathNative {
	pub fn fibonacci(n: i32) -> i64 {
		unsafe { fast_fibonacci(n) }
	}

	pub fn factorial(n: i32) -> u64 {
		unsafe { fast_factorial(n) }
	}

	pub fn is_prime(n: u64) -> bool {
		unsafe { is_prime_fast(n) }
	}

	pub fn vector_add(a: Vector3, b: Vector3) -> Vector3 {
		unsafe { vector3_add(a, b) }
	}

	pub fn vector_cross(a: Vector3, b: Vector3) -> Vector3 {
		unsafe { vector3_cross(a, b) }
	}

	pub fn vector_dot(a: Vector3, b: Vector3) -> f64 {
		unsafe { vector3_dot(a, b) }
	}

	pub fn vector_magnitude(v: Vector3) -> f64 {
		unsafe { vector3_magnitude(v) }
	}

	pub fn vector_normalize(v: Vector3) -> Vector3 {
		unsafe { vector3_normalize(v) }
	}
}

pub struct SafeMatrix {
	ptr: *mut Matrix,
}

impl SafeMatrix {
	pub fn new(rows: usize, cols: usize) -> Option<Self> {
		let ptr = unsafe { matrix_create(rows, cols) };
		if ptr.is_null() { None } else { Some(SafeMatrix { ptr }) }
	}

	pub fn get(&self, row: usize, col: usize) -> Option<f64> {
		unsafe {
			let mat = &*self.ptr;
			if row >= mat.rows || col >= mat.cols {
				return None;
			}
			Some(*mat.data.add(row * mat.cols + col))
		}
	}

	pub fn set(&mut self, row: usize, col: usize, value: f64) -> bool {
		unsafe {
			let mat = &*self.ptr;
			if row >= mat.rows || col >= mat.cols {
				return false;
			}
			*mat.data.add(row * mat.cols + col) = value;
			true
		}
	}

	pub fn dimensions(&self) -> (usize, usize) {
		unsafe {
			let mat = &*self.ptr;
			(mat.rows, mat.cols)
		}
	}

	pub fn multiply(&self, other: &SafeMatrix) -> Option<SafeMatrix> {
		let result_ptr = unsafe { matrix_multiply(self.ptr, other.ptr) };
		if result_ptr.is_null() {
			None
		} else {
			Some(SafeMatrix { ptr: result_ptr })
		}
	}

	pub fn determinant(&self) -> f64 {
		unsafe { matrix_determinant(self.ptr) }
	}
}

impl Drop for SafeMatrix {
	fn drop(&mut self) {
		unsafe {
			matrix_destroy(self.ptr);
		}
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ComputationResult {
	pub computation_type: String,
	pub input_data: serde_json::Value,
	pub result: serde_json::Value,
	pub execution_time_ms: u64,
	pub hash: HashResult,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkReport {
	pub timestamp: String,
	pub system_info: HashMap<String, String>,
	pub results: Vec<ComputationResult>,
	pub summary: BenchmarkSummary,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkSummary {
	pub total_computations: usize,
	pub total_time_ms: u64,
	pub fastest_computation: String,
	pub slowest_computation: String,
	pub average_time_ms: f64,
}

pub struct ComplexApplication {
	results: Vec<ComputationResult>,
}

impl ComplexApplication {
	pub fn new() -> Self {
		Self { results: Vec::new() }
	}

	pub fn run_math_benchmarks(&mut self) -> Result<(), Box<dyn std::error::Error>> {
		println!("Running complex mathematical computations...");

		self.benchmark_fibonacci()?;

		self.benchmark_prime_checking()?;

		self.benchmark_vector_operations()?;

		self.benchmark_matrix_operations()?;

		Ok(())
	}

	fn benchmark_fibonacci(&mut self) -> Result<(), Box<dyn std::error::Error>> {
		let inputs = vec![10, 20, 30, 35, 40];

		for &n in &inputs {
			let start = Instant::now();
			let result = MathNative::fibonacci(n);
			let duration = start.elapsed();

			let input_data = serde_json::json!({ "n": n });
			let result_data = serde_json::json!({ "fibonacci": result });
			let serialized = serde_json::to_string(&result_data)?;
			let hash = CryptoUtils::hash_string(&serialized);

			let computation_result = ComputationResult {
				computation_type: "fibonacci".to_string(),
				input_data,
				result: result_data,
				execution_time_ms: duration.as_millis() as u64,
				hash,
			};

			println!("Fibonacci({}) = {} ({}ms)", n, result, duration.as_millis());
			self.results.push(computation_result);
		}

		Ok(())
	}

	fn benchmark_prime_checking(&mut self) -> Result<(), Box<dyn std::error::Error>> {
		let test_numbers = vec![982451653, 982451654, 982451655, 982451656, 982451657];

		for &n in &test_numbers {
			let start = Instant::now();
			let is_prime = MathNative::is_prime(n);
			let duration = start.elapsed();

			let input_data = serde_json::json!({ "number": n });
			let result_data = serde_json::json!({ "is_prime": is_prime });
			let serialized = serde_json::to_string(&result_data)?;
			let hash = CryptoUtils::hash_string(&serialized);

			let computation_result = ComputationResult {
				computation_type: "prime_check".to_string(),
				input_data,
				result: result_data,
				execution_time_ms: duration.as_millis() as u64,
				hash,
			};

			println!("is_prime({}) = {} ({}ms)", n, is_prime, duration.as_millis());
			self.results.push(computation_result);
		}

		Ok(())
	}

	fn benchmark_vector_operations(&mut self) -> Result<(), Box<dyn std::error::Error>> {
		let vectors = vec![
			(Vector3 { x: 1.0, y: 2.0, z: 3.0 }, Vector3 { x: 4.0, y: 5.0, z: 6.0 }),
			(Vector3 { x: 2.5, y: -1.0, z: 0.5 }, Vector3 { x: -1.0, y: 3.0, z: 2.0 }),
			(Vector3 { x: 0.0, y: 1.0, z: 0.0 }, Vector3 { x: 1.0, y: 0.0, z: 0.0 }),
		];

		for (i, (a, b)) in vectors.iter().enumerate() {
			let start = Instant::now();

			let sum = MathNative::vector_add(*a, *b);
			let cross = MathNative::vector_cross(*a, *b);
			let dot = MathNative::vector_dot(*a, *b);
			let mag_a = MathNative::vector_magnitude(*a);
			let mag_b = MathNative::vector_magnitude(*b);
			let norm_a = MathNative::vector_normalize(*a);

			let duration = start.elapsed();

			let input_data = serde_json::json!({
				"vector_a": { "x": a.x, "y": a.y, "z": a.z },
				"vector_b": { "x": b.x, "y": b.y, "z": b.z }
			});

			let result_data = serde_json::json!({
				"add": { "x": sum.x, "y": sum.y, "z": sum.z },
				"cross": { "x": cross.x, "y": cross.y, "z": cross.z },
				"dot_product": dot,
				"magnitude_a": mag_a,
				"magnitude_b": mag_b,
				"normalized_a": { "x": norm_a.x, "y": norm_a.y, "z": norm_a.z }
			});

			let serialized = serde_json::to_string(&result_data)?;
			let hash = CryptoUtils::hash_string(&serialized);

			let computation_result = ComputationResult {
				computation_type: "vector_operations".to_string(),
				input_data,
				result: result_data,
				execution_time_ms: duration.as_millis() as u64,
				hash,
			};

			println!("Vector operations {} completed ({}ms)", i + 1, duration.as_millis());
			self.results.push(computation_result);
		}

		Ok(())
	}

	fn benchmark_matrix_operations(&mut self) -> Result<(), Box<dyn std::error::Error>> {
		let mut mat_a = SafeMatrix::new(2, 2).unwrap();
		mat_a.set(0, 0, 1.0);
		mat_a.set(0, 1, 2.0);
		mat_a.set(1, 0, 3.0);
		mat_a.set(1, 1, 4.0);

		let mut mat_b = SafeMatrix::new(2, 2).unwrap();
		mat_b.set(0, 0, 5.0);
		mat_b.set(0, 1, 6.0);
		mat_b.set(1, 0, 7.0);
		mat_b.set(1, 1, 8.0);

		let start = Instant::now();

		let det_a = mat_a.determinant();
		let det_b = mat_b.determinant();
		let product = mat_a.multiply(&mat_b).unwrap();
		let det_product = product.determinant();

		let duration = start.elapsed();

		let input_data = serde_json::json!({
			"matrix_a": [[1.0, 2.0], [3.0, 4.0]],
			"matrix_b": [[5.0, 6.0], [7.0, 8.0]]
		});

		let result_data = serde_json::json!({
			"determinant_a": det_a,
			"determinant_b": det_b,
			"determinant_product": det_product,
			"product_matrix": [
				[product.get(0, 0), product.get(0, 1)],
				[product.get(1, 0), product.get(1, 1)]
			]
		});

		let serialized = serde_json::to_string(&result_data)?;
		let hash = CryptoUtils::hash_string(&serialized);

		let computation_result = ComputationResult {
			computation_type: "matrix_operations".to_string(),
			input_data,
			result: result_data,
			execution_time_ms: duration.as_millis() as u64,
			hash,
		};

		println!("Matrix operations completed ({}ms)", duration.as_millis());
		self.results.push(computation_result);

		Ok(())
	}

	pub fn run_crypto_operations(&mut self) -> Result<(), Box<dyn std::error::Error>> {
		println!("Running cryptographic operations...");

		self.benchmark_hashing()?;

		self.benchmark_file_operations()?;

		self.benchmark_key_derivation()?;

		Ok(())
	}

	fn benchmark_hashing(&mut self) -> Result<(), Box<dyn std::error::Error>> {
		let test_strings = vec![
			"Hello, World!",
			"The quick brown fox jumps over the lazy dog",
			&"x".repeat(1000),
			&"complex data with unicode: αβγδε 🚀🌟💫",
		];

		for test_str in test_strings {
			let start = Instant::now();
			let hash_result = CryptoUtils::hash_string(test_str);
			let duration = start.elapsed();

			let input_data = serde_json::json!({
				"input_string": test_str,
				"input_length": test_str.len()
			});

			let result_data = serde_json::json!({
				"hash": hash_result.hash,
				"algorithm": hash_result.algorithm,
				"size": hash_result.size
			});

			let serialized = serde_json::to_string(&result_data)?;
			let meta_hash = CryptoUtils::hash_string(&serialized);

			let computation_result = ComputationResult {
				computation_type: "string_hashing".to_string(),
				input_data,
				result: result_data,
				execution_time_ms: duration.as_millis() as u64,
				hash: meta_hash,
			};

			println!(
				"Hashed string (len={}): {} ({}ms)",
				test_str.len(),
				&hash_result.hash[..16],
				duration.as_millis()
			);
			self.results.push(computation_result);
		}

		Ok(())
	}

	fn benchmark_file_operations(&mut self) -> Result<(), Box<dyn std::error::Error>> {
		let test_file_path = "test_data.txt";
		let test_content = "This is a test file for cryptographic operations.\n".repeat(100);

		fs::write(test_file_path, &test_content)?;

		let start = Instant::now();
		let file_hash = CryptoUtils::hash_file(test_file_path)?;
		let file_metadata = CryptoUtils::analyze_file(test_file_path, 1024)?;
		let duration = start.elapsed();

		let input_data = serde_json::json!({
			"file_path": test_file_path,
			"chunk_size": 1024
		});

		let result_data = serde_json::json!({
			"file_hash": file_hash,
			"metadata": {
				"path": file_metadata.path,
				"size": file_metadata.size,
				"chunk_count": file_metadata.chunk_hashes.len(),
				"chunk_merkle": CryptoUtils::verify_chunks(&file_metadata.chunk_hashes)
			}
		});

		let serialized = serde_json::to_string(&result_data)?;
		let meta_hash = CryptoUtils::hash_string(&serialized);

		let computation_result = ComputationResult {
			computation_type: "file_hashing".to_string(),
			input_data,
			result: result_data,
			execution_time_ms: duration.as_millis() as u64,
			hash: meta_hash,
		};

		println!(
			"Analyzed file: {} chunks, hash {} ({}ms)",
			file_metadata.chunk_hashes.len(),
			&file_hash.hash[..16],
			duration.as_millis()
		);

		self.results.push(computation_result);

		fs::remove_file(test_file_path).ok();

		Ok(())
	}

	fn benchmark_key_derivation(&mut self) -> Result<(), Box<dyn std::error::Error>> {
		let passwords = vec!["password123", "super_secure_password", "🔐🗝️🛡️"];
		let salts = vec![b"salt", b"random_salt_value", b"unicode_salt_\xf0\x9f\xa7\x82"];

		for password in &passwords {
			for salt in &salts {
				let start = Instant::now();
				let key = CryptoUtils::derive_key(password, salt, 32);
				let duration = start.elapsed();

				let input_data = serde_json::json!({
					"password": password,
					"salt": base64::encode(salt),
					"output_length": 32
				});

				let result_data = serde_json::json!({
					"key": base64::encode(&key),
					"key_length": key.len()
				});

				let serialized = serde_json::to_string(&result_data)?;
				let hash = CryptoUtils::hash_string(&serialized);

				let computation_result = ComputationResult {
					computation_type: "key_derivation".to_string(),
					input_data,
					result: result_data,
					execution_time_ms: duration.as_millis() as u64,
					hash,
				};

				println!("Derived key from '{}' ({}ms)", password, duration.as_millis());
				self.results.push(computation_result);
			}
		}

		Ok(())
	}

	pub fn generate_report(&self) -> Result<BenchmarkReport, Box<dyn std::error::Error>> {
		let total_time: u64 = self.results.iter().map(|r| r.execution_time_ms).sum();
		let total_count = self.results.len();

		let fastest = self
			.results
			.iter()
			.min_by_key(|r| r.execution_time_ms)
			.map(|r| r.computation_type.clone())
			.unwrap_or_else(|| "none".to_string());

		let slowest = self
			.results
			.iter()
			.max_by_key(|r| r.execution_time_ms)
			.map(|r| r.computation_type.clone())
			.unwrap_or_else(|| "none".to_string());

		let average_time = if total_count > 0 {
			total_time as f64 / total_count as f64
		} else {
			0.0
		};

		let mut system_info = HashMap::new();
		system_info.insert("architecture".to_string(), std::env::consts::ARCH.to_string());
		system_info.insert("os".to_string(), std::env::consts::OS.to_string());
		system_info.insert("target_triple".to_string(), env!("TARGET").to_string());

		let summary = BenchmarkSummary {
			total_computations: total_count,
			total_time_ms: total_time,
			fastest_computation: fastest,
			slowest_computation: slowest,
			average_time_ms: average_time,
		};

		Ok(BenchmarkReport {
			timestamp: std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_secs()
				.to_string(),
			system_info,
			results: self.results.clone(),
			summary,
		})
	}

	pub fn save_report(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
		let report = self.generate_report()?;
		let serialized = serde_json::to_string_pretty(&report)?;

		let file = File::create(path)?;
		let mut writer = BufWriter::new(file);
		writer.write_all(serialized.as_bytes())?;
		writer.flush()?;

		println!("Report saved to: {}", path);
		Ok(())
	}
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	println!("🚀 Complex Rust Application with C Integration and Cryptography");
	println!("================================================================");

	let mut app = ComplexApplication::new();

	app.run_math_benchmarks()?;

	println!();

	app.run_crypto_operations()?;

	println!();

	let report = app.generate_report()?;
	println!("📊 Benchmark Summary:");
	println!("  Total computations: {}", report.summary.total_computations);
	println!("  Total time: {}ms", report.summary.total_time_ms);
	println!("  Average time: {:.2}ms", report.summary.average_time_ms);
	println!("  Fastest: {}", report.summary.fastest_computation);
	println!("  Slowest: {}", report.summary.slowest_computation);

	app.save_report("benchmark_report.json")?;

	println!("\n🔗 Content-Addressed Storage Examples:");
	for result in &app.results[..std::cmp::min(3, app.results.len())] {
		let cas_path = CryptoUtils::cas_path(&result.hash.hash);
		println!("  {} -> {}", result.computation_type, cas_path);
	}

	println!("\n✅ All operations completed successfully!");
	Ok(())
}

mod base64 {
	const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

	pub fn encode(input: &[u8]) -> String {
		let mut result = String::new();
		let chunks = input.chunks_exact(3);
		let remainder = chunks.remainder();

		for chunk in chunks {
			let b1 = chunk[0] as usize;
			let b2 = chunk[1] as usize;
			let b3 = chunk[2] as usize;

			let n = (b1 << 16) | (b2 << 8) | b3;

			result.push(CHARS[(n >> 18) & 63] as char);
			result.push(CHARS[(n >> 12) & 63] as char);
			result.push(CHARS[(n >> 6) & 63] as char);
			result.push(CHARS[n & 63] as char);
		}

		match remainder.len() {
			1 => {
				let n = (remainder[0] as usize) << 16;
				result.push(CHARS[(n >> 18) & 63] as char);
				result.push(CHARS[(n >> 12) & 63] as char);
				result.push('=');
				result.push('=');
			}
			2 => {
				let n = ((remainder[0] as usize) << 16) | ((remainder[1] as usize) << 8);
				result.push(CHARS[(n >> 18) & 63] as char);
				result.push(CHARS[(n >> 12) & 63] as char);
				result.push(CHARS[(n >> 6) & 63] as char);
				result.push('=');
			}
			_ => {}
		}

		result
	}
}
